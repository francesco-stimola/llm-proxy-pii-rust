//! Overlap resolution shared by every detector.
//!
//! Detectors can produce spans that overlap — a regex email and an ML `Person`
//! covering the same characters, two recognizers matching the same digits. A
//! single stretch of text must carry exactly one label, so this reduces a bag of
//! candidate [`PiiEntity`]s to a non-overlapping set.

use std::ops::Range;

use super::{PiiEntity, PiiKind};

/// Reduce overlapping spans to a non-overlapping set, kept in reading order.
///
/// Candidates are ranked by [`PiiKind::priority`](super::PiiKind::priority)
/// (desc), then by span length (desc), then greedily accepted only if they don't
/// overlap something already kept. Priority-then-length is what makes a
/// deterministic structured match (email, IBAN) win over an ML `Person`/`Org`
/// guess on the same characters — and an IBAN win the digits a phone would claim.
///
/// **Email containment gate (M4-R9)** — applied *before* priority. An email is the
/// only structured kind carrying `@`, so a card / IBAN / national-ID / secret can
/// overlap one in two ways, and they need opposite outcomes:
///
/// - **Contained** in the email's local part (`4111111111111111@x.com`) → the email
///   is the complete, correct match; the contained span is a false decomposition and
///   is dropped here so the email wins. Otherwise it would fragment the email and
///   forward the `@domain` in clear.
/// - **Partially overlapping** it — a *grouped* form butting against `@domain`
///   (`4111 1111 1111 1111@x.com`, where the space-free local part makes the email
///   regex grab only `1111@x.com`) → **not** dropped, so it falls through to
///   priority, where every structured kind outranks `Email` and therefore wins. This
///   is the fail-safe direction: the leading groups (`4111 1111 1111`) get masked
///   instead of being left in clear.
///
/// **Remainder policy (M2-R7, deliberate):** when a lower-priority span overlaps
/// a kept one, the *whole* lower span is dropped — including any part that
/// extends beyond the overlap. A `Person` span that happens to enclose an email
/// is discarded entirely (only the email survives). This is a lean, conscious
/// choice: masking the whole enclosing span would over-mask, and trimming spans
/// adds complexity for a rare case. Structured PII is never lost either way; the
/// only cost is recall on the non-overlapping *unstructured* remainder.
pub fn resolve_overlaps(mut candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
    drop_spans_contained_in_an_email(&mut candidates);

    candidates.sort_by(|a, b| {
        b.kind
            .priority()
            .cmp(&a.kind.priority())
            .then_with(|| b.span.len().cmp(&a.span.len()))
    });

    let mut kept: Vec<PiiEntity> = Vec::new();
    for candidate in candidates {
        let clashes = kept
            .iter()
            .any(|k| candidate.span.start < k.span.end && k.span.start < candidate.span.end);
        if !clashes {
            kept.push(candidate);
        }
    }

    kept.sort_by_key(|e| e.span.start);
    kept
}

/// The Email containment gate (M4-R9): drop every **structured** span that lies
/// *entirely inside* an `Email` span — it is a substring of the email's local part
/// (`123456789@x.com`, `4111111111111111@x.com`), not a separate entity, so the
/// email must win it. Only **full containment** counts: a merely *overlapping*
/// span (a grouped card/IBAN/NINO abutting `@domain`) survives and beats `Email` on
/// priority, so its digits are masked rather than left in clear.
///
/// NER spans are left to priority (`Email` already outranks them). An email is never
/// itself contained in a structured span — no other structured kind matches `@` — so
/// dropping the contained span can't strand a surviving email.
fn drop_spans_contained_in_an_email(candidates: &mut Vec<PiiEntity>) {
    let emails: Vec<Range<usize>> = candidates
        .iter()
        .filter(|e| e.kind == PiiKind::Email)
        .map(|e| e.span.clone())
        .collect();
    if emails.is_empty() {
        return;
    }
    candidates.retain(|c| {
        if c.kind == PiiKind::Email || !c.kind.is_structured() {
            return true;
        }
        !emails
            .iter()
            .any(|e| e.start <= c.span.start && c.span.end <= e.end)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::{Confidence, PiiKind};

    fn entity(kind: PiiKind, span: std::ops::Range<usize>) -> PiiEntity {
        PiiEntity {
            kind,
            text: "x".to_string(),
            span,
            confidence: Confidence::Verified,
        }
    }

    #[test]
    fn structured_beats_ner_on_overlap() {
        // An ML Person guess overlapping a deterministic Email → Email wins.
        let kept = resolve_overlaps(vec![
            entity(PiiKind::Person, 0..16),
            entity(PiiKind::Email, 0..16),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::Email);
    }

    #[test]
    fn non_overlapping_entities_all_survive_in_reading_order() {
        let kept = resolve_overlaps(vec![
            entity(PiiKind::Location, 20..28),
            entity(PiiKind::Person, 0..5),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].span.start, 0);
        assert_eq!(kept[1].span.start, 20);
    }

    #[test]
    fn email_containing_a_structured_span_wins_it() {
        // M4-R9 gate, containment side: a card that is a substring of the email's
        // local part (`4111111111111111@x.com`) is a false decomposition — the whole
        // email wins, so the `@domain` is never forwarded in clear.
        let kept = resolve_overlaps(vec![
            entity(PiiKind::CreditCard, 0..16),
            entity(PiiKind::Email, 0..28),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::Email);
    }

    #[test]
    fn email_partially_overlapping_a_structured_span_loses_it() {
        // M4-R9 gate, partial-overlap side: a *grouped* card
        // (`4111 1111 1111 1111@x.com`) is only partially overlapped by the email
        // (`1111@x.com`) — no containment, so the card must win. Otherwise Email
        // would swallow the last group and leave `4111 1111 1111` in clear: a leak.
        let kept = resolve_overlaps(vec![
            entity(PiiKind::CreditCard, 0..19),
            entity(PiiKind::Email, 15..31),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::CreditCard);
        assert_eq!(kept[0].span, 0..19);
    }

    #[test]
    fn ner_span_enclosing_a_structured_span_is_dropped_whole() {
        // M2-R7: a Person span that strictly encloses an Email keeps only the
        // Email; the Person (incl. its non-overlapping remainder) is discarded.
        let kept = resolve_overlaps(vec![
            entity(PiiKind::Person, 0..30),
            entity(PiiKind::Email, 10..24),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::Email);
    }
}

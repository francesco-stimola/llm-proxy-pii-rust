//! Overlap resolution shared by every detector.
//!
//! Detectors can produce spans that overlap — a regex email and an ML `Person`
//! covering the same characters, two recognizers matching the same digits. A
//! single stretch of text must carry exactly one label, so this reduces a bag of
//! candidate [`PiiEntity`]s to a non-overlapping set.
//!
//! # The invariant (M4-R10 / M4-R11)
//!
//! **No structured span's bytes are ever abandoned.** This is *the* rule; everything
//! below follows from it.
//!
//! The original resolver settled every overlap by **dropping the whole loser span**,
//! which silently left the loser's bytes *in clear*. A flat `priority()` scalar can
//! only express "one of them wins" — it cannot express "both must be masked". So on a
//! **partial** overlap (`555 867 5309john.doe@example.com`, where the phone and the
//! email share only `5309`) whichever side lost was forwarded unmasked, and tuning the
//! priorities merely chose *which* PII leaked.
//!
//! Therefore two structured spans that overlap are **merged into their union**, labelled
//! with the higher-priority kind, instead of one being dropped. The union is masked as a
//! single placeholder and restored verbatim, so the round-trip stays exact and nothing is
//! left behind. It over-masks a little (a bare `@domain` can end up inside the
//! placeholder) — the project's stated, fail-safe direction: over-mask, never leak.
//!
//! **NER spans keep the whole-span drop** (M2-R7): abandoning a `Person` remainder costs
//! recall, never a leak.

use std::ops::Range;

use super::{Confidence, PiiEntity, PiiKind};

/// Reduce overlapping spans to a non-overlapping set, kept in reading order.
///
/// Three phases:
///
/// 1. **Email containment gate** — a structured span lying *entirely inside* an `Email`
///    span is a false decomposition of the email's local part (`4111111111111111@x.com`,
///    `123456789@x.com`), so it is dropped and the email keeps the label. Safe under the
///    merge invariant: an email is never *dropped* (only merged into a union that covers
///    at least its own span), so the bytes of the span removed here are always masked by
///    whatever the email becomes.
/// 2. **Structured union-merge** — overlapping structured spans collapse into their union
///    (label = highest priority, then longest). A single sort-by-start sweep reaches the
///    fixpoint, since a merge can only ever extend the running union to the right.
/// 3. **NER greedy drop** — NER candidates are taken by priority (desc) then length
///    (desc), and kept only if they don't overlap something already kept. A `Person` that
///    encloses an email is discarded whole (M2-R7): masking the enclosing span would
///    over-mask, and the only cost is recall on the *unstructured* remainder.
pub fn resolve_overlaps(input: &str, candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
    let (mut structured, mut ner): (Vec<PiiEntity>, Vec<PiiEntity>) =
        candidates.into_iter().partition(|e| e.kind.is_structured());

    drop_spans_contained_in_an_email(&mut structured);
    let mut kept = merge_structured(input, structured);

    ner.sort_by(|a, b| {
        b.kind
            .priority()
            .cmp(&a.kind.priority())
            .then_with(|| b.span.len().cmp(&a.span.len()))
            .then_with(|| a.span.start.cmp(&b.span.start))
    });
    for candidate in ner {
        if !kept.iter().any(|k| overlaps(&candidate.span, &k.span)) {
            kept.push(candidate);
        }
    }

    kept.sort_by_key(|e| e.span.start);
    kept
}

/// Half-open ranges intersect (touching end-to-start is *not* an overlap).
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// The Email containment gate (M4-R9): drop every **structured** span that lies
/// *entirely inside* an `Email` span — it is a substring of the email's local part
/// (`123456789@x.com`, `sk-…@x.com`), not a separate entity, so the email must keep the
/// label. Only **full containment** counts: a merely *overlapping* span (a grouped
/// card/IBAN/NINO abutting `@domain`) survives into the union-merge below, which masks
/// both. Dropping here can never strand the removed span, because the containing email is
/// itself never dropped (see [`merge_structured`]).
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
        if c.kind == PiiKind::Email {
            return true;
        }
        !emails
            .iter()
            .any(|e| e.start <= c.span.start && c.span.end <= e.end)
    });
}

/// Merge overlapping **structured** spans into their union — the heart of the
/// no-abandoned-bytes invariant (M4-R10 / M4-R11).
///
/// Sorting by start and sweeping once reaches the fixpoint: a candidate can only extend
/// the running union to the right, so a merge never creates an earlier overlap to revisit.
/// The union carries the **highest-priority** constituent's kind (ties → the longest
/// span, then the earliest), and its text is re-sliced from `input` so the `Vault` masks
/// and restores exactly the bytes the union covers.
///
/// A union that is *wider* than its winning candidate is no longer a checksum-validated
/// value, so it is honestly tagged [`Confidence::Structural`]; an untouched span keeps
/// the confidence its recognizer gave it.
fn merge_structured(input: &str, mut candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
    candidates.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then_with(|| b.span.end.cmp(&a.span.end))
    });

    // (union, winning candidate)
    let mut groups: Vec<(Range<usize>, PiiEntity)> = Vec::new();
    for candidate in candidates {
        match groups.last_mut() {
            Some((union, winner)) if candidate.span.start < union.end => {
                union.end = union.end.max(candidate.span.end);
                if outranks(&candidate, winner) {
                    *winner = candidate;
                }
            }
            _ => groups.push((candidate.span.clone(), candidate)),
        }
    }

    groups
        .into_iter()
        .map(|(union, winner)| match input.get(union.clone()) {
            Some(text) => PiiEntity {
                kind: winner.kind,
                confidence: if union == winner.span {
                    winner.confidence
                } else {
                    Confidence::Structural
                },
                text: text.to_string(),
                span: union,
            },
            // Unreachable in practice (spans come from regexes over this same input);
            // degrade to the winning span rather than panic or invent text.
            None => winner,
        })
        .collect()
}

/// Which of two overlapping structured candidates names the merged union: the
/// higher-priority kind, then the longer span. Deterministic — the sweep keeps the
/// incumbent on a full tie.
fn outranks(candidate: &PiiEntity, incumbent: &PiiEntity) -> bool {
    (candidate.kind.priority(), candidate.span.len())
        > (incumbent.kind.priority(), incumbent.span.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::{Confidence, PiiKind};

    /// Long enough to slice any span the tests below use.
    const INPUT: &str = "0123456789012345678901234567890123456789";

    fn entity(kind: PiiKind, span: std::ops::Range<usize>) -> PiiEntity {
        PiiEntity {
            kind,
            text: INPUT[span.clone()].to_string(),
            span,
            confidence: Confidence::Verified,
        }
    }

    fn resolve(candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
        resolve_overlaps(INPUT, candidates)
    }

    #[test]
    fn structured_beats_ner_on_overlap() {
        // An ML Person guess overlapping a deterministic Email → Email wins.
        let kept = resolve(vec![
            entity(PiiKind::Person, 0..16),
            entity(PiiKind::Email, 0..16),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::Email);
    }

    #[test]
    fn non_overlapping_entities_all_survive_in_reading_order() {
        let kept = resolve(vec![
            entity(PiiKind::Location, 20..28),
            entity(PiiKind::Person, 0..5),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].span.start, 0);
        assert_eq!(kept[1].span.start, 20);
    }

    #[test]
    fn email_containing_a_structured_span_wins_it() {
        // Containment gate: a card that is a substring of the email's local part
        // (`4111111111111111@x.com`) is a false decomposition — the email wins the
        // label, and its span already covers the card's bytes.
        let kept = resolve(vec![
            entity(PiiKind::CreditCard, 0..16),
            entity(PiiKind::Email, 0..28),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::Email);
        assert_eq!(kept[0].span, 0..28);
    }

    #[test]
    fn partially_overlapping_structured_spans_merge_into_their_union() {
        // M4-R10/R11: a *grouped* card and the email that starts inside its last group
        // only partially overlap. Dropping either side would abandon its bytes in clear,
        // so they merge into the union — labelled by the higher-priority kind (Card).
        let kept = resolve(vec![
            entity(PiiKind::CreditCard, 0..19),
            entity(PiiKind::Email, 15..31),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::CreditCard);
        assert_eq!(kept[0].span, 0..31, "the union must cover BOTH spans");
        // The union is wider than the checksum-validated card → honestly not Verified.
        assert_eq!(kept[0].confidence, Confidence::Structural);
    }

    #[test]
    fn a_chain_of_overlaps_merges_to_a_fixpoint() {
        // A merge can create a new overlap; the sweep must reach the fixpoint in one
        // pass (phone → email → card, each overlapping only its neighbour).
        let kept = resolve(vec![
            entity(PiiKind::Phone, 0..12),
            entity(PiiKind::Email, 8..24),
            entity(PiiKind::CreditCard, 20..34),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].span, 0..34);
        // Highest priority among the three constituents.
        assert_eq!(kept[0].kind, PiiKind::CreditCard);
    }

    #[test]
    fn a_span_stranded_by_the_containment_gate_is_still_covered() {
        // M4-R10: the gate deletes the card (contained in the email), and a phone then
        // partially overlaps that email. Under the old drop-the-loser the email lost and
        // the already-deleted card was forwarded IN CLEAR. With the union-merge the email
        // is never dropped — it is absorbed into a union that still covers the card.
        let kept = resolve(vec![
            entity(PiiKind::Phone, 0..12),
            entity(PiiKind::CreditCard, 13..29),
            entity(PiiKind::Email, 8..34),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].span, 0..34, "the card's bytes (13..29) must be covered");
    }

    #[test]
    fn ner_span_enclosing_a_structured_span_is_dropped_whole() {
        // M2-R7: a Person span that strictly encloses an Email keeps only the
        // Email; the Person (incl. its non-overlapping remainder) is discarded.
        let kept = resolve(vec![
            entity(PiiKind::Person, 0..30),
            entity(PiiKind::Email, 10..24),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, PiiKind::Email);
    }
}

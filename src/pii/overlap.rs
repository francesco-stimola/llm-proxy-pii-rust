//! Overlap resolution shared by every detector.
//!
//! Detectors can produce spans that overlap — a regex email and an ML `Person`
//! covering the same characters, two recognizers matching the same digits. A
//! single stretch of text must carry exactly one label, so this reduces a bag of
//! candidate [`PiiEntity`]s to a non-overlapping set.

use super::PiiEntity;

/// Reduce overlapping spans to a non-overlapping set, kept in reading order.
///
/// Candidates are ranked by [`PiiKind::priority`](super::PiiKind::priority)
/// (desc), then by span length (desc), then greedily accepted only if they don't
/// overlap something already kept. Priority-then-length is what makes a
/// deterministic structured match (email, IBAN) win over an ML `Person`/`Org`
/// guess on the same characters — and an IBAN win the digits a phone would claim.
///
/// **Remainder policy (M2-R7, deliberate):** when a lower-priority span overlaps
/// a kept one, the *whole* lower span is dropped — including any part that
/// extends beyond the overlap. A `Person` span that happens to enclose an email
/// is discarded entirely (only the email survives). This is a lean, conscious
/// choice: masking the whole enclosing span would over-mask, and trimming spans
/// adds complexity for a rare case. Structured PII is never lost either way; the
/// only cost is recall on the non-overlapping *unstructured* remainder.
pub fn resolve_overlaps(mut candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
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

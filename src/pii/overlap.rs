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
//! Nothing structured is ever *deleted*, so the invariant holds by construction. (An
//! earlier revision deleted a span enclosed by an email before ranking the rest — which
//! stranded that span in clear whenever the enclosing email then lost, M4-R10. Enclosure
//! is now a **naming** rule, not a deletion: see [`name_of`].)
//!
//! **NER spans keep the whole-span drop** (M2-R7): abandoning a `Person` remainder costs
//! recall, never a leak.

use std::ops::Range;

use super::{Confidence, PiiEntity, PiiKind};

/// Reduce overlapping spans to a non-overlapping set, kept in reading order.
///
/// Two phases:
///
/// 1. **Structured union-merge** — every group of transitively-overlapping structured
///    spans collapses into its union. Nothing is dropped, so no structured byte is ever
///    abandoned. A single sort-by-start sweep reaches the fixpoint, since a merge can only
///    ever extend the running union to the right. See [`merge_structured`] for how the
///    union is *named*.
/// 2. **NER greedy drop** — NER candidates are taken by priority (desc) then length
///    (desc), and kept only if they don't overlap something already kept. A `Person` that
///    encloses an email is discarded whole (M2-R7): masking the enclosing span would
///    over-mask, and the only cost is recall on the *unstructured* remainder.
pub fn resolve_overlaps(input: &str, candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
    let (structured, mut ner): (Vec<PiiEntity>, Vec<PiiEntity>) =
        candidates.into_iter().partition(|e| e.kind.is_structured());

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

/// Merge each group of transitively-overlapping **structured** spans into its union — the
/// heart of the no-abandoned-bytes invariant (M4-R10 / M4-R11).
///
/// Sorting by start and sweeping once reaches the fixpoint: a candidate can only extend
/// the running union to the right, so a merge never creates an earlier overlap to revisit.
/// The union's text is re-sliced from `input`, so the `Vault` (which keys on `text` and
/// splices by `span`) masks and restores exactly the bytes the union covers.
///
/// A union *wider* than the candidate that names it is no longer a checksum-validated
/// value, so it is honestly tagged [`Confidence::Structural`]; an untouched span keeps the
/// confidence its recognizer gave it.
fn merge_structured(input: &str, mut candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
    candidates.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then_with(|| b.span.end.cmp(&a.span.end))
    });

    // Group by transitive overlap. Every candidate lands in exactly one group, and the
    // group's union covers all of it — that *is* the invariant.
    let mut groups: Vec<Vec<PiiEntity>> = Vec::new();
    let mut union_end = 0usize;
    for candidate in candidates {
        match groups.last_mut() {
            Some(group) if candidate.span.start < union_end => {
                union_end = union_end.max(candidate.span.end);
                group.push(candidate);
            }
            _ => {
                union_end = candidate.span.end;
                groups.push(vec![candidate]);
            }
        }
    }

    groups
        .into_iter()
        .flat_map(|group| materialize(input, group))
        .collect()
}

/// Turn one overlap group into the entity (or entities) that survive it.
fn materialize(input: &str, group: Vec<PiiEntity>) -> Vec<PiiEntity> {
    let union = match (
        group.iter().map(|e| e.span.start).min(),
        group.iter().map(|e| e.span.end).max(),
    ) {
        (Some(start), Some(end)) => start..end,
        _ => return group, // empty group is impossible, but never invent a span
    };

    let winner = name_of(&group, &union);
    let kind = winner.kind;
    let confidence = if winner.span == union {
        winner.confidence
    } else {
        Confidence::Structural
    };

    // M4-R14 — the slice must never *shrink*: widening to char boundaries can only add
    // bytes, so it cannot abandon a constituent.
    let span = widen_to_char_boundaries(input, union);
    match input.get(span.clone()) {
        Some(text) => vec![PiiEntity {
            kind,
            span,
            text: text.to_string(),
            confidence,
        }],
        // Unreachable: `widen_to_char_boundaries` returns a clamped, boundary-aligned
        // range, so the slice always succeeds. If it somehow didn't, returning the whole
        // group unmerged is the only fail-safe answer — a single span here would drop the
        // other constituents' bytes, which is exactly the leak class this code exists to
        // prevent. Never a panic: this is a proxy, and the input is attacker-influenced.
        None => {
            debug_assert!(false, "merged span is not sliceable from the input");
            tracing::warn!(?kind, "un-sliceable merged span — keeping constituents unmerged");
            group
        }
    }
}

/// Which candidate **names** the union (M4-R15).
///
/// Normally the **highest-priority** candidate in the group (ties → the longest span, then
/// the earliest). Crucially this considers *every raw candidate*, including ones an email
/// encloses — otherwise a `Secret` glued to a phone (`555 867 5309.sk-…@x.com`) would be
/// announced to the model as `[PHONE_1]` and the kind-only audit log would under-report a
/// secret as a phone.
///
/// **One exception — the email containment rule** (M4-R7 / M4-R9): when the union is
/// *exactly* an `Email` span, the group is a genuine email whose local part merely *looks*
/// like a card or an ID (`4111111111111111@x.com`, `123456789@x.com`). That enclosed match
/// is a false decomposition, not a second entity, so the email keeps the label. (This is
/// what the old containment *gate* achieved by deleting the enclosed span — but deleting
/// it was a booby trap, since the enclosing email was not guaranteed to survive the sort,
/// stranding the deleted span in clear (M4-R10). Expressing it as a naming rule instead of
/// a deletion keeps the behaviour and removes the trap: the union is unchanged either way,
/// because an enclosed span merges *into* its enclosing email.)
fn name_of<'a>(group: &'a [PiiEntity], union: &Range<usize>) -> &'a PiiEntity {
    if let Some(email) = group
        .iter()
        .find(|e| e.kind == PiiKind::Email && e.span == *union)
    {
        return email;
    }
    group
        .iter()
        .reduce(|best, e| if outranks(e, best) { e } else { best })
        .unwrap_or(&group[0])
}

/// Strictly higher priority, then strictly longer span. Deterministic: a full tie keeps
/// the incumbent, and the sweep feeds candidates in span order.
fn outranks(candidate: &PiiEntity, incumbent: &PiiEntity) -> bool {
    (candidate.kind.priority(), candidate.span.len())
        > (incumbent.kind.priority(), incumbent.span.len())
}

/// Expand a byte range outward to the nearest `char` boundaries of `input`, clamped to its
/// length (M4-R14). Widening only ever *adds* bytes, so a merged span can never end up
/// covering less than its constituents did. The result is always sliceable.
fn widen_to_char_boundaries(input: &str, span: Range<usize>) -> Range<usize> {
    let mut start = span.start.min(input.len());
    let mut end = span.end.min(input.len());
    while start > 0 && !input.is_char_boundary(start) {
        start -= 1;
    }
    while end < input.len() && !input.is_char_boundary(end) {
        end += 1;
    }
    start..end.max(start)
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
    fn email_enclosing_a_structured_span_names_the_union() {
        // Naming rule (`name_of`): the union is *exactly* the email's span, so the enclosed
        // card is a false decomposition of its local part (`4111111111111111@x.com`) — the
        // email keeps the label. Nothing is deleted; the union already covers the card.
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
    fn a_span_enclosed_by_an_email_is_still_covered_and_names_the_union() {
        // M4-R10: a card enclosed by an email, and a phone partially overlapping that
        // email. The card's bytes must stay covered (they used to be stranded in clear
        // when the enclosing email lost the sort and the card had already been deleted).
        // M4-R15: the union is *named* by the highest-priority RAW candidate it covers —
        // the card — not by whichever candidate happened to survive.
        let kept = resolve(vec![
            entity(PiiKind::Phone, 0..12),
            entity(PiiKind::CreditCard, 13..29),
            entity(PiiKind::Email, 8..34),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].span, 0..34, "the card's bytes (13..29) must be covered");
        assert_eq!(
            kept[0].kind,
            PiiKind::CreditCard,
            "the union must be named by the highest-priority candidate it covers"
        );
    }

    #[test]
    fn an_enclosed_secret_names_the_union_not_the_phone() {
        // M4-R15: `555 867 5309.sk-…@x.com` — the secret sits inside the email's local
        // part, so it used to be deleted before priority was consulted and the union came
        // out as `[PHONE_1]`. No leak (the secret was masked), but the model was told the
        // blob is a phone and the kind-only audit log under-reported a Secret as a Phone.
        let kept = resolve(vec![
            entity(PiiKind::Phone, 0..12),
            entity(PiiKind::Secret, 13..28),
            entity(PiiKind::Email, 8..34),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].span, 0..34);
        assert_eq!(kept[0].kind, PiiKind::Secret, "Secret outranks Phone and Email");
    }

    #[test]
    fn a_union_ending_inside_a_multibyte_char_still_covers_every_constituent() {
        // M4-R14: the re-slice must never *shrink*. Two overlapping structured spans whose
        // union endpoint falls inside a multi-byte character must widen out to the char
        // boundary — never degrade to a single constituent, which is the leak class this
        // resolver exists to prevent.
        let input = "abc€def"; // '€' is 3 bytes at 3..6
        let raw = |kind, span: std::ops::Range<usize>| PiiEntity {
            kind,
            text: String::new(), // deliberately not sliced — the resolver must re-slice
            span,
            confidence: Confidence::Verified,
        };
        // Union = 0..5, and byte 5 is *inside* '€'.
        let kept = resolve_overlaps(
            input,
            vec![
                raw(PiiKind::CreditCard, 0..4),
                raw(PiiKind::Phone, 2..5),
            ],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].span, 0..6, "widened out to the char boundary");
        assert_eq!(kept[0].text, "abc€", "text re-sliced from the input");
        // Both constituents' bytes (0..4 and 2..5) are inside the kept span.
        assert!(kept[0].span.start <= 0 && 5 <= kept[0].span.end);
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

//! **PHONE-OM (M10) — the over-mask guard, on text nobody curated.**
//!
//! The negative corpus contains digit-shaped non-phones *we thought of*. M10 widened the
//! candidate set twice over — a non-trunk shape family means any plausible digit group is a
//! candidate, and nine plans give each candidate nine chances to be somebody's valid number
//! — so "we thought of it" stopped being good enough.
//!
//! Real agent traffic is full of digit runs nobody writes test cases for: line numbers in
//! diffs, ports, byte offsets, timestamps, PIDs, error codes, file sizes. So this asserts
//! over the **M7 latency fixture** — a real ~22 KiB Claude Code turn already in the repo,
//! written for a different purpose and therefore not curated for this one. With every region
//! on, the `Phone` spans it yields must be **exactly** the ones listed below: a change in
//! that set is a regression to explain, not a diff to accept.
//!
//! And the harm here is **functional, not privacy**. A masked line number or port inside
//! `tool_use.input` hands the model `[PHONE_1]` where it needed `8080`, and the agent then
//! does the wrong thing. No corpus test shows you that, because a corpus contains only the
//! false positives we imagined.

#[path = "common/m7_turn.rs"]
mod m7_turn;

use llm_proxy_pii_rust::pii::recognizers::{vetted_phone_regions, StructuredRecognizers};
use llm_proxy_pii_rust::pii::{PiiDetector, PiiKind};

/// Every `Phone` span the fixture is expected to produce, as `(field, text)`.
///
/// **Empty is the honest expectation, not a placeholder.** The fixture is instruction
/// boilerplate, tool schemas and one Italian user message whose PII is an email and an
/// IBAN — there is no phone number in it. Any entry appearing here later is an over-mask
/// until someone shows otherwise.
const EXPECTED_PHONE_SPANS: &[(&str, &str)] = &[];

#[test]
fn the_realistic_turn_yields_exactly_the_expected_phone_spans() {
    let detector = StructuredRecognizers::new();
    assert_eq!(
        vetted_phone_regions().len(),
        9,
        "this guard is written for the full shipped region set"
    );

    let mut found: Vec<(String, String)> = Vec::new();
    let mut bytes = 0usize;
    for field in m7_turn::realistic_turn() {
        bytes += field.text.len();
        for entity in detector.detect(&field.text) {
            if entity.kind == PiiKind::Phone {
                found.push((field.name.clone(), entity.text));
            }
        }
    }

    // Non-vacuity: a guard over a fixture that shrank to nothing proves nothing (M4-R13).
    assert!(
        bytes > 20_000,
        "the M7 fixture is only {bytes} bytes — this guard is supposed to run over a real \
         ~22 KiB turn, and a shrunken fixture would pass it for the wrong reason"
    );

    let expected: Vec<(String, String)> = EXPECTED_PHONE_SPANS
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
    assert_eq!(
        found, expected,
        "the `Phone` spans found in a real 22 KiB Claude Code turn changed. Every entry is \
         a digit run the model will receive as `[PHONE_N]` instead of its real value — for \
         a port or a line number inside `tool_use.input` that is a functional break, not a \
         privacy one. Explain the change before updating EXPECTED_PHONE_SPANS."
    );
}

/// The same turn, but with a domestic number **from every shape family** spliced into the
/// user message — so the guard above cannot pass merely because the detector stopped working.
///
/// An assert-absence test needs a positive control (M9-R28, BENCH-01): "no phone spans" and
/// "no detection at all" produce the same empty vector, and only this distinguishes them.
///
/// **One control per family, not one control (M10-R7).** The first version spliced in only
/// `06 69821234` — trunk-anchored, i.e. the family M10 did *not* add. Delete both un-anchored
/// families and every control still passed: the fixture still yielded zero `Phone` spans
/// (fewer recognizers cannot find more), the region count was still 9, the fixture was still
/// 22 KiB, and the trunk number was still found. The guard was green with the two families
/// whose over-mask risk it exists to bound switched off — and ROADMAP step 10 nominates this
/// test as the automated stand-in for the CC battery that could not be run, so it has to be
/// able to *see* them.
#[test]
fn the_guard_would_notice_a_phone_that_really_is_there() {
    // One per `PHONE_SHAPES` entry, in order: trunk 2/3-group, the French five-pair form,
    // un-anchored groups, un-anchored prefix + long block.
    const ONE_PER_SHAPE_FAMILY: &[&str] = &[
        "06 69821234",
        "01 23 45 67 89",
        "91 123 45 67",
        "347 1234567",
    ];

    let detector = StructuredRecognizers::new();
    for number in ONE_PER_SHAPE_FAMILY {
        let mut fields = m7_turn::realistic_turn();
        let user = fields
            .last_mut()
            .expect("the turn ends with the user message");
        user.text.push_str(&format!(" Il numero e' {number}."));

        let found: Vec<String> = fields
            .iter()
            .flat_map(|f| detector.detect(&f.text))
            .filter(|e| e.kind == PiiKind::Phone)
            .map(|e| e.text)
            .collect();
        assert_eq!(
            found,
            vec![number.to_string()],
            "every shape family must be live in this build — otherwise the empty expectation \
             above is satisfied by a detector that cannot see that family at all"
        );
    }
}

/// The fixture's **shape**, asserted in the default build (M10-R12).
///
/// `m7_latency.rs` carries the full shape assertions, but they live in an `#[ignore]`d test
/// inside a file gated `#![cfg(feature = "onnx")]` — so they run only with a model present,
/// i.e. never in CI. Two comments pointed the next reader at that guard as the reason the
/// shared fixture "cannot silently drift". These are the cheap structural parts, where both
/// consumers of the shared module get them on every `cargo test`.
#[test]
fn the_shared_fixture_keeps_its_shape() {
    let fields = m7_turn::realistic_turn();
    let bytes: usize = fields.iter().map(|f| f.text.len()).sum();

    assert!(
        (20_000..50_000).contains(&bytes),
        "the realistic turn is {bytes} bytes — it is supposed to be a ~22 KiB Claude Code turn"
    );
    assert_eq!(
        fields
            .iter()
            .filter(|f| f.part == m7_turn::Part::System)
            .count(),
        1,
        "Claude Code sends its whole system prompt as ONE field — the per-field cost model \
         this fixture exists to exercise depends on that"
    );
    assert!(
        fields
            .iter()
            .filter(|f| f.part == m7_turn::Part::SchemaDescription)
            .count()
            > 50,
        "the many-small-fields tier is what makes this turn's shape realistic"
    );
    assert_eq!(
        fields
            .iter()
            .filter(|f| f.part == m7_turn::Part::UserMessage)
            .count(),
        1,
        "one user message, and it is where all the PII is"
    );
}

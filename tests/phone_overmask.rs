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
//!
//! **Two fixtures, because one had a shape (M11-R55).** The M7 turn holds no IPv4 address and
//! no digit run separated by 2-4 spaces, so it could not see either class the round-14
//! separator widening admitted and stayed green through both. `PHONE-OM-TOOL` runs the same
//! construction over `tests/common/tool_output_turn.rs` — `ls -l`, `df -h`, a `psql` result,
//! journal lines with IP addresses — and its expectation is **the measured set, not an empty
//! one**: M11-R55 accepted this over-mask and published it, so a guard asserting zero here
//! would assert the opposite of what ships. What it guards is that the set cannot move
//! without somebody moving it on purpose.

#[path = "common/m7_turn.rs"]
mod m7_turn;
#[path = "common/tool_output_turn.rs"]
mod tool_output_turn;

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

/// **PHONE-OM-TOOL (M11-R55) — the same construction over the shape the M7 turn does not have.**
///
/// Every `Phone` span the tool-output fixture is expected to produce, as `(field, text)`.
///
/// **A measured expectation, not an empty one, and that is the finding's decision showing up in
/// the suite.** M11-R55 measured what round 14's separator widening admitted — dotted-quad IPv4
/// addresses and column-aligned numeric rows — and the maintainer accepted it: an over-mask is
/// restored byte-identically on the response path, a miss is not. So the honest expectation here
/// is the set that ships, and the guard is that it cannot **move** silently. Narrow the alphabet
/// or the separator run and entries disappear; widen either and entries appear. Both are red.
const EXPECTED_TOOL_OUTPUT_PHONE_SPANS: &[(&str, &str)] = &[
    // A `psql` result: six rows, six spans. Five of the six are *truncated* rather than whole —
    // `318   120   3499` stops before the fourth column — which is the same coalescing the
    // resolver does everywhere; the response path restores the bytes either way.
    ("tool_result[psql]", "101   250   1999   140"),
    ("tool_result[psql]", "205   310   2499   140"),
    ("tool_result[psql]", "318   120   3499"),
    ("tool_result[psql]", "90   1299   318"),
    ("tool_result[psql]", "512   105   205"),
    ("tool_result[psql]", "913   107   207"),
    // Dotted quads. `10.55.120.7` and `172.16.31.9` are NOT here and that is not an oversight:
    // no enabled plan assigns their digit runs, so the rate is a rate and not a certainty.
    ("tool_result[journal]", "62.30.40.50"),
    ("tool_result[journal]", "170.75.154.131"),
    ("tool_result[journal]", "192.168.14.203"),
    // The one that costs an agent a turn: a host in an `ssh` argument, and a literal the
    // command greps for.
    ("tool_use[ssh].input", "62.30.40.50"),
    ("tool_use[ssh].input", "512 105 205"),
];

#[test]
fn the_tool_output_turn_yields_exactly_the_expected_phone_spans() {
    let detector = StructuredRecognizers::new();
    assert_eq!(
        vetted_phone_regions().len(),
        9,
        "this guard is written for the full shipped region set"
    );

    let mut found: Vec<(String, String)> = Vec::new();
    let mut bytes = 0usize;
    for field in tool_output_turn::tool_output_turn() {
        bytes += field.text.len();
        for entity in detector.detect(&field.text) {
            if entity.kind == PiiKind::Phone {
                found.push((field.name.clone(), entity.text));
            }
        }
    }

    // Non-vacuity, two ways. The byte floor is the M4-R13 rule; the *shape* floor is M11-R55's,
    // because this fixture exists for two specific shapes and a rewrite that dropped them would
    // leave a green guard measuring nothing at all.
    assert!(
        bytes > 1_500,
        "the tool-output fixture is only {bytes} bytes — a shrunken fixture would pass this \
         guard for the wrong reason"
    );
    let all: String = tool_output_turn::tool_output_turn()
        .iter()
        .map(|f| f.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let occurrences = |pattern: &str| regex::Regex::new(pattern).unwrap().find_iter(&all).count();
    assert!(
        occurrences(r"(?-u:\b)\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(?-u:\b)") >= 5,
        "the fixture must carry dotted-quad IPv4 addresses — that is half of what it exists for"
    );
    assert!(
        occurrences(r"[0-9]{2,3}[ ]{2,4}[0-9]{2,4}") >= 5,
        "the fixture must carry digit runs separated by 2-4 spaces — the other half. The M7 turn \
         has zero of these, which is exactly why a second fixture exists (M11-R55)"
    );

    let expected: Vec<(String, String)> = EXPECTED_TOOL_OUTPUT_PHONE_SPANS
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
    assert_eq!(
        found, expected,
        "the `Phone` spans found in a turn of ordinary command output changed. Each one is a \
         value the model receives as `[PHONE_N]` — an IP address in an `ssh` argument, a row of \
         a `psql` result — and the client still gets its bytes back byte-for-byte. This set is \
         published in ARCHITECTURE.md and CHANGELOG.md as an accepted cost, so a change here \
         moves a published promise, in either direction."
    );
}

/// **PHONE-BUD (M10-R28 / M10-R30) — the headroom the budget's number rests on, measured over a
/// real turn instead of asserted in a doc comment.**
///
/// `MAX_PHONE_VALIDATIONS_PER_REQUEST` is defended by the claim that ordinary traffic cannot come
/// near it. M10 made that claim and could not show it: the allowance was per *call*, `mask_all`
/// re-minted it up to five times per field, and `PrivacyStage` called it once per field — so "the
/// budget" was not a number any single quantity had, and nobody noticed for three rounds (M10-R30).
///
/// Now there is one budget per request, so headroom is a thing you can put a number on. This charges
/// **every field of the 22 KiB Claude Code turn against one budget**, exactly as the request path
/// does, and pins the total. The assertion is loose on purpose — an order of magnitude, not a
/// fixture-fragile constant — because what must never regress is the *margin*, not the digit.
#[test]
fn a_real_claude_code_turn_spends_almost_none_of_the_request_budget() {
    use llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST;
    use llm_proxy_pii_rust::pii::Budget;

    let detector = StructuredRecognizers::new();
    let budget = Budget::new(MAX_PHONE_VALIDATIONS_PER_REQUEST);

    let mut bytes = 0usize;
    for field in m7_turn::realistic_turn() {
        bytes += field.text.len();
        detector
            .try_detect(&field.text, &budget)
            .expect("a real 22 KiB turn must never exhaust the request budget");
    }

    // Non-vacuity, the same bar as the guard above: a fixture that shrank proves nothing.
    assert!(
        bytes > 20_000,
        "the M7 fixture is only {bytes} bytes — this measurement is meaningless on a stub"
    );

    let spent = budget.spent();
    println!(
        "PHONE-BUD: a real {bytes} byte Claude Code turn spends {spent} of \
         {MAX_PHONE_VALIDATIONS_PER_REQUEST} validation units"
    );
    assert!(
        spent * 100 < MAX_PHONE_VALIDATIONS_PER_REQUEST,
        "a real {bytes} byte Claude Code turn spent {spent} of {MAX_PHONE_VALIDATIONS_PER_REQUEST} \
         validation units — over 1% of the whole request allowance on one ordinary turn. The number \
         is defended by having wide headroom against real traffic; if this fails, either the \
         headroom is gone or the budget is being charged for work it was not sized for (M10-R29)."
    );
}

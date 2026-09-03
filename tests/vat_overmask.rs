//! **VAT-OM (M11 Track A) — the tax tier's over-mask cost on text nobody curated.**
//!
//! `VAT-09` measures the bare Partita IVA's false-positive rate the only way a synthetic
//! sweep can: about **one arbitrary 11-digit number in ten** satisfies its mod-10. That is
//! half a measurement. It says what the recognizer does to a *uniform stream of 11-digit
//! numbers*; it says nothing about how many 11-digit numbers real agent traffic contains, or
//! what they are.
//!
//! [M10](../docs/ROADMAP.md) set the precedent this file follows. The domestic phone tier
//! shipped with a synthetic rate **and** a proof of zero hits over a real turn
//! (`tests/phone_overmask.rs`), because the negative corpus only ever contains the false
//! positives somebody imagined. The VAT tier shipped with the synthetic half alone, and the
//! maintainer's decision of 2026-09-03 (ROADMAP → M11 Track A, *"A third decision"*) closed
//! that with a bar declared **before** the number existed: **zero** bare-form `TaxId` spans
//! over the same real ~22 KiB Claude Code turn.
//!
//! **Why the bare form and not the tier.** Four of the six shipped recognizers demand a
//! literal country prefix (`IT`/`DE`/`GB`/`PT`) and the fifth a `NL…B…` skeleton, so their
//! false-positive surface is a string ordinary text does not produce. The bare P.IVA is
//! `\b\d{11}\b` + mod-10 with **no context required** — it alone carries the whole 0.100.
//!
//! **And the harm is functional, not privacy** — the same asymmetry PHONE-OM was written for.
//! An over-masked value is restored on the response path, so nothing leaks either way. What
//! breaks is the model's input: an eleven-digit database id or order number inside a
//! `tool_use.input` arrives as `[TAXID_1]`, and the agent then queries the wrong row. ASCII
//! word boundaries already exclude the two common timestamp widths (10 = Unix seconds,
//! 13 = milliseconds); what is left exposed is exactly ids and order numbers of length 11.

#[path = "common/m7_turn.rs"]
mod m7_turn;

use llm_proxy_pii_rust::pii::recognizers::{shipped_tax_recognizer_count, StructuredRecognizers};
use llm_proxy_pii_rust::pii::{PiiDetector, PiiEntity, PiiKind};

/// Every `TaxId` span the fixture is expected to produce, as `(field, text)`.
///
/// **Empty is the declared bar, not a placeholder.** The turn is instruction boilerplate,
/// tool schemas and one Italian user message whose PII is an email and an IBAN — there is no
/// VAT number in it, and no 11-digit token that mod-10 accepts. An entry appearing here later
/// is an over-mask on real traffic until somebody shows otherwise, and ROADMAP says what
/// happens then: it is fixed, not documented around.
const EXPECTED_TAXID_SPANS: &[(&str, &str)] = &[];

/// Is `c` an ASCII word character — the alphabet `(?-u:\b)` draws its boundaries against?
///
/// The recognizers all disable Unicode for their boundaries (VAT-07: with Unicode `\b` a Han
/// character is a word character, and the whole tier would be inert in CJK prose). This
/// mirrors that decision rather than importing the regex crate, so the count below is an
/// **independent** reading of the fixture and not the same engine agreeing with itself.
fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The length of every maximal ASCII-word token in `text` that is **all digits**.
///
/// This is the **denominator** of the measurement, and the reason it is a length distribution
/// rather than a count. A token of exactly 11 digits is where `(?-u:\b)\d{11}(?-u:\b)` matches,
/// so the number of them is the surface the bare P.IVA's mod-10 is actually offered on this
/// traffic. Without it, "zero spans" cannot be told apart from "zero candidates" — the first
/// reads as a precision result, the second is only a statement about the fixture — and knowing
/// how *far* the longest run is from 11 is what says which of the two was observed.
fn digit_token_lengths(text: &str) -> Vec<usize> {
    text.split(|c: char| !is_ascii_word(c))
        .filter(|tok| !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_digit()))
        .map(str::len)
        .collect()
}

/// **VAT-15 — the tax tier over a real 22 KiB Claude Code turn must mask nothing.**
///
/// The bar ROADMAP declared before this number existed. Two assertions, because one of them
/// can be satisfied for the wrong reason:
///
/// 1. **No `TaxId` span**, which is the bar verbatim.
/// 2. **No masked span of any kind that is exactly 11 ASCII digits.** `TaxId` sits *below*
///    `NationalId` and `Phone` in the priority order (VAT-14), so an 11-digit run that the
///    bare recognizer claims may be **named** by another tier and vanish from assertion 1
///    while still being masked. Filtering on the label would therefore measure the naming
///    rule, not the over-mask. Filtering on the shape cannot be fooled that way.
#[test]
fn the_realistic_turn_yields_exactly_the_expected_taxid_spans() {
    let detector = StructuredRecognizers::new();

    let mut found: Vec<(String, String)> = Vec::new();
    let mut eleven_digit_shaped: Vec<(String, PiiKind, String)> = Vec::new();
    let mut digit_runs: Vec<usize> = Vec::new();
    let mut bytes = 0usize;
    for field in m7_turn::realistic_turn() {
        bytes += field.text.len();
        digit_runs.extend(digit_token_lengths(&field.text));
        for entity in detector.detect(&field.text) {
            if entity.kind == PiiKind::TaxId {
                found.push((field.name.clone(), entity.text.clone()));
            }
            if entity.text.len() == 11 && entity.text.bytes().all(|b| b.is_ascii_digit()) {
                eleven_digit_shaped.push((field.name.clone(), entity.kind, entity.text));
            }
        }
    }

    // Non-vacuity: a guard over a fixture that shrank to nothing proves nothing (M4-R13).
    assert!(
        bytes > 20_000,
        "the M7 fixture is only {bytes} bytes — this guard is supposed to run over a real \
         ~22 KiB turn, and a shrunken fixture would pass it for the wrong reason"
    );

    // The denominator, printed rather than asserted. It is not a threshold — it is the fact a
    // reader needs in order to know what the zero above means, and it belongs in the run output
    // where the number was produced. TESTING.md → VAT-15 records the values and the residue
    // they leave, which is the part a passing test cannot state on its own.
    let candidates = digit_runs.iter().filter(|n| **n == 11).count();
    println!(
        "VAT-15: {candidates} eleven-digit token(s) among {} all-digit tokens in {bytes} bytes \
         of real turn (longest run {} digits) — that is the surface the bare P.IVA mod-10 was offered",
        digit_runs.len(),
        digit_runs.iter().copied().max().unwrap_or(0)
    );

    let expected: Vec<(String, String)> = EXPECTED_TAXID_SPANS
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
    assert_eq!(
        found, expected,
        "the `TaxId` spans found in a real 22 KiB Claude Code turn changed. Every entry is a \
         token the model will receive as `[TAXID_N]` instead of its real value — for a database \
         id or an order number inside `tool_use.input` that is a functional break, not a privacy \
         one. ROADMAP → M11 Track A declared zero as the bar BEFORE this number existed: explain \
         the change and fix it, do not update EXPECTED_TAXID_SPANS."
    );
    assert!(
        eleven_digit_shaped.is_empty(),
        "a span of exactly eleven digits was masked over the real turn: {eleven_digit_shaped:?}. \
         The label may be `NationalId` or `Phone` — `TaxId` ranks below both — but the shape is \
         the bare P.IVA's, so the assertion above cannot see it. This is the same over-mask, \
         wearing another tier's name."
    );
}

/// **VAT-16 — the guard above would notice a VAT number that really is there.**
///
/// An assert-absence test needs a positive control (M9-R28, BENCH-01): "no VAT spans" and "no
/// detection at all" produce the same empty vector, and only this distinguishes them.
///
/// **One control per shipped scheme, and the count is read from the recognizer set** — not
/// from a list in this file. M10-R7 is the reason: PHONE-OM's first control covered one shape
/// family of three, so deleting the other two left every control green while the guard's whole
/// subject was switched off. A remembered list has that defect built in; asking
/// [`shipped_tax_recognizer_count`] makes a seventh recognizer landing without a seventh
/// control a **failure** rather than a silent narrowing.
#[test]
fn the_guard_would_notice_a_vat_number_that_really_is_there() {
    // One per `vat_recognizers()` entry, in order: IT bare, IT VIES, DE, GB, PT, NL. Real
    // published numbers, the same corpus VAT-01/VAT-03 pin — not contrivances that happen to
    // satisfy a checksum.
    const ONE_PER_SHIPPED_SCHEME: &[&str] = &[
        "00905811006",    // 🇮🇹 ENI, bare — the form Italians actually write
        "IT00905811006",  // 🇮🇹 ENI, VIES form
        "DE136695976",    // 🇩🇪 the German administration's own documented vector
        "GB220430231",    // 🇬🇧 Tesco
        "PT524287244",    // 🇵🇹
        "NL111222333B01", // 🇳🇱 an 11-proef body, so this control is `Verified`
    ];

    assert_eq!(
        ONE_PER_SHIPPED_SCHEME.len(),
        shipped_tax_recognizer_count(),
        "this guard holds one live control per shipped VAT recognizer, and the tier now ships \
         {} of them. A scheme with no control here is a scheme VAT-15's empty expectation would \
         be satisfied by never seeing at all (M10-R7).",
        shipped_tax_recognizer_count()
    );

    let detector = StructuredRecognizers::new();
    for number in ONE_PER_SHIPPED_SCHEME {
        let mut fields = m7_turn::realistic_turn();
        let user = fields
            .last_mut()
            .expect("the turn ends with the user message");
        user.text.push_str(&format!(" La partita IVA e' {number}."));

        let found: Vec<String> = fields
            .iter()
            .flat_map(|f| detector.detect(&f.text))
            .filter(|e| e.kind == PiiKind::TaxId)
            .map(|e: PiiEntity| e.text)
            .collect();
        assert_eq!(
            found,
            vec![number.to_string()],
            "every shipped scheme must be live in this build — otherwise VAT-15's empty \
             expectation is satisfied by a detector that cannot see that scheme at all"
        );
    }
}

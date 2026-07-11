//! Adversarial / evasion recall tests (M1.5).
//!
//! For a privacy tool a *miss is a leak*, so we measure recall against evasion
//! attempts and pin down exactly where structured detection stops. Some cases we
//! now catch (broadened recognizers); others are **documented gaps** that only
//! ML/NER (M2) or dedicated heuristics can close — asserting them here keeps the
//! boundary honest and flags the day behaviour changes.

use llm_proxy_pii_rust::pii::PiiDetector;
use llm_proxy_pii_rust::pii::PiiKind;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;

fn detect(input: &str) -> Vec<(PiiKind, String)> {
    StructuredRecognizers::new()
        .detect(input)
        .into_iter()
        .map(|e| (e.kind, e.text))
        .collect()
}

fn detects(input: &str, kind: PiiKind, text: &str) -> bool {
    detect(input).contains(&(kind, text.to_string()))
}

#[test]
fn caught_email_variants() {
    assert!(detects("ping john+tag@example.com now", PiiKind::Email, "john+tag@example.com"));
    assert!(detects("MAIL JOHN@EXAMPLE.COM", PiiKind::Email, "JOHN@EXAMPLE.COM"));
    assert!(detects(
        "user_name.surname@sub.domain.co.uk here",
        PiiKind::Email,
        "user_name.surname@sub.domain.co.uk"
    ));
}

#[test]
fn caught_phone_and_iban_edge_shapes() {
    assert!(detects("tel (555) 867-5309.", PiiKind::Phone, "(555) 867-5309"));
    assert!(detects("+1 555.867.5309 ok", PiiKind::Phone, "+1 555.867.5309"));
    // IBAN immediately before a currency word must not swallow it.
    assert!(detects(
        "send IT60X0542811101000000123456 EUR",
        PiiKind::Iban,
        "IT60X0542811101000000123456"
    ));
}

#[test]
fn documented_gaps_obfuscated_email_not_yet_detected() {
    // ACCEPTED LIMITATION (structured detection): human-obfuscated emails are not
    // valid addresses, so masking them would explode false positives. Tracked for
    // NER/heuristic follow-up. If any of these start matching, revisit the
    // fail-closed story and update this test.
    assert!(detect("reach me at john [at] example [dot] com").is_empty());
    assert!(detect("john (at) example (dot) com").is_empty());
    assert!(detect("j o h n @ e x a m p l e . c o m").is_empty());
}

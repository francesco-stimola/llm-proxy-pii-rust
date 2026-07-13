//! Adversarial / evasion recall tests (M1.5).
//!
//! For a privacy tool a *miss is a leak*, so we measure recall against evasion
//! attempts and pin down exactly where structured detection stops. Some cases we
//! now catch (broadened recognizers); others are **documented gaps** that only
//! ML/NER (M2) or dedicated heuristics can close — asserting them here keeps the
//! boundary honest and flags the day behaviour changes.

use llm_proxy_pii_rust::pii::PiiDetector;
use llm_proxy_pii_rust::pii::PiiKind;
use llm_proxy_pii_rust::pii::anonymizer::Vault;
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

/// Mask `input` with the structured recognizers — what the upstream would see.
fn masked(input: &str) -> String {
    let detector = StructuredRecognizers::new();
    let mut vault = Vault::new();
    vault.mask(input, &detector.detect(input))
}

#[test]
fn grouped_pii_glued_to_a_domain_leaks_nothing() {
    // M4-R9 — evasion shape: gluing `@domain.tld` onto a *space-grouped* card / IBAN
    // / NINO. The email local-part class excludes the space, so the email match forms
    // from only the trailing group and *partially* overlaps the structured span. If
    // Email won that overlap (it briefly did, under M4-R7), the leading groups would
    // be left IN CLEAR. Assert directly on the masked body: no digit group survives.
    let out = masked("card 4111 1111 1111 1111@example.com");
    assert!(!out.contains("4111"), "card digits left in clear: {out}");
    assert!(!out.contains("1111"), "card digits left in clear: {out}");

    let out = masked("iban DE89 3704 0044 0532 0130 00@example.com");
    assert!(!out.contains("DE89"), "IBAN body left in clear: {out}");
    assert!(!out.contains("3704"), "IBAN body left in clear: {out}");
    assert!(!out.contains("0532"), "IBAN body left in clear: {out}");

    let out = masked("nino AB 12 34 56 C@example.com");
    assert!(!out.contains("AB 12"), "NINO left in clear: {out}");
    assert!(!out.contains("34 56"), "NINO left in clear: {out}");

    // The containment case still resolves the other way (whole email masked), so a
    // continuous card glued to a domain leaks neither the digits nor the domain.
    let out = masked("card 4111111111111111@example.com");
    assert!(!out.contains("4111111111111111"), "card left in clear: {out}");
    assert!(!out.contains("example.com"), "domain left in clear: {out}");
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
fn phone_international_span_stops_at_the_number() {
    // M1.5 review follow-up: the international arm must not swallow an unrelated
    // trailing number group (same class as the fixed IBAN over-match).
    assert_eq!(
        detect("chiama +39 333 0000001 12345 subito"),
        vec![(PiiKind::Phone, "+39 333 0000001".to_string())]
    );
    // The 3-group Italian shape still works and also stops cleanly.
    assert_eq!(
        detect("num +39 333 000 0001 99999 fine"),
        vec![(PiiKind::Phone, "+39 333 000 0001".to_string())]
    );
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

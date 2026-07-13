//! Property-based tests — the port of the old proxy's `test_pii_hypothesis.py`.
//!
//! PROP-01: generated valid PII is always detected, masked, and round-tripped.
//! PROP-02: random alphabetic strings are never detected (no false positives).

use proptest::prelude::*;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::PiiDetector;
use llm_proxy_pii_rust::pii::PiiKind;

/// Detect the single expected entity, then assert the mask/demask round-trip is
/// exact and the raw value is gone from the masked text.
fn assert_single_entity_roundtrip(
    input: &str,
    expected_kind: PiiKind,
) -> Result<(), TestCaseError> {
    let detector = StructuredRecognizers::new();
    let entities = detector.detect(input);

    prop_assert_eq!(entities.len(), 1, "exactly one entity in {:?}", input);
    prop_assert_eq!(entities[0].kind, expected_kind);
    prop_assert_eq!(&entities[0].text, input);

    let mut vault = Vault::new();
    let masked = vault.mask(input, &entities);
    prop_assert!(!masked.contains(input), "raw value leaked: {:?}", masked);
    prop_assert_eq!(vault.demask(&masked), input.to_string());
    Ok(())
}

proptest! {
    /// PROP-01a — generated emails.
    #[test]
    fn generated_email_is_detected_and_roundtripped(
        email in "[a-z][a-z0-9]{0,9}@[a-z]{2,10}\\.[a-z]{2,4}"
    ) {
        assert_single_entity_roundtrip(&email, PiiKind::Email)?;
    }

    /// PROP-01b — generated US phone numbers.
    #[test]
    fn generated_phone_is_detected_and_roundtripped(
        phone in "[0-9]{3}-[0-9]{3}-[0-9]{4}"
    ) {
        assert_single_entity_roundtrip(&phone, PiiKind::Phone)?;
    }

    /// PROP-01c — generated US SSNs.
    #[test]
    fn generated_ssn_is_detected_and_roundtripped(
        ssn in "[0-9]{3}-[0-9]{2}-[0-9]{4}"
    ) {
        assert_single_entity_roundtrip(&ssn, PiiKind::Ssn)?;
    }

    /// PROP-02 — plain alphabetic text (letters + spaces) is never flagged.
    #[test]
    fn alphabetic_text_has_no_false_positives(text in "[a-zA-Z ]{0,60}") {
        let detector = StructuredRecognizers::new();
        prop_assert!(
            detector.detect(&text).is_empty(),
            "false positive in {:?}",
            text
        );
    }
}

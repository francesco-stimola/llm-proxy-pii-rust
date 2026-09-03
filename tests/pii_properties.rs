//! Property-based tests — the port of the old proxy's `test_pii_hypothesis.py`.
//!
//! PROP-01: generated valid PII is always detected, masked, and round-tripped.
//! PROP-02: random alphabetic strings are never detected (no false positives).
//! PROP-05: a VAT number is restored byte-exactly whatever surrounds it (M11).

use proptest::prelude::*;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::Budget;
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

    /// **PROP-05 (M11 Track A) — a VAT number is restored byte-exactly whatever surrounds it.**
    ///
    /// The question a new recognizer has to answer, and the one no corpus test can: *is there an
    /// input where a VAT number is detected but not restored, or restored wrong?* `VAT-13` pins
    /// the round trip for two hand-written strings and `VAT-18`/`VAT-19` pin it over HTTP; this
    /// quantifies over **arbitrary neighbours**, which is where truncation and adjacency defects
    /// actually live — a masked span that eats one byte too many, or a placeholder that a
    /// neighbouring digit run makes ambiguous to the de-masker.
    ///
    /// **Every case carries a real published VAT number**, so the property can never be
    /// satisfied by "nothing was detected" — the assertion that the known value is gone from the
    /// masked text is the non-vacuity check, and it also guarantees `mask_all` did real work.
    /// The random part is deliberately a mix of valid and invalid, prefixed and bare, so the
    /// neighbour is sometimes another span and sometimes just digits.
    ///
    /// **What this does NOT claim.** `TESTING.md`'s note on the PROP-03/PROP-04 pair describes a
    /// third property — *no byte of a real value remains, including one the validator declined* —
    /// needed by any recognizer whose regex can match a **superset** of the real value. The VAT
    /// patterns are all pinned at both ends with `(?-u:\b)` and none sets `shrink_on_reject`, so
    /// by that note's own criterion they are in the group that does not need it. This is the
    /// exact-restore property, not that one.
    #[test]
    fn a_vat_number_round_trips_exactly_whatever_surrounds_it(
        known in prop::sample::select(vec![
            "00905811006",    // IT bare — ENI
            "IT00159560366",  // IT VIES — Ferrari
            "DE136695976",    // DE — the documented vector
            "PT504499777",    // PT — Galp
            "NL111222333B01", // NL — format-anchored
        ]),
        before in "[a-z .,]{0,12}",
        neighbour in "(IT|DE|GB|PT|NL|ES|FR|)[0-9]{8,12}(B[0-9]{2})?",
        after in "[a-z .,]{0,12}",
    ) {
        // The known value is separated from `before` on purpose. Glue it to a word
        // character and `(?-u:\b)` correctly refuses to match it — that is VAT-08's
        // subject, and the first run of this property found it in two cases ("a" +
        // "00905811006"). Leaving it in would make the non-vacuity assertion below fail
        // on inputs where NOT detecting is the right answer, which tests the generator
        // rather than the product. Arbitrary adjacency is still exercised where it is
        // interesting: around `neighbour`, the second span.
        let text = format!("{before} {known}, {neighbour}{after}");
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();

        let masked = vault
            .mask_all(&text, &detector, &Budget::per_call())
            .map_err(|e| TestCaseError::fail(format!("masking must converge: {e:?}")))?;

        // Non-vacuity AND the privacy claim in one: the known value was really detected, so it
        // is really gone. A case where nothing matched cannot satisfy this.
        prop_assert!(
            !masked.contains(known),
            "the known VAT {:?} survived masking in {:?} -> {:?}",
            known,
            text,
            masked
        );
        // The whole point: exact restore, byte for byte, however the neighbours fell.
        prop_assert_eq!(vault.demask(&masked), text);
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

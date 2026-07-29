//! Data-driven tests over `tests/corpus/pii_cases.json`.
//!
//! The corpus is the single source of truth for structured-PII behaviour: each
//! recognizer case asserts the expected entity kind + text, the validator cases
//! exercise Luhn / IBAN mod-97, and the round-trip cases assert the vault
//! invariants (raw value absent, exact restore). See `docs/TESTING.md`.

use std::collections::HashMap;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::recognizers::{
    iban_mod97, luhn_valid, StructuredRecognizers, PHONE_REGIONS,
};
use llm_proxy_pii_rust::pii::PiiDetector;
use llm_proxy_pii_rust::pii::PiiKind;

/// The corpus is embedded at compile time so the test never depends on the cwd.
const CORPUS_JSON: &str = include_str!("corpus/pii_cases.json");

#[derive(Deserialize)]
struct Corpus {
    recognizers: HashMap<String, Category>,
    validators: Validators,
    vault_roundtrip: Vec<RoundtripCase>,
}

#[derive(Deserialize)]
struct Category {
    positive: Vec<DetectionCase>,
    negative: Vec<DetectionCase>,
}

#[derive(Deserialize)]
struct DetectionCase {
    id: String,
    input: String,
    /// Which country's rendering this case is. Decoration on most categories; **load-bearing
    /// on `national_phone`**, where `phone_regions_all_have_corpus_cases` reads it.
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    entities: Vec<ExpectedEntity>,
}

#[derive(Deserialize)]
struct ExpectedEntity {
    kind: PiiKind,
    text: String,
}

#[derive(Deserialize)]
struct Validators {
    luhn: ValidSet,
    iban_mod97: ValidSet,
}

#[derive(Deserialize)]
struct ValidSet {
    valid: Vec<String>,
    invalid: Vec<String>,
}

#[derive(Deserialize)]
struct RoundtripCase {
    id: String,
    input: String,
    #[serde(default)]
    no_pii: bool,
}

fn load() -> Corpus {
    serde_json::from_str(CORPUS_JSON).expect("corpus JSON must parse")
}

fn detected_pairs(detector: &StructuredRecognizers, input: &str) -> Vec<(PiiKind, String)> {
    detector
        .detect(input)
        .into_iter()
        .map(|e| (e.kind, e.text))
        .collect()
}

#[test]
fn recognizer_positives_match_expected() {
    let corpus = load();
    let detector = StructuredRecognizers::new();
    for (category, cases) in &corpus.recognizers {
        for case in &cases.positive {
            let expected: Vec<(PiiKind, String)> = case
                .entities
                .iter()
                .map(|e| (e.kind, e.text.clone()))
                .collect();
            let got = detected_pairs(&detector, &case.input);
            assert_eq!(
                got, expected,
                "[{category}/{}] input {:?}",
                case.id, case.input
            );
        }
    }
}

#[test]
fn recognizer_negatives_detect_nothing() {
    let corpus = load();
    let detector = StructuredRecognizers::new();
    for (category, cases) in &corpus.recognizers {
        for case in &cases.negative {
            let got = detected_pairs(&detector, &case.input);
            assert!(
                got.is_empty(),
                "[{category}/{}] expected no PII in {:?}, got {got:?}",
                case.id,
                case.input
            );
        }
    }
}

/// **PHONE-COV (M10).** Every region the **code** enables has corpus cases — enumerated
/// from `PHONE_REGIONS`, not from a list in this file.
///
/// A country added silently is the failure mode this milestone exists to end, and it is the
/// one a checklist does not catch: `PII_LOCALES=it,us` shipped for two milestones naming
/// regions that mapped to no recognizer at all, and every test that exercised a domestic
/// number passed its region in explicitly, so nothing noticed. Deriving the requirement from
/// the enabled set inverts that — switching a region on without evidence now **fails the
/// suite**.
///
/// Both directions are checked. A region with no cases is unverified coverage; a `locale`
/// naming a region the code does not enable is a case that silently tests nothing.
#[test]
fn phone_regions_all_have_corpus_cases() {
    let corpus = load();
    let cases = corpus
        .recognizers
        .get("national_phone")
        .expect("the corpus must carry a `national_phone` category");

    let locales_of = |set: &Vec<DetectionCase>| -> Vec<String> {
        set.iter()
            .map(|c| {
                c.locale
                    .clone()
                    .unwrap_or_else(|| panic!("[national_phone/{}] needs a `locale`", c.id))
            })
            .collect()
    };
    let positives = locales_of(&cases.positive);
    let negatives = locales_of(&cases.negative);

    let enabled: Vec<&str> = PHONE_REGIONS.iter().map(|r| r.code).collect();
    for code in &enabled {
        assert!(
            positives.iter().any(|l| l == code),
            "region `{code}` is enabled by the code but has no POSITIVE corpus case — a \
             region must never be switched on without evidence that its numbers are detected"
        );
        assert!(
            negatives.iter().any(|l| l == code),
            "region `{code}` is enabled by the code but has no NEGATIVE corpus case — \
             enabling a region widens the accepted set, so each one owes look-alikes of its \
             own that must NOT be masked"
        );
    }
    for locale in positives.iter().chain(negatives.iter()) {
        assert!(
            enabled.contains(&locale.as_str()),
            "corpus case locale `{locale}` names a region the code does not enable — the \
             case runs, matches nothing in particular, and reads as coverage it isn't"
        );
    }
}

#[test]
fn luhn_validator_matches_corpus() {
    let corpus = load();
    for number in &corpus.validators.luhn.valid {
        assert!(luhn_valid(number), "expected Luhn-valid: {number}");
    }
    for number in &corpus.validators.luhn.invalid {
        assert!(!luhn_valid(number), "expected Luhn-invalid: {number}");
    }
}

#[test]
fn iban_mod97_validator_matches_corpus() {
    let corpus = load();
    for iban in &corpus.validators.iban_mod97.valid {
        assert!(iban_mod97(iban), "expected mod-97 valid: {iban}");
    }
    for iban in &corpus.validators.iban_mod97.invalid {
        assert!(!iban_mod97(iban), "expected mod-97 invalid: {iban}");
    }
}

#[test]
fn vault_roundtrip_is_exact() {
    let corpus = load();
    let detector = StructuredRecognizers::new();
    for case in &corpus.vault_roundtrip {
        let mut vault = Vault::new();
        let entities = detector.detect(&case.input);
        let masked = vault.mask(&case.input, &entities);
        let restored = vault.demask(&masked);

        assert_eq!(
            restored, case.input,
            "[{}] round-trip must be exact",
            case.id
        );

        if case.no_pii {
            assert_eq!(
                masked, case.input,
                "[{}] no-PII text must be unchanged",
                case.id
            );
        } else {
            assert_ne!(masked, case.input, "[{}] PII must be masked", case.id);
            for entity in &entities {
                assert!(
                    !masked.contains(&entity.text),
                    "[{}] raw value {:?} leaked into masked text",
                    case.id,
                    entity.text
                );
            }
        }
    }
}

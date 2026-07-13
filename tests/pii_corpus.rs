//! Data-driven tests over `tests/corpus/pii_cases.json`.
//!
//! The corpus is the single source of truth for structured-PII behaviour: each
//! recognizer case asserts the expected entity kind + text, the validator cases
//! exercise Luhn / IBAN mod-97, and the round-trip cases assert the vault
//! invariants (raw value absent, exact restore). See `docs/TESTING.md`.

use std::collections::HashMap;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::recognizers::{iban_mod97, luhn_valid, StructuredRecognizers};
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

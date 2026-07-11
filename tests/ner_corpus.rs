//! NER corpus (M2): structure + the REG-03 false-positive guard.
//!
//! Positive *recall* over this corpus is measured once an ONNX NER model is
//! wired (feature `onnx`, `OnnxNerDetector`) — see `docs/M2-NER-EVALUATION.md`.
//! Until then this test enforces what holds without a model: the **deterministic
//! layer must never guess an unstructured entity** — no `Person` / `Organization`
//! / `Location` may come out of the structured recognizers (REG-03).

use std::collections::HashMap;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::PiiDetector;
use llm_proxy_pii_rust::pii::PiiKind;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;

const CORPUS_JSON: &str = include_str!("corpus/ner_cases.json");

#[derive(Deserialize)]
struct Corpus {
    entities: HashMap<String, Category>,
    multilingual_preview: Vec<Case>,
}

#[derive(Deserialize)]
struct Category {
    positive: Vec<Case>,
    negative: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    input: String,
    #[serde(default)]
    entities: Vec<ExpectedEntity>,
}

#[derive(Deserialize)]
struct ExpectedEntity {
    kind: PiiKind,
    #[allow(dead_code)]
    text: String,
}

fn load() -> Corpus {
    serde_json::from_str(CORPUS_JSON).expect("NER corpus JSON must parse")
}

fn is_unstructured(kind: PiiKind) -> bool {
    matches!(
        kind,
        PiiKind::Person | PiiKind::Organization | PiiKind::Location
    )
}

#[test]
fn corpus_is_well_formed_and_labels_are_unstructured() {
    let corpus = load();
    let mut positives = 0;
    for category in corpus.entities.values() {
        for case in category.positive.iter() {
            for entity in &case.entities {
                assert!(
                    is_unstructured(entity.kind),
                    "NER corpus positive {} has a non-NER label {:?}",
                    case.id,
                    entity.kind
                );
                positives += 1;
            }
        }
    }
    assert!(positives >= 6, "expected a few labelled positives, got {positives}");
    assert!(!corpus.multilingual_preview.is_empty());
}

#[test]
fn deterministic_layer_never_emits_unstructured_entities() {
    // REG-03 across the whole NER corpus, positives and negatives alike: the
    // structured recognizers must not guess names / orgs / locations.
    let corpus = load();
    let detector = StructuredRecognizers::new();

    let all_inputs = corpus
        .entities
        .values()
        .flat_map(|c| c.positive.iter().chain(c.negative.iter()))
        .chain(corpus.multilingual_preview.iter());

    for case in all_inputs {
        for entity in detector.detect(&case.input) {
            assert!(
                !is_unstructured(entity.kind),
                "[{}] deterministic layer wrongly emitted {:?} on {:?}",
                case.id,
                entity.kind,
                case.input
            );
        }
    }
}

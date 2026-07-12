//! NER evaluation harness (M2). Compiles only with `--features onnx`; the test is
//! `#[ignore]`d so it runs only on demand, when a real model is configured:
//!
//! ```text
//! set NER_MODEL_PATH=…\model.onnx
//! set NER_TOKENIZER_PATH=…\tokenizer.json
//! set NER_LABELS=O,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC   # class-id order
//! set NER_TOKEN_TYPE_IDS=1                                # BERT-family only
//! cargo test --features onnx --test ner_eval -- --ignored --nocapture
//! ```
//!
//! It scores the candidate **through the real hybrid** (structured recognizers +
//! NER, merged by the overlap resolver) against `tests/corpus/ner_cases.json`,
//! per `docs/M2-NER-EVALUATION.md`. Record the numbers in `docs/DEVLOG.md`.
#![cfg(feature = "onnx")]

use std::collections::HashMap;
use std::time::Instant;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::PiiKind;
use llm_proxy_pii_rust::pii::composite::CompositeDetector;
use llm_proxy_pii_rust::pii::onnx::OnnxNerDetector;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::{PiiDetector, PiiEntity};

const CORPUS_JSON: &str = include_str!("corpus/ner_cases.json");

#[derive(Deserialize)]
struct Corpus {
    entities: HashMap<String, Category>,
    #[serde(default)]
    multilingual_preview: Vec<Case>,
}
#[derive(Deserialize)]
struct Category {
    positive: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    input: String,
    #[serde(default)]
    entities: Vec<ExpectedEntity>,
}
#[derive(Deserialize)]
struct ExpectedEntity {
    kind: PiiKind,
    text: String,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn is_ner(kind: PiiKind) -> bool {
    matches!(kind, PiiKind::Person | PiiKind::Organization | PiiKind::Location)
}

#[derive(Default, Clone, Copy)]
struct Counts {
    tp: u32,
    fp: u32,
    fn_: u32,
}

#[test]
#[ignore = "requires a real ONNX NER model (set NER_MODEL_PATH / NER_TOKENIZER_PATH / NER_LABELS)"]
fn evaluate_ner_model_against_corpus() {
    let (Some(model), Some(tokenizer), Some(labels)) =
        (env("NER_MODEL_PATH"), env("NER_TOKENIZER_PATH"), env("NER_LABELS"))
    else {
        panic!("set NER_MODEL_PATH / NER_TOKENIZER_PATH / NER_LABELS to run the eval");
    };
    let id2label: Vec<String> = labels.split(',').map(|s| s.trim().to_string()).collect();
    let needs_tt = env("NER_TOKEN_TYPE_IDS").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    let ner = OnnxNerDetector::load(&model, &tokenizer, id2label, 1, needs_tt)
        .expect("load NER model");
    let detector = CompositeDetector::new(vec![
        Box::new(StructuredRecognizers::new()),
        Box::new(ner),
    ]);

    let corpus: Corpus = serde_json::from_str(CORPUS_JSON).expect("parse corpus");

    let mut per_kind: HashMap<PiiKind, Counts> = HashMap::new();
    let mut cases = 0u32;
    let started = Instant::now();

    let all_cases = corpus
        .entities
        .values()
        .flat_map(|c| c.positive.iter())
        .chain(corpus.multilingual_preview.iter());
    {
        for case in all_cases {
            cases += 1;
            let detected: Vec<(PiiKind, String)> = detector
                .detect(&case.input)
                .into_iter()
                .filter(|e: &PiiEntity| is_ner(e.kind))
                .map(|e| (e.kind, e.text))
                .collect();
            let expected: Vec<(PiiKind, String)> = case
                .entities
                .iter()
                .filter(|e| is_ner(e.kind))
                .map(|e| (e.kind, e.text.clone()))
                .collect();

            if detected != expected {
                eprintln!("  input:    {:?}", case.input);
                eprintln!("    expected: {expected:?}");
                eprintln!("    detected: {detected:?}");
            }

            for exp in &expected {
                let entry = per_kind.entry(exp.0).or_default();
                if detected.contains(exp) {
                    entry.tp += 1;
                } else {
                    entry.fn_ += 1;
                }
            }
            for det in &detected {
                if !expected.contains(det) {
                    per_kind.entry(det.0).or_default().fp += 1;
                }
            }
        }
    }

    let elapsed = started.elapsed();
    eprintln!("\n=== NER eval ({cases} cases, {:?} total) ===", elapsed);
    eprintln!("{:<14} {:>6} {:>6} {:>6}  {:>7} {:>7} {:>7}", "kind", "TP", "FP", "FN", "recall", "prec", "F1");
    for kind in [PiiKind::Person, PiiKind::Organization, PiiKind::Location] {
        let c = per_kind.get(&kind).copied().unwrap_or_default();
        let recall = ratio(c.tp, c.tp + c.fn_);
        let prec = ratio(c.tp, c.tp + c.fp);
        let f1 = if recall + prec > 0.0 { 2.0 * recall * prec / (recall + prec) } else { 0.0 };
        eprintln!(
            "{:<14} {:>6} {:>6} {:>6}  {:>7.3} {:>7.3} {:>7.3}",
            format!("{kind:?}"), c.tp, c.fp, c.fn_, recall, prec, f1
        );
    }
    eprintln!("(recall is the number that matters — a miss is a leak)\n");
}

fn ratio(num: u32, den: u32) -> f64 {
    if den == 0 { 0.0 } else { num as f64 / den as f64 }
}

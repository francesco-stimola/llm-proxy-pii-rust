//! NER evaluation harness (M2). Compiles only with `--features onnx`; the test is
//! `#[ignore]`d so it runs only on demand, when a real model is configured.
//!
//! Two ways to point it at a model — this is the only place the M2.5 auto-download
//! is exercised:
//!
//! ```text
//! # (A) Opt-in auto-download into the standard HF cache (M2.5), labels derived
//! #     from config.json — how the picked XLM-R int8 was scored:
//! set NER_MODEL_REPO=jiting/xlm-roberta-base-ner-hrl_onnx
//! set NER_MODEL_REVISION=478a2a3          # pinned; the default too
//! cargo test-onnx --test ner_eval -- --ignored --nocapture
//!
//! # (B) Explicit local files:
//! set NER_MODEL_PATH=…\model.onnx
//! set NER_TOKENIZER_PATH=…\tokenizer.json
//! set NER_LABELS=O,B-DATE,I-DATE,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC   # class-id order
//! set NER_TOKEN_TYPE_IDS=1                                             # BERT-family only
//! cargo test-onnx --test ner_eval -- --ignored --nocapture
//! ```
//!
//! It scores the candidate **through the real hybrid** (structured recognizers +
//! NER, merged by the overlap resolver) against `tests/corpus/ner_cases.json`,
//! per `docs/M2-NER-EVALUATION.md`. Record the numbers in `docs/DEVLOG.md`.
#![cfg(feature = "onnx")]

use std::collections::HashMap;
use std::time::Instant;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::composite::CompositeDetector;
use llm_proxy_pii_rust::pii::onnx::{ExecutionProvider, OnnxNerDetector};
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::PiiKind;
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
    matches!(
        kind,
        PiiKind::Person | PiiKind::Organization | PiiKind::Location
    )
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
    let needs_tt =
        env("NER_TOKEN_TYPE_IDS").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    // Two model sources (mirrors `src/server.rs::load_onnx_ner`): an explicit
    // local file, or the opt-in revision-pinned `hf-hub` fetch (M2.5). This
    // harness is the **only** place the real download is exercised.
    let (model, tokenizer, id2label): (String, String, Vec<String>) = if let Some(repo) =
        env("NER_MODEL_REPO")
    {
        use llm_proxy_pii_rust::pii::hf::HfModelSpec;
        let spec = HfModelSpec {
            repo,
            revision: env("NER_MODEL_REVISION").unwrap_or_else(|| "478a2a3".to_string()),
            model_file: env("NER_MODEL_FILE")
                .unwrap_or_else(|| "onnx/model_quantized.onnx".to_string()),
            tokenizer_file: env("NER_TOKENIZER_FILE")
                .unwrap_or_else(|| "tokenizer.json".to_string()),
            config_file: env("NER_CONFIG_FILE").unwrap_or_else(|| "config.json".to_string()),
        };
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let resolved = rt
            .block_on(spec.resolve())
            .expect("resolve model via hf-hub");
        // An explicit NER_LABELS overrides the config-derived labels.
        let id2label = match env("NER_LABELS") {
            Some(labels) => labels.split(',').map(|s| s.trim().to_string()).collect(),
            None => resolved.id2label,
        };
        (
            resolved.model_path.to_string_lossy().into_owned(),
            resolved.tokenizer_path.to_string_lossy().into_owned(),
            id2label,
        )
    } else {
        let (Some(model), Some(tokenizer), Some(labels)) = (
            env("NER_MODEL_PATH"),
            env("NER_TOKENIZER_PATH"),
            env("NER_LABELS"),
        ) else {
            panic!(
                    "set NER_MODEL_REPO (auto-download) or NER_MODEL_PATH / NER_TOKENIZER_PATH / NER_LABELS to run the eval"
                );
        };
        let id2label = labels.split(',').map(|s| s.trim().to_string()).collect();
        (model, tokenizer, id2label)
    };

    eprintln!("scoring model: {model}");
    // pool=1, intra=1: this harness scores *recall*, not latency — pinning both knobs keeps a
    // score reproducible across boxes with different core counts.
    let ner = OnnxNerDetector::load(
        &model,
        &tokenizer,
        id2label,
        1,
        1,
        needs_tt,
        ExecutionProvider::Cpu,
    )
    .expect("load NER model");
    let detector =
        CompositeDetector::new(vec![Box::new(StructuredRecognizers::new()), Box::new(ner)]);

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

            tally(&expected, &detected, &mut per_kind);
        }
    }

    let elapsed = started.elapsed();
    eprintln!("\n=== NER eval ({cases} cases, {:?} total) ===", elapsed);
    eprintln!(
        "{:<14} {:>6} {:>6} {:>6}  {:>7} {:>7} {:>7}",
        "kind", "TP", "FP", "FN", "recall", "prec", "F1"
    );
    for kind in [PiiKind::Person, PiiKind::Organization, PiiKind::Location] {
        let c = per_kind.get(&kind).copied().unwrap_or_default();
        let recall = ratio(c.tp, c.tp + c.fn_);
        let prec = ratio(c.tp, c.tp + c.fp);
        let f1 = if recall + prec > 0.0 {
            2.0 * recall * prec / (recall + prec)
        } else {
            0.0
        };
        eprintln!(
            "{:<14} {:>6} {:>6} {:>6}  {:>7.3} {:>7.3} {:>7.3}",
            format!("{kind:?}"),
            c.tp,
            c.fp,
            c.fn_,
            recall,
            prec,
            f1
        );
    }
    eprintln!("(recall is the number that matters — a miss is a leak)\n");
}

fn ratio(num: u32, den: u32) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

/// Accumulate span-level TP/FP/FN per kind for one case, as **multisets** (M2-R10).
///
/// An expected entity is a true positive only if a *distinct* detection matches
/// it: for each `(kind, text)` key, `tp = min(expected, detected)`. Set-membership
/// (`Vec::contains`) would let two identical expected entities both match a single
/// detection (recall 0.5 → 1.0) and would miss a spurious duplicate detection as
/// an FP — a silent recall inflation in a leak-measurement harness.
fn tally(
    expected: &[(PiiKind, String)],
    detected: &[(PiiKind, String)],
    per_kind: &mut HashMap<PiiKind, Counts>,
) {
    let exp = multiset(expected);
    let det = multiset(detected);
    // Every key that appears on either side (detected-only keys are pure FP).
    let mut keys: Vec<&(PiiKind, String)> = exp.keys().collect();
    keys.extend(det.keys().filter(|k| !exp.contains_key(*k)));
    for key in keys {
        let e = exp.get(key).copied().unwrap_or(0);
        let d = det.get(key).copied().unwrap_or(0);
        let tp = e.min(d);
        let entry = per_kind.entry(key.0).or_default();
        entry.tp += tp;
        entry.fn_ += e - tp;
        entry.fp += d - tp;
    }
}

/// Count identical `(kind, text)` entities into a multiset.
fn multiset(items: &[(PiiKind, String)]) -> HashMap<(PiiKind, String), u32> {
    let mut m: HashMap<(PiiKind, String), u32> = HashMap::new();
    for it in items {
        *m.entry(it.clone()).or_default() += 1;
    }
    m
}

#[test]
fn tally_counts_duplicates_as_multiset() {
    // Two identical expected entities, the model finds it once → recall 0.5, not
    // 1.0 (the `Vec::contains` bug would have matched both against one hit).
    let expected = vec![
        (PiiKind::Person, "Mario".to_string()),
        (PiiKind::Person, "Mario".to_string()),
    ];
    let detected = vec![(PiiKind::Person, "Mario".to_string())];
    let mut per_kind = HashMap::new();
    tally(&expected, &detected, &mut per_kind);
    let c = per_kind[&PiiKind::Person];
    assert_eq!(
        (c.tp, c.fn_, c.fp),
        (1, 1, 0),
        "one of two duplicates is a miss"
    );

    // A spurious duplicate detection is a false positive, not free.
    let expected = vec![(PiiKind::Person, "Mario".to_string())];
    let detected = vec![
        (PiiKind::Person, "Mario".to_string()),
        (PiiKind::Person, "Mario".to_string()),
    ];
    let mut per_kind = HashMap::new();
    tally(&expected, &detected, &mut per_kind);
    let c = per_kind[&PiiKind::Person];
    assert_eq!(
        (c.tp, c.fn_, c.fp),
        (1, 0, 1),
        "the extra detection is an FP"
    );
}

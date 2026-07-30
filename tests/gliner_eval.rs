//! GLiNER evaluation + smoke harness (M8). Compiles only with `--features onnx`;
//! the tests are `#[ignore]`d so they run only on demand, when a real model is
//! configured.
//!
//! ```text
//! set GLINER_MODEL_PATH=…\onnx\model_quantized.onnx
//! set GLINER_TOKENIZER_PATH=…\tokenizer.json
//! set GLINER_CONFIG_PATH=…\gliner_config.json      # max_len / max_width / <<ENT>> / <<SEP>>
//! set GLINER_LABELS=person,organization,location,phone number,address   # optional (default set)
//! set GLINER_THRESHOLD=0.5                                              # optional
//! cargo test-onnx --test gliner_eval -- --ignored --nocapture
//! ```
//!
//! - `smoke_gliner_detects_known_entities` (**SMOKE-GLINER**) is the S0 validation: it
//!   proves the tensor construction is right on the **real** export — if `words_mask` /
//!   `span_idx` / the logits layout were wrong, the known sentences below would not decode.
//! - `evaluate_gliner_against_corpus` scores it **through the real hybrid** on
//!   `tests/corpus/ner_cases.json`, per `docs/M2-NER-EVALUATION.md`. Record numbers in DEVLOG.
#![cfg(feature = "onnx")]

use std::collections::HashMap;
use std::time::Instant;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::composite::CompositeDetector;
use llm_proxy_pii_rust::pii::gliner::{GLiNerDetector, GlinerParams, DEFAULT_THRESHOLD};
use llm_proxy_pii_rust::pii::gliner_decode::{default_gliner_labels, parse_gliner_labels};
use llm_proxy_pii_rust::pii::onnx::ExecutionProvider;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::{Budget, PiiDetector, PiiEntity, PiiKind};

const CORPUS_JSON: &str = include_str!("corpus/ner_cases.json");

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Build a GLiNER detector from the env config, or skip (return `None`) if unset.
fn build_gliner() -> Option<GLiNerDetector> {
    let (model, tokenizer, config) = (
        env("GLINER_MODEL_PATH")?,
        env("GLINER_TOKENIZER_PATH")?,
        env("GLINER_CONFIG_PATH")?,
    );
    let params = GlinerParams::from_config_json(
        &std::fs::read_to_string(&config).expect("read gliner config"),
    )
    .expect("parse gliner config");
    let labels = match env("GLINER_LABELS") {
        Some(spec) => parse_gliner_labels(&spec).expect("parse GLINER_LABELS"),
        None => default_gliner_labels(),
    };
    let threshold = env("GLINER_THRESHOLD")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_THRESHOLD);
    // pool=1, intra=1: recall harness, not latency — reproducible across boxes.
    Some(
        GLiNerDetector::load(
            &model,
            &tokenizer,
            labels,
            params,
            threshold,
            1,
            1,
            ExecutionProvider::Cpu,
        )
        .expect("load GLiNER model"),
    )
}

#[test]
#[ignore = "requires the real GLiNER model (set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH)"]
fn smoke_gliner_detects_known_entities() {
    let Some(gliner) = build_gliner() else {
        panic!("set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH to run the smoke test");
    };

    let cases: &[(&str, &[(PiiKind, &str)])] = &[
        (
            "My name is Mario Rossi and I work at Google in Milano.",
            &[
                (PiiKind::Person, "Mario Rossi"),
                (PiiKind::Organization, "Google"),
                (PiiKind::Location, "Milano"),
            ],
        ),
        // The contextual recall gap M8 exists for: an un-anchored national phone.
        (
            "Call me on 020 7946 0958 tomorrow.",
            &[(PiiKind::Phone, "020 7946 0958")],
        ),
        (
            "Contact Anna Bianchi at Acme Corporation.",
            &[
                (PiiKind::Person, "Anna Bianchi"),
                (PiiKind::Organization, "Acme Corporation"),
            ],
        ),
    ];

    let mut ok = true;
    for (input, expected) in cases {
        let got: Vec<(PiiKind, String)> = gliner
            .detect(input)
            .into_iter()
            .map(|e: PiiEntity| (e.kind, e.text))
            .collect();
        eprintln!("input:    {input:?}\n  detected: {got:?}\n  expected: {expected:?}");
        for (k, t) in *expected {
            if !got.iter().any(|(gk, gt)| gk == k && gt == t) {
                eprintln!("  MISS: {k:?} {t:?}");
                ok = false;
            }
        }
    }
    assert!(
        ok,
        "SMOKE-GLINER: the model did not decode the known entities — tensor contract mismatch?"
    );
}

#[test]
#[ignore = "requires the real GLiNER model (set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH)"]
fn gliner_placeholder_inertness_canary() {
    // The model-swap canary the docs single out — "GLiNER especially" (M5-R4 / M7-R23).
    // Unlike XLM-R int8 (which `m5_r4` shows is inert), GLiNER is *zero-shot and
    // context-driven*, so it DOES tag our own `[KIND_N]` placeholders as entities. This
    // canary confirms that empirically AND proves the fixpoint is safe regardless:
    //   * `keep_maskable` drops an *exact* `[KIND_N]` hit by construction (CC-08), and
    //   * S4 keeps GLiNER off every pass after the first,
    // so `mask_all` on placeholder-dense text **converges** instead of 400ing.
    let Some(gliner) = build_gliner() else {
        panic!(
            "set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH to run the canary"
        );
    };
    let text = "Contact [PERSON_1] at [ORG_1] and call [PHONE_1] soon.";

    // (1) GLiNER tags placeholders — the risk the docs flag. If a future model stopped
    //     doing this the assert would fire, prompting a re-read of the invariant.
    let raw = gliner.detect(text);
    eprintln!(
        "GLiNER on placeholder text tagged {} spans (filter-leaning, as documented): {:?}",
        raw.len(),
        raw.iter()
            .map(|e| (e.kind, e.text.as_str()))
            .collect::<Vec<_>>()
    );
    assert!(
        !raw.is_empty(),
        "expected GLiNER to tag placeholders (the M5-R4 case); if it went inert, revisit the canary"
    );

    // (2) …but the fixpoint converges by construction — not a 400.
    let detector = CompositeDetector::new(vec![
        Box::new(StructuredRecognizers::new()),
        Box::new(gliner),
    ]);
    let mut vault = Vault::new();
    let masked = vault
        .mask_all(text, &detector, &Budget::per_call())
        .expect("mask_all must converge on placeholder-dense text, never fail closed");
    eprintln!("converged; masked = {masked:?}");
}

#[test]
#[ignore = "requires the real GLiNER model (set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH)"]
fn gliner_chunks_a_large_field_and_keeps_recall() {
    // M8-R2: exercise the **multi-window** path against the real model (every other gated input
    // is one short sentence). A field far longer than one window (`MAX_WINDOW_TEXT_TOKENS`) must
    //   (a) run **without error** — the chunker + the M8-R1 `max_len` choke-point guard don't
    //       crash or spuriously fail on a field that spans several windows; and
    //   (b) keep recall — a Person is still detected.
    //
    // The Person is placed **near the start** on purpose: GLiNER int8's confidence dilutes with
    // preceding context (DEVLOG M8), so an entity buried at the *end* of a long field is a
    // documented model weakness, not a chunking bug. What chunking must guarantee is that an
    // entity lands near *some* window's start (here, window 0) and is found. The long tail forces
    // the multi-window path around it.
    let Some(gliner) = build_gliner() else {
        panic!("set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH to run this");
    };
    let tail = " the meeting notes describe the process and the steps to follow".repeat(80);
    let text = format!("My colleague Mario Rossi will attend the review.{tail}");

    let got: Vec<(PiiKind, String)> = gliner
        .detect(&text)
        .into_iter()
        .map(|e| (e.kind, e.text))
        .collect();
    eprintln!("large field ({} bytes) → {got:?}", text.len());
    assert!(
        got.iter()
            .any(|(k, t)| *k == PiiKind::Person && t == "Mario Rossi"),
        "the multi-window path must run without error and keep the Person near window 0; got {got:?}"
    );
}

// ---- full corpus eval (recall/precision/F1 through the hybrid) ----

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
#[ignore = "requires the real GLiNER model (set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH)"]
fn evaluate_gliner_against_corpus() {
    let Some(gliner) = build_gliner() else {
        panic!(
            "set GLINER_MODEL_PATH / GLINER_TOKENIZER_PATH / GLINER_CONFIG_PATH to run the eval"
        );
    };
    let detector = CompositeDetector::new(vec![
        Box::new(StructuredRecognizers::new()),
        Box::new(gliner),
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

    let elapsed = started.elapsed();
    eprintln!("\n=== GLiNER eval ({cases} cases, {elapsed:?} total) ===");
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

/// Multiset TP/FP/FN per kind (mirrors ner_eval's M2-R10 tally).
fn tally(
    expected: &[(PiiKind, String)],
    detected: &[(PiiKind, String)],
    per_kind: &mut HashMap<PiiKind, Counts>,
) {
    let exp = multiset(expected);
    let det = multiset(detected);
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

fn multiset(items: &[(PiiKind, String)]) -> HashMap<(PiiKind, String), u32> {
    let mut m: HashMap<(PiiKind, String), u32> = HashMap::new();
    for it in items {
        *m.entry(it.clone()).or_default() += 1;
    }
    m
}

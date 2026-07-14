//! NER performance measurement (M5, PERF-01). Compiles only with `--features
//! onnx`; `#[ignore]`d — run on demand against a configured model, same as
//! `tests/ner_eval.rs`. Two questions this answers (see `docs/ARCHITECTURE.md`
//! → *Masking must be linear…* and `docs/DEVLOG.md` M4-R21/R24):
//!
//! 1. **The fixpoint's second detector pass (M4-R21).** `Vault::mask_all` runs
//!    the detector at least twice (mask, then confirm the fixpoint) — under NER
//!    that's ~2x inference cost. This measures the real factor, not just the
//!    one-off number recorded when M4-R21 was closed.
//! 2. **`OnnxNerDetector` chunking (`docs/ROADMAP.md` M5).** Before this measurement,
//!    it fed the whole field as one sequence; past the model's
//!    `max_position_embeddings` (514 for XLM-R base) that **errored outright**
//!    (an ONNX `Expand` op failure — measured, not a graceful slowdown), which
//!    would silently drop to structured-only by default or **block every such
//!    request** under `NER_REQUIRED`. `src/pii/onnx.rs` now chunks with overlap;
//!    this test pins that large fields keep finding entities instead of erroring.
//!
//! ```text
//! set NER_MODEL_PATH=…\model_quantized.onnx
//! set NER_TOKENIZER_PATH=…\tokenizer.json
//! set NER_LABELS=O,B-DATE,I-DATE,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC
//! cargo test --features onnx --test ner_perf -- --ignored --nocapture
//! ```
#![cfg(feature = "onnx")]

use std::time::Instant;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::onnx::OnnxNerDetector;
use llm_proxy_pii_rust::pii::PiiDetector;

fn load_detector() -> OnnxNerDetector {
    let model =
        std::env::var("NER_MODEL_PATH").expect("set NER_MODEL_PATH (see this file's doc comment)");
    let tokenizer = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    let labels = std::env::var("NER_LABELS").expect("set NER_LABELS");
    let id2label = labels.split(',').map(str::to_string).collect();
    OnnxNerDetector::load(&model, &tokenizer, id2label, 1, false).expect("load NER model")
}

/// One natural-language sentence carrying a name, repeated to build up field
/// size — the realistic shape (prose), not a degenerate repeated token.
const SENTENCE: &str = "Mario Rossi from Acme Corporation visited Milan yesterday. ";

#[test]
#[ignore]
fn m4_r21_the_fixpoints_second_pass_roughly_doubles_ner_inference() {
    let detector = load_detector();
    let input = SENTENCE.repeat(3); // one realistic short field, not pathological

    let started = Instant::now();
    let _ = detector.detect(&input);
    let single_pass = started.elapsed();

    let mut vault = Vault::new();
    let started = Instant::now();
    let _ = vault
        .mask_all(&input, &detector)
        .expect("masking must converge");
    let mask_all_cost = started.elapsed();

    let factor = mask_all_cost.as_secs_f64() / single_pass.as_secs_f64().max(1e-9);
    eprintln!(
        "detect() alone: {single_pass:?}; mask_all() (fixpoint-confirmed): {mask_all_cost:?}; \
         factor: {factor:.2}x"
    );
    // A generous ceiling: mask_all must run the detector a small, bounded
    // number of times (2-3 for text with no PII to converge on), never grow
    // unboundedly. This is a sanity bound, not a tight assertion — see the
    // module doc for why exact numbers belong in DEVLOG, not here.
    assert!(
        factor < 5.0,
        "mask_all cost {mask_all_cost:?} vs detect() {single_pass:?} — factor {factor:.2}x is \
         far above the expected ~2x fixpoint-confirmation cost"
    );
}

#[test]
#[ignore]
fn onnx_ner_latency_and_recall_across_field_sizes() {
    // Repeat the sentence to grow the field; report latency and entity count.
    // Before chunking, this errored past ~500 tokens (measured — an ONNX
    // `Expand` op failure from the model's position-embedding limit, not a
    // graceful slowdown); now it must keep finding entities at every size,
    // scaling roughly linearly (one more chunk every `stride` tokens).
    let detector = load_detector();

    for reps in [1usize, 4, 16, 64, 256, 1024] {
        let input = SENTENCE.repeat(reps);
        let expected = 3 * reps; // Person + Organization + Location per sentence
        let started = Instant::now();
        let entities = detector
            .try_detect(&input)
            .unwrap_or_else(|err| panic!("reps={reps}: NER call failed: {err}"));
        let elapsed = started.elapsed();
        eprintln!(
            "reps={reps:>5} ({} chars): {elapsed:?}, {}/{expected} entities found",
            input.len(),
            entities.len()
        );
        // Chunking is a recall mechanism, not an exact one (a boundary sliver
        // can still miss); allow a small tolerance but the vast majority must
        // still be found — this is the property that mattered ("silent total
        // loss past ~500 tokens" vs "an outright error"), not perfect recall.
        assert!(
            entities.len() as f64 >= 0.9 * expected as f64,
            "reps={reps}: only {}/{expected} entities found — chunking lost too much recall",
            entities.len()
        );
    }
}

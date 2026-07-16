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
//! cargo test-onnx --test ner_perf -- --ignored --nocapture
//! ```
#![cfg(feature = "onnx")]

use std::time::Instant;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::onnx::{
    chunk_char_ranges, OnnxNerDetector, CHUNK_OVERLAP_TOKENS, MAX_WINDOW_TOKENS, MODEL_MAX_TOKENS,
};
use llm_proxy_pii_rust::pii::PiiDetector;

fn load_detector() -> OnnxNerDetector {
    let model =
        std::env::var("NER_MODEL_PATH").expect("set NER_MODEL_PATH (see this file's doc comment)");
    let tokenizer = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    let labels = std::env::var("NER_LABELS").expect("set NER_LABELS");
    let id2label = labels.split(',').map(str::to_string).collect();
    OnnxNerDetector::load(&model, &tokenizer, id2label, 1, false).expect("load NER model")
}

fn load_tokenizer() -> tokenizers::Tokenizer {
    let path = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    tokenizers::Tokenizer::from_file(path).expect("load tokenizer")
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

/// Multi-script, no-inter-word-space, combining-mark-heavy fields — the shapes most likely to make
/// a re-tokenized window drift past the planning bound (CJK and scripts with no spaces give the
/// tokenizer no natural cut points, so a window edge lands mid-"word" and re-tokenizes differently).
fn adversarial_fields() -> Vec<(&'static str, String)> {
    vec![
        (
            "chinese",
            "我的信用卡号是4111111111111111，请联系张伟。".repeat(60),
        ),
        (
            "japanese",
            "カード番号は4111111111111111です。田中さんに連絡してください。".repeat(60),
        ),
        (
            "cyrillic",
            "Карта 4111111111111111, свяжитесь с Иваном Петровым в Москве. ".repeat(60),
        ),
        (
            "combining",
            "Z\u{0301}a\u{0308}l\u{0327}g\u{0308}o\u{0301} Mario Rossi Milano. ".repeat(80),
        ),
        (
            "mixed",
            "Mario Rossi 张伟 Иван 田中 Müller Łódź café ".repeat(100),
        ),
        ("no-spaces", "あ".repeat(4000)),
    ]
}

#[test]
#[ignore]
fn m5_r2_every_retokenized_window_stays_within_the_models_usable_length() {
    // M5-R2. `MAX_WINDOW_TOKENS` (480) plans windows in the coordinates of the WHOLE field's
    // tokenization — but `infer_chunked` then RE-TOKENIZES each window from its own text (it must:
    // a middle window needs its own <s>…</s> framing). That re-tokenization adds the two special
    // tokens and drifts at the cut edges, so the sequence actually handed to the model is
    // `window + specials + drift` — measured at 481-483, i.e. always OVER the planning bound.
    //
    // The real ceiling is `MODEL_MAX_TOKENS` (512 = XLM-R's 514 max_position_embeddings minus
    // RoBERTa's position-id offset of 2). Exceed it and the ONNX graph fails outright — the PERF-01
    // `Expand` error. `run_and_decode` refuses such a sequence with a named `Err` rather than
    // running it (and rather than silently clamping — that would decide the fail-open/fail-closed
    // posture inside the detector, which is `FailOpen`/`NER_REQUIRED`'s job; see M5-R7). That error
    // is the *fail-safe*. **This test is what keeps it from ever being the mechanism.**
    let tokenizer = load_tokenizer();

    for (name, input) in adversarial_fields() {
        let full = tokenizer.encode(input.as_str(), true).expect("tokenize");
        let ranges = chunk_char_ranges(
            &input,
            full.get_offsets(),
            MAX_WINDOW_TOKENS,
            CHUNK_OVERLAP_TOKENS,
        );

        let mut worst = 0usize;
        for &(start, end) in &ranges {
            let chunk = input
                .get(start..end)
                .unwrap_or_else(|| panic!("{name}: window {start}..{end} is not sliceable"));
            let len = tokenizer.encode(chunk, true).expect("tokenize chunk").len();
            worst = worst.max(len);
            assert!(
                len <= MODEL_MAX_TOKENS,
                "{name}: a re-tokenized window is {len} tokens, over the model's usable \
                 {MODEL_MAX_TOKENS} — it would be clamped (recall loss), and on a model with a \
                 tighter budget it would be the PERF-01 `Expand` error all over again"
            );
        }
        eprintln!(
            "{name:>10}: {} chars, {} windows, worst re-tokenized window {worst} tokens \
             (planning bound {MAX_WINDOW_TOKENS}, model ceiling {MODEL_MAX_TOKENS})",
            input.len(),
            ranges.len()
        );
        // Non-vacuity: a field that never chunks proves nothing about chunking.
        assert!(
            ranges.len() > 1,
            "{name}: this field must be large enough to chunk, or the guard is vacuous"
        );
    }
}

#[test]
#[ignore]
fn m5_r4_the_ner_treats_placeholders_as_inert() {
    // M5-R4. `Vault::mask_all` masks to a FIXPOINT, and ARCHITECTURE proves it converges because
    // "a placeholder is inert" — but that proof is about the REGEX RECOGNIZERS (`[KIND_N]` has no
    // `@`, no `sk-`, not enough digits, and `[`/`]` are outside every character class). It is a
    // construction proof, and it does NOT cover the NER: an ML model is under no such constraint,
    // and nothing stops one from tagging `[PERSON_1]` as a Person.
    //
    // If it did, a mask pass would replace a placeholder, the text would not strictly shrink,
    // MAX_MASK_PASSES would be exhausted, and the request would 400 — fail-closed (M4-R20 saw to
    // that), so never a leak, but a hard availability failure on ordinary input.
    //
    // It holds for XLM-R int8 — EMPIRICALLY, which is not the same as by construction. This is the
    // check a MODEL SWAP must not skip, and the Backlog's designated successor is GLiNER: a
    // zero-shot, open-label, CONTEXT-driven extractor, i.e. exactly the kind of model that might
    // look at `Contact [PERSON_1] at [ORG_1]` and tag both.
    let detector = load_detector();

    // Long enough to be chunked, so this covers the chunked path too (before M5's chunking fix, a
    // field this size never reached the model at all).
    let placeholders =
        "Contact [PERSON_1] at [ORG_1] in [LOCATION_1] about [EMAIL_1] and [PHONE_1]. ".repeat(40);
    assert!(
        placeholders.len() > 2_000,
        "must be large enough to exercise the chunked path"
    );

    let entities = detector
        .try_detect(&placeholders)
        .expect("NER must not error");
    assert!(
        entities.is_empty(),
        "the NER tagged {} entities in placeholder-only text — placeholder inertness does NOT \
         hold for this model, so `Vault::mask_all` can fail to reach a fixpoint and 400 on \
         ordinary input. Found: {:?}",
        entities.len(),
        entities
            .iter()
            .map(|e| (e.kind, e.text.as_str()))
            .collect::<Vec<_>>()
    );

    // …and the full hybrid really does converge on such a field.
    let structured = llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers::new();
    let composite = llm_proxy_pii_rust::pii::composite::CompositeDetector::new(vec![
        Box::new(structured),
        Box::new(load_detector()),
    ]);
    let mut vault = Vault::new();
    vault
        .mask_all(&placeholders, &composite)
        .expect("masking placeholder-dense text must reach a fixpoint, not exhaust its passes");
}

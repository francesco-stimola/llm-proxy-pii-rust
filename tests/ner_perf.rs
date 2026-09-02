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
//! cargo test-onnx --test ner_perf -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **`--test-threads=1` matters for the timing guards (M7-R12).** Cargo runs tests concurrently by
//! default, so without it these measure the NER against other copies of themselves — measured at
//! 1.5x on a sibling harness. The recall guards here are unaffected; the latency ones are not.
#![cfg(feature = "onnx")]

use std::time::Instant;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::onnx::{
    available_cores, chunk_char_ranges, ExecutionProvider, OnnxNerDetector, CHUNK_OVERLAP_TOKENS,
    MAX_WINDOW_TOKENS, MODEL_MAX_TOKENS,
};
use llm_proxy_pii_rust::pii::{Budget, PiiDetector};

fn load_detector() -> OnnxNerDetector {
    let model =
        std::env::var("NER_MODEL_PATH").expect("set NER_MODEL_PATH (see this file's doc comment)");
    let tokenizer = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    let labels = std::env::var("NER_LABELS").expect("set NER_LABELS");
    let id2label = labels.split(',').map(str::to_string).collect();
    // pool=1, intra=1: these guards measure *recall* and *pass counts*, not wall clock, so they
    // pin the historical single-threaded shape rather than inheriting M7's derived default (which
    // would make their numbers depend on the runner's core count).
    OnnxNerDetector::load(
        &model,
        &tokenizer,
        id2label,
        1,
        1,
        false,
        ExecutionProvider::Cpu,
    )
    .expect("load NER model")
}

fn load_tokenizer() -> tokenizers::Tokenizer {
    let path = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    tokenizers::Tokenizer::from_file(path).expect("load tokenizer")
}

/// **NER-EP-01 (M9-R1, M9-R2).** A requested-but-unavailable accelerator must (a) still load,
/// (b) report **CPU** as its effective provider, and (c) do so for a *multi-session* pool.
///
/// (a) is the fail-closed promise — a privacy proxy must start even when the named GPU isn't
/// there. (b) is the honesty promise M9 exists for: the detector must surface what it is
/// *running on*, because the startup log derives its `provider=` field from this, and a line
/// naming a GPU the process is not using is exactly the M7-R5 defect M9 reintroduced. (c) is the
/// homogeneity guarantee: each session decides independently whether the accelerator
/// initializes, so a pool could otherwise end up part-GPU/part-CPU with round-robin dispatch
/// making the backend a per-request variable — and detection would silently vary per request.
///
/// `Rocm` is requested because it is never present on this project's platforms, so the fallback
/// path is the one under test on every box. Run:
/// `cargo test-onnx --test ner_perf -- --ignored ner_ep_01`
#[test]
#[ignore = "requires the real NER model (set NER_MODEL_PATH / NER_TOKENIZER_PATH / NER_LABELS)"]
fn ner_ep_01_unavailable_provider_falls_back_to_cpu_for_the_whole_pool() {
    let model =
        std::env::var("NER_MODEL_PATH").expect("set NER_MODEL_PATH (see this file's doc comment)");
    let tokenizer = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    let labels = std::env::var("NER_LABELS").expect("set NER_LABELS");
    let id2label: Vec<String> = labels.split(',').map(str::to_string).collect();

    for pool_size in [1usize, 2] {
        let detector = OnnxNerDetector::load(
            &model,
            &tokenizer,
            id2label.clone(),
            pool_size,
            1,
            false,
            ExecutionProvider::Rocm,
        )
        .expect("an unavailable accelerator must fall back to CPU, never fail to load");

        assert_eq!(
            detector.provider(),
            ExecutionProvider::Cpu,
            "pool_size={pool_size}: the detector must report the EFFECTIVE provider (cpu after \
             fallback), not the requested one — the startup log derives `provider=` from this"
        );

        // And it must still actually detect: falling back is a latency change, not a masking one.
        let hits = detector.detect("My name is Angela Merkel and I live in Berlin.");
        assert!(
            !hits.is_empty(),
            "pool_size={pool_size}: the post-fallback CPU detector must still detect entities"
        );
    }
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
        .mask_all(&input, &detector, &Budget::per_call())
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
            .try_detect(&input, &Budget::per_call())
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

/// **M7-R3 — `NER_INTRA_THREADS` must change *speed*, never *detection*.**
///
/// M7's entire safety argument is that the thread knob is "purely a performance knob". That claim
/// is **empirical, not structural**, and until this test it was asserted by nothing: every recall
/// guard in the repo pins `intra=1` on purpose (a score that moves with the runner's core count is
/// worthless — see `load_detector` above), while production now derives `6` or more. So the one
/// knob M7 changed was the one knob no guard exercised. The M5-R4 shape exactly.
///
/// **Why this is not paranoia.** ONNX Runtime's intra-op parallelism repartitions GEMM and
/// reduction work across threads, and floating-point addition is **not associative** — a different
/// partition can change a logit in its last bits. The BIO decode is a per-token `argmax`
/// (`ner_decode`), so a near-tie between two labels can flip on nothing but thread count. A flipped
/// `B-PER` is lost recall, and in this repo a miss is what makes a leak.
///
/// It holds today for **XLM-R int8 on the CPU EP** — verified here, and that is the whole point:
/// **an execution-provider swap (the Backlog's DirectML/CUDA item, where cross-thread
/// non-determinism is genuinely likely) or a model swap (GLiNER) must re-run this.** Without it,
/// the only thing standing between either and a silent recall change is a reviewer thinking to
/// check by hand.
#[test]
#[ignore]
fn m7_r3_intra_threads_changes_speed_not_detection() {
    let model = std::env::var("NER_MODEL_PATH").expect("set NER_MODEL_PATH");
    let tokenizer = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    let labels = std::env::var("NER_LABELS").expect("set NER_LABELS");
    let id2label: Vec<String> = labels.split(',').map(str::to_string).collect();

    // Inputs chosen for where a partition difference could actually surface: prose with the
    // entities we care about, a field past the chunking window (a window-boundary decision is the
    // most likely place for two thread counts to disagree), and the fragment-prone shape that
    // produces M7's `("An", Organization)` over-mask, which is a near-tie by construction.
    //
    // **Assert the property the code branches on, in the unit it branches on (M7-R8).** The first
    // version asserted `long_field.len() > 2_000` — BYTES — for a decision `infer_chunked` makes on
    // TOKENS (`> MAX_WINDOW_TOKENS`). `SENTENCE.repeat(40)` is 2,360 bytes and clears that assert by
    // 18%, while being **442 tokens** — 38 short of the trigger. So the guard's most important input
    // ran as a single pass and it covered **zero** chunked inputs, under an assert claiming the
    // opposite. A byte count is a proxy; the tokenizer is the thing. (Third time this repo asserted
    // a proxy for what it meant — see M5-R10.)
    let long_field = SENTENCE.repeat(60);
    let long_tokens = load_tokenizer()
        .encode(long_field.as_str(), true)
        .expect("encode")
        .get_ids()
        .len();
    assert!(
        long_tokens > MAX_WINDOW_TOKENS,
        "the long input is {long_tokens} tokens, not over the {MAX_WINDOW_TOKENS}-token window — so \
         this guard never reaches `infer_chunked`, the likeliest place for two thread counts to \
         disagree, and it is exactly as vacuous as M7-R8 found it (M7-R8)"
    );
    let inputs: Vec<&str> = vec![
        SENTENCE,
        "Mario Rossi from Acme Corporation visited Milan yesterday.",
        "Anthropic's models are trained with a knowledge cutoff, so verify any claim.",
        "Il cliente Mario Rossi di Acme SpA a Milano ha scritto a mario.rossi@acme.it.",
        "张伟在北京为华为工作。",
        &long_field,
    ];

    // `(kind, span.start, span.end)` per input — spans included, because a shifted span is a
    // different mask even when the count is identical.
    let fingerprint = |intra: usize| -> Vec<Vec<(String, usize, usize)>> {
        let detector = OnnxNerDetector::load(
            &model,
            &tokenizer,
            id2label.clone(),
            1,
            intra,
            false,
            ExecutionProvider::Cpu,
        )
        .expect("load NER model");
        inputs
            .iter()
            .map(|input| {
                detector
                    .try_detect(input, &Budget::per_call())
                    .expect("NER must not error")
                    .iter()
                    .map(|e| (format!("{:?}", e.kind), e.span.start, e.span.end))
                    .collect()
            })
            .collect()
    };

    // `intra=1` is the shape every recall guard pins, so it is the baseline the corpus scores were
    // measured against.
    let baseline = fingerprint(1);
    let total: usize = baseline.iter().map(Vec::len).sum();
    assert!(
        total > 10,
        "the baseline found only {total} entities — a guard that detects almost nothing would \
         pass this comparison vacuously"
    );
    eprintln!("intra= 1 (the shape every recall guard pins): {total} entities");

    // **Sweeps up to the LOGICAL count, and must keep doing so after M11 Track B.** The shipped
    // default now derives from the *thread base* (physical cores capped by the granted
    // parallelism), so nothing ships at `intra = available_cores()` on an SMT box any more —
    // narrowing this sweep to the base would nonetheless be a mistake. This is a
    // detection-inertness guard, not a shape-of-the-default guard: it exists to prove that
    // *however* ONNX Runtime partitions the work, the detections do not move. More partitions is
    // strictly more coverage, and the logical count is the largest partitioning a caller can
    // actually request through `NER_INTRA_THREADS` on this box.
    for intra in [2usize, 4, 6, available_cores()] {
        let got = fingerprint(intra);
        let differing = baseline.iter().zip(&got).filter(|(a, b)| a != b).count();
        eprintln!(
            "intra={intra:>2}: {} entities, {differing} inputs differ from the intra=1 baseline",
            got.iter().map(Vec::len).sum::<usize>()
        );
        assert_eq!(
            got, baseline,
            "intra={intra} produced DIFFERENT detections than intra=1. `NER_INTRA_THREADS` is \
             documented as a performance knob; if this fires, it silently changes recall — which \
             is a leak-relevant regression, not a perf one. Suspect a changed execution provider \
             or model (see this test's doc comment)."
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
    // Tokens, not bytes (M7-R16 — the twin of M7-R8, in this same file). This assert used to read
    // `placeholders.len() > 2_000` and claim it "exercises the chunked path", for a branch
    // `infer_chunked` takes on `> MAX_WINDOW_TOKENS`. It happened to be true — 1,162 tokens, ~2.4x
    // the window — but only because placeholder-dense text tokenizes ~2.7x denser per byte than
    // prose (`[`, `PERSON`, `_`, `1`, `]` each cost a token), so the byte proxy over-shot into
    // correctness. That is a property of this string's *character*, not of the assert: reword it,
    // lower the repeat, or swap the tokenizer's vocabulary, and the assert keeps passing while the
    // field silently stops chunking — and placeholder inertness (M5-R4), the property a MODEL SWAP
    // must re-check, would go unchecked on the chunked path.
    let placeholder_tokens = load_tokenizer()
        .encode(placeholders.as_str(), true)
        .expect("encode")
        .get_ids()
        .len();
    assert!(
        placeholder_tokens > MAX_WINDOW_TOKENS,
        "the placeholder field is {placeholder_tokens} tokens, not over the {MAX_WINDOW_TOKENS}-token \
         window — so this guard never reaches `infer_chunked` and placeholder inertness goes \
         unchecked on the chunked path (M7-R16)"
    );

    let entities = detector
        .try_detect(&placeholders, &Budget::per_call())
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
        .mask_all(&placeholders, &composite, &Budget::per_call())
        .expect("masking placeholder-dense text must reach a fixpoint, not exhaust its passes");
}

/// S4 (CC-05/CC-08), live. A field dense with product/org names makes the NER emit **sub-word
/// fragments** (`"Slack"` → `"lack"`, `"Anthropic"` → `"An"`); before S4 those chained past
/// `MAX_MASK_PASSES` on every pass and the request **400'd** — reproduced live on a real Claude
/// Code system prompt. With S4 the NER runs only on pass 0 ([`PiiDetector::redetect`] returns
/// nothing after), so `mask_all` converges. A model swap must keep this true; if a new NER
/// fragments *and* S4 regresses, this is the guard that catches it before a user does.
#[test]
#[ignore]
fn m7_s4_dense_org_names_converge_instead_of_400() {
    let composite = llm_proxy_pii_rust::pii::composite::CompositeDetector::new(vec![
        Box::new(
            llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers::with_locales(&[
                "it".to_string(),
                "us".to_string(),
            ]),
        ),
        Box::new(load_detector()),
    ]);
    // The exact shape the live 400s came from: dense product/org names, scaled well past the
    // ~5 KB where PLAIN (NER every pass) already needed 6 passes and grew from there.
    let dense = "You are Claude Code, Anthropic's official CLI for Claude. Claude Code integrates \
with Slack, GitHub, and the Claude API. The Claude Desktop app and Claude Code both talk to \
Anthropic. Use GitHub for issues, Slack for chat, and the Anthropic Console for billing. "
        .repeat(60);
    let mut vault = Vault::new();
    vault
        .mask_all(&dense, &composite, &Budget::per_call())
        .expect("S4: a dense system prompt must converge in one NER pass, not 400");
}

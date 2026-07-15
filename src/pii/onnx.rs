//! ONNX NER detector for UNSTRUCTURED entities (names, organizations,
//! locations) — milestone M2. Enabled by the `onnx` feature.
//!
//! CPU execution provider first (maximum compatibility/reproducibility); GPU
//! (CUDA / DirectML) comes later (M4) and is not automatic — it depends on the
//! model and its quantization.
//!
//! This module owns only the model I/O: tokenize (with a HF fast tokenizer),
//! run the ONNX session, and turn per-token logits into label ids. The actual
//! label→[`PiiKind`] mapping and BIO→span merge live in the model-independent
//! [`ner_decode`](super::ner_decode), which is unit-tested without a model.
//!
//! **Model contract:** a token-classification model with input `input_ids` +
//! `attention_mask` (and `token_type_ids` when `NER_TOKEN_TYPE_IDS` is set, e.g.
//! BERT-family models such as Piiranha) and a single output named `logits` of
//! shape `[1, seq, num_labels]`. `NER_LABELS` must list exactly `num_labels`
//! labels in class-id order — a mismatch is rejected (never silently degraded).
//!
//! **Chunking (M5, PERF-01).** A field longer than [`MAX_WINDOW_TOKENS`] is
//! split into overlapping windows rather than fed to the model whole. This
//! isn't a latency optimization: RoBERTa-family absolute position embeddings
//! top out at `max_position_embeddings` (514 for the picked XLM-R int8's
//! `config.json`), and a sequence past that limit makes the ONNX graph's
//! position-embedding lookup go **out of range** — measured
//! (`tests/ner_perf.rs`) as an outright `Expand` op failure, not a graceful
//! slowdown. Without chunking, any field over roughly 2 KB of prose (~500
//! tokens) fails NER outright — silently swallowed by the default fail-*open*
//! wrapper, but a hard **block** under `NER_REQUIRED` (every such request would
//! 400). See [`infer_chunked`](OnnxNerDetector::infer_chunked).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{Encoding, Tokenizer};

use super::ner_decode::{decode_entities, validate_label_count, TokenTag};
use super::overlap::widen_to_char_boundaries;
use super::{DetectError, PiiDetector, PiiEntity};

/// **The hard ceiling: the longest sequence the model can actually be handed.**
///
/// XLM-R declares `max_position_embeddings: 514`, but RoBERTa-family position ids start at
/// `pad_token_id + 1 = 2`, so the *usable* length is **512**. Past it the position-embedding
/// lookup goes out of range and the ONNX graph fails outright (the PERF-01 `Expand` error).
///
/// This is enforced where it matters — in [`run_and_decode`](OnnxNerDetector::run_and_decode),
/// the single choke point every path into the session goes through (M5-R2). It is *not* enough
/// to plan windows under a budget and hope: [`infer_chunked`](OnnxNerDetector::infer_chunked)
/// **re-tokenizes** each window from its own text, which adds the two special tokens and drifts
/// at the cut edges — measured at consistently **481–483** tokens for a 480-token window, i.e.
/// always *over* the planning bound. The drift is small for this tokenizer, but nothing
/// structurally bounds it, and the cost of being wrong is exactly the failure chunking exists to
/// prevent. So the bound is *checked*, not assumed.
pub const MODEL_MAX_TOKENS: usize = 512;

/// **The planning window** used to lay out chunks — deliberately *under* [`MODEL_MAX_TOKENS`] to
/// absorb the re-tokenization drift described there (measured +1…+3; 32 tokens of headroom here).
///
/// Note the distinction the name now carries and the old one didn't (M5-R2): this bounds the
/// **window**, not the **sequence**. The sequence is `window + specials + edge drift`, and only
/// [`MODEL_MAX_TOKENS`] bounds *that*.
pub const MAX_WINDOW_TOKENS: usize = 480;

/// Overlap (in tokens) between consecutive chunks, so an entity that would
/// otherwise land right at a chunk boundary is still whole in at least one
/// chunk. Generous relative to a Person/Org/Location span (rarely more than a
/// handful of tokens).
///
/// `pub` for the same reason [`chunk_char_ranges`] is (M5-R9): the live guard in
/// `tests/ner_perf.rs` drives the real chunker, and a hand-copied `32` there would be a second
/// home for this fact — free to drift from the one that matters.
pub const CHUNK_OVERLAP_TOKENS: usize = 32;

/// The re-tokenization headroom a window must leave below [`MODEL_MAX_TOKENS`]: the two special
/// tokens a re-tokenized chunk re-adds, plus the cut-edge drift (measured **+1…+3** for XLM-R —
/// but nothing *structurally* bounds it, so the margin itself is the guard). A tokenizer swap
/// re-opens this number.
///
/// **This — not the mere ordering `window < ceiling` — is the invariant the chunker relies on
/// (M5-R10).** `479 < 512` and `511 < 512` satisfy `<` identically, but a 511-token window
/// re-tokenizes to ~514 and the PERF-01 `Expand` error is back. The drift has "nowhere to go" the
/// moment the headroom drops below it, not the moment the window reaches the ceiling — so the
/// headroom is what must be pinned. It is deliberately larger than the measured +1…+3: this is the
/// *only* guard in this area that a modelless CI can run, so it holds the constraint the code
/// actually depends on, with room to spare.
pub const MIN_DRIFT_HEADROOM_TOKENS: usize = 16;

// The relationships between the constants above are **compile-time** invariants (M5-R2, M5-R10),
// not runtime tests — get one wrong and the crate must not build at all.
//
// 1. The planning window must leave at least MIN_DRIFT_HEADROOM_TOKENS below the model ceiling, or
//    re-tokenization drift overflows it and the PERF-01 `Expand` error comes straight back. The
//    subtraction is const-evaluated, so a window *over* the ceiling underflows and is a compile
//    error too — this subsumes the weaker `MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS` rather than
//    sitting beside it.
// 2. A window must advance by at least one token, or chunking would not terminate.
const _: () = assert!(
    MODEL_MAX_TOKENS - MAX_WINDOW_TOKENS >= MIN_DRIFT_HEADROOM_TOKENS,
    "the planning window must leave room for re-tokenization drift (specials + cut-edge drift)"
);
const _: () = assert!(
    CHUNK_OVERLAP_TOKENS < MAX_WINDOW_TOKENS,
    "a chunk window must advance, or chunking would not terminate"
);

/// NER-based detector backed by an ONNX Runtime session.
///
/// Holds a small **pool** of sessions so inference isn't a single-threaded
/// bottleneck under concurrent load (the sync [`PiiDetector::detect`] is called
/// from many request tasks). Sessions are checked out round-robin.
pub struct OnnxNerDetector {
    sessions: Vec<Mutex<Session>>,
    tokenizer: Tokenizer,
    /// id → label string (e.g. `["O", "B-PER", "I-PER", …]`), from the model's
    /// config. Passed in so this stays model-agnostic.
    id2label: Vec<String>,
    /// Whether the model expects a `token_type_ids` input (BERT-family).
    needs_token_type_ids: bool,
    next: AtomicUsize,
}

impl OnnxNerDetector {
    /// Load the model + tokenizer from disk and build a CPU session pool.
    ///
    /// `id2label` is the model's label list (index = class id); `pool_size` is
    /// clamped to at least 1; `needs_token_type_ids` threads a zero
    /// `token_type_ids` input for BERT-family models.
    pub fn load(
        model_path: &str,
        tokenizer_path: &str,
        id2label: Vec<String>,
        pool_size: usize,
        needs_token_type_ids: bool,
    ) -> Result<Self> {
        let tokenizer =
            Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!("load tokenizer: {e}"))?;

        // ort's builder errors aren't `Send + Sync` and each step carries a
        // different error param, so convert to a string at every step rather
        // than chaining/propagating them into `anyhow` directly.
        let pool_size = pool_size.max(1);
        let mut sessions = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let builder = Session::builder().map_err(|e| anyhow!("session builder: {e}"))?;
            let builder = builder
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow!("optimization level: {e}"))?;
            // `commit_from_file` takes `&mut self`, so this binding must be mut.
            let mut builder = builder
                .with_intra_threads(1)
                .map_err(|e| anyhow!("intra threads: {e}"))?;
            let session = builder
                .commit_from_file(model_path)
                .map_err(|e| anyhow!("load model {model_path}: {e}"))?;
            sessions.push(Mutex::new(session));
        }

        Ok(Self {
            sessions,
            tokenizer,
            id2label,
            needs_token_type_ids,
            next: AtomicUsize::new(0),
        })
    }

    /// Tokenize `input`; dispatch to one model call when it fits the model's
    /// sequence budget, or to [`infer_chunked`](Self::infer_chunked) when it doesn't.
    fn infer(&self, input: &str) -> Result<Vec<PiiEntity>> {
        // Drop the tokenizer's error detail: it can echo the input text, and this
        // is a "never log raw PII" tool (M2-R8).
        let encoding = self
            .tokenizer
            .encode(input, true)
            .map_err(|_| anyhow!("tokenizer error"))?;

        if encoding.get_ids().len() <= MAX_WINDOW_TOKENS {
            return self.run_and_decode(input, &encoding);
        }
        self.infer_chunked(input, &encoding)
    }

    /// Split `input` into overlapping windows (by byte offset, derived from
    /// `full_encoding`'s per-token offsets) each within [`MAX_WINDOW_TOKENS`] tokens, run each
    /// independently, and merge the results.
    ///
    /// Each window is **re-tokenized from its own text**, not sliced out of
    /// `full_encoding`'s token ids: a middle chunk needs its own `<s>` / `</s>`
    /// framing, which a raw token-id slice would lack. Windows overlap by
    /// [`CHUNK_OVERLAP_TOKENS`], so an entity landing on what would otherwise be
    /// a chunk boundary is still whole in a neighboring window; an exact
    /// duplicate entity from the overlap is deduped by (kind, span, text).
    ///
    /// A window that cuts an entity in half still emits the *truncated* fragment (`Mil` where the
    /// neighbouring window sees `Milan`) — a different span, so `dedup` does **not** remove it.
    /// [`resolve_overlaps`](super::overlap::resolve_overlaps) does: its NER phase tiebreaks on
    /// span length descending, takes the whole entity, and drops the fragment as overlapping. See
    /// `ARCHITECTURE.md` → *NER chunking* — that tiebreak is load-bearing here.
    ///
    /// This is a **recall** mechanism, never a leak-relevant one: structured PII
    /// (the fail-closed layer) is detected independently, over the whole field,
    /// and is never chunked. An entity that falls in the small sliver right at a
    /// window edge without enough overlap to be whole in either window is a
    /// missed name/org/location — the same class of gap `OVL-02`/M2-R7 already
    /// document as accepted for the best-effort NER layer.
    fn infer_chunked(&self, input: &str, full_encoding: &Encoding) -> Result<Vec<PiiEntity>> {
        let ranges = chunk_char_ranges(
            input,
            full_encoding.get_offsets(),
            MAX_WINDOW_TOKENS,
            CHUNK_OVERLAP_TOKENS,
        );

        let mut entities = Vec::new();
        for (char_start, char_end) in ranges {
            // `chunk_char_ranges` widens every range to `char` boundaries, so this is total by
            // construction. We still don't *index* — this is a proxy on attacker-influenced input,
            // and the rest of the masking path (`decode_entities`, `Vault::mask`,
            // `overlap::materialize`) all refuse to panic on a tokenizer-derived range for exactly
            // this reason (M2-R6, M4-R14). Skipping a window costs recall on it; panicking costs
            // the request (M5-R3).
            let Some(chunk) = input.get(char_start..char_end) else {
                debug_assert!(false, "chunk_char_ranges must return sliceable ranges");
                tracing::warn!("NER chunk window fell off a char boundary; skipping the window");
                continue;
            };
            let chunk_encoding = self
                .tokenizer
                .encode(chunk, true)
                .map_err(|_| anyhow!("tokenizer error"))?;
            for mut entity in self.run_and_decode(chunk, &chunk_encoding)? {
                entity.span = (entity.span.start + char_start)..(entity.span.end + char_start);
                entities.push(entity);
            }
        }

        entities.sort_by(|a, b| {
            a.span
                .start
                .cmp(&b.span.start)
                .then(a.span.end.cmp(&b.span.end))
        });
        entities.dedup();
        Ok(entities)
    }

    /// Run the model on an already-tokenized `(input, encoding)` pair and decode
    /// its logits into entity spans. `input` and `encoding` must correspond to
    /// the *same* text — this is the one-shot path shared by the direct call and
    /// each chunk of [`infer_chunked`](Self::infer_chunked).
    ///
    /// **This is where [`MODEL_MAX_TOKENS`] is enforced (M5-R2)**, because it is the single choke
    /// point every path into the session goes through — the direct call *and* every re-tokenized
    /// chunk. A sequence longer than the model's usable length is **rejected**, never forwarded:
    /// past it the ONNX graph fails outright (the PERF-01 `Expand` error), so this turns a cryptic
    /// tensor-shape failure into a named one, *before* the session ever sees it.
    ///
    /// **It returns `Err` — it does NOT clamp and continue (M5-R7).** The first version of this
    /// fix truncated the sequence and returned `Ok(partial)`, reasoning that losing a window's tail
    /// beats losing the whole field. That reasoning is fine; **making the call here is not.**
    /// Whether a degraded NER is acceptable is a *posture* decision, and this codebase already has
    /// exactly one place that owns it: the [`FailOpen`](super::composite::FailOpen) wrapper and the
    /// [`try_detect`](PiiDetector::try_detect) error channel (M2-R1/R2). Under the default
    /// (fail-open) posture `FailOpen` swallows this error and the request proceeds structured-only;
    /// under **`NER_REQUIRED`** the detector is *unwrapped*, so the error propagates and the
    /// request is **blocked (400)** — which is precisely what that operator asked for. Clamping
    /// silently returned `Ok` in **both** postures, quietly forwarding a partially-scanned field to
    /// someone who had explicitly demanded a block. That is not a smaller failure; it is the
    /// failure *relocated* — the exact move the M4 retrospective is about.
    ///
    /// This path is **latent by design**: [`MAX_WINDOW_TOKENS`] leaves 32 tokens of headroom for
    /// re-tokenization drift (measured +1…+3 over 17 adversarial scripts), and
    /// `tests/ner_perf.rs::m5_r2_every_retokenized_window_stays_within_the_models_usable_length`
    /// is the guard that keeps it that way. It is the *fail-safe*, not the mechanism.
    fn run_and_decode(&self, input: &str, encoding: &Encoding) -> Result<Vec<PiiEntity>> {
        let seq = encoding.get_ids().len();
        if seq == 0 {
            return Ok(Vec::new());
        }
        if seq > MODEL_MAX_TOKENS {
            // Counts only, never the text (the never-log-raw-PII rule).
            return Err(anyhow!(
                "NER sequence is {seq} tokens, over the model's usable length of \
                 {MODEL_MAX_TOKENS}; refusing to run it (the posture — fail open to \
                 structured-only, or block under NER_REQUIRED — is decided by the caller)"
            ));
        }
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&i| i as i64).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&i| i as i64)
            .collect();
        let offsets = encoding.get_offsets();

        let input_ids =
            Tensor::from_array(([1, seq], ids)).map_err(|e| anyhow!("input_ids tensor: {e}"))?;
        let attention_mask = Tensor::from_array(([1, seq], mask))
            .map_err(|e| anyhow!("attention_mask tensor: {e}"))?;

        // Round-robin a session; recover a poisoned lock rather than permanently
        // disabling a pool slot (M2-R9).
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        let mut session = self.sessions[idx]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let outputs = if self.needs_token_type_ids {
            let type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&i| i as i64).collect();
            let token_type_ids = Tensor::from_array(([1, seq], type_ids))
                .map_err(|e| anyhow!("token_type_ids tensor: {e}"))?;
            session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                    "token_type_ids" => token_type_ids,
                ])
                .map_err(|e| anyhow!("ONNX run: {e}"))?
        } else {
            session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                ])
                .map_err(|e| anyhow!("ONNX run: {e}"))?
        };

        // logits: [1, seq, num_labels], row-major. Look the output up by name —
        // never panic on a differently-shaped model (M2-R4).
        let logits_value = outputs
            .get("logits")
            .ok_or_else(|| anyhow!("model has no `logits` output"))?;
        let (_shape, logits) = logits_value
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("extract logits: {e}"))?;

        let num_labels = logits.len() / seq;
        if num_labels == 0 {
            return Ok(Vec::new());
        }
        // A mismatched label list would silently drop entities (M2-R3).
        validate_label_count(self.id2label.len(), num_labels).map_err(|m| anyhow!(m))?;

        let mut tags: Vec<TokenTag> = Vec::with_capacity(seq);
        for (token, &(start, end)) in offsets.iter().enumerate().take(seq) {
            let row = &logits[token * num_labels..(token + 1) * num_labels];
            let best = argmax(row);
            let label = self.id2label.get(best).map(String::as_str).unwrap_or("O");
            tags.push(TokenTag { label, start, end });
        }

        Ok(decode_entities(input, &tags))
    }
}

/// Index of the largest value in `row` (0 if empty / all-NaN).
fn argmax(row: &[f32]) -> usize {
    row.iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

impl PiiDetector for OnnxNerDetector {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        // Infallible view: fail open (the caller decides whether to require it).
        self.try_detect(input).unwrap_or_default()
    }

    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        self.infer(input).map_err(|err| DetectError {
            detector: "onnx-ner",
            // `infer` never embeds input text in its errors (see M2-R8).
            message: err.to_string(),
        })
    }
}

/// Compute the byte ranges of the overlapping windows that cover a tokenized `input`, from its
/// **full** per-token `offsets` (as produced by tokenizing the whole text once).
///
/// Pure and model-independent — no tokenizer or session needed — so, unlike the
/// rest of this module, it is unit-tested without a real ONNX model.
///
/// **Every returned range is guaranteed sliceable** — in-bounds and on `char` boundaries — because
/// each is widened through [`widen_to_char_boundaries`] before being emitted (M5-R3). The offsets
/// come from the *tokenizer*, so treating them as `char`-aligned is an assumption about a
/// third-party library's output on attacker-influenced input, and this module already refuses to
/// make that assumption anywhere else (`decode_entities` guards the identical class of offset —
/// that *was* M2-R6). Widening only ever *adds* bytes, so it cannot shrink a window's coverage.
///
/// **The one thing this exists to get right:** the token at `offsets.len() - 1`
/// is always the closing `</s>` added by `encode(_, true)`, whose offset is the
/// sentinel `(0, 0)` — not the real text end. A window reaching the sequence
/// end must use `input.len()` for its char end, not that sentinel (measured,
/// `tests/ner_perf.rs`: using the sentinel silently dropped the final window,
/// losing a third of the entities on a large field — an outright bug, not a
/// recall nuance, caught only by testing at a size that exercised the last
/// window's boundary).
///
/// `pub` so the live guard in `tests/ner_perf.rs` (M5-R2) can drive the **real** chunker with a
/// real tokenizer and assert each re-tokenized window lands under [`MODEL_MAX_TOKENS`]. A copy of
/// this logic in the test would drift from the code it is supposed to guard.
pub fn chunk_char_ranges(
    input: &str,
    offsets: &[(usize, usize)],
    window: usize,
    overlap: usize,
) -> Vec<(usize, usize)> {
    let seq = offsets.len();
    let stride = window.saturating_sub(overlap).max(1);

    let mut ranges = Vec::new();
    let mut token_start = 0usize;
    loop {
        let token_end = (token_start + window).min(seq);
        let char_start = offsets[token_start].0;
        let char_end = if token_end == seq {
            input.len()
        } else {
            offsets[token_end - 1].1.max(char_start)
        };
        let widened = widen_to_char_boundaries(input, char_start..char_end);
        ranges.push((widened.start, widened.end));

        if token_end == seq {
            break;
        }
        token_start += stride;
    }
    ranges
}

#[cfg(test)]
mod chunk_tests {
    use super::chunk_char_ranges;

    /// Build a fake offsets table shaped like a real `encode(_, true)` output:
    /// a leading `(0, 0)` for `<s>`, one `(i, i+1)` per content token, and a
    /// trailing `(0, 0)` for `</s>` — the exact shape that hid the M5 chunking
    /// bug (the closing special token's sentinel offset, mistaken for the real
    /// text end).
    fn fake_offsets(content_tokens: usize) -> Vec<(usize, usize)> {
        let mut offsets = vec![(0, 0)]; // <s>
        offsets.extend((0..content_tokens).map(|i| (i, i + 1)));
        offsets.push((0, 0)); // </s>
        offsets
    }

    /// An all-ASCII input of `n` bytes, so the `(i, i+1)` offsets of
    /// [`fake_offsets`] line up one-to-one with its bytes.
    fn ascii(n: usize) -> String {
        "a".repeat(n)
    }

    #[test]
    fn a_single_window_covers_the_whole_input_when_it_fits() {
        let input = ascii(10);
        let offsets = fake_offsets(10); // seq = 12 (incl. <s>/</s>)
        let ranges = chunk_char_ranges(&input, &offsets, 480, 32);
        assert_eq!(ranges, vec![(0, 10)]);
    }

    #[test]
    fn the_last_window_reaches_the_true_text_end_not_the_closing_token_sentinel() {
        // seq = 22 (<s> + 20 content + </s>); window=10, overlap=2 → stride=8.
        // Windows (token space): [0,10) [8,18) [16,22). The last one's final
        // token index is 21 = seq-1, the `</s>` sentinel — this is exactly the
        // case that must NOT collapse to a zero-length range.
        let input = ascii(20);
        let offsets = fake_offsets(20);
        let ranges = chunk_char_ranges(&input, &offsets, 10, 2);

        let last = *ranges.last().unwrap();
        assert_eq!(
            last.1, 20,
            "must reach the real text end, not the sentinel (0,0)"
        );
        assert!(last.1 > last.0, "the last window must not be empty");
    }

    #[test]
    fn windows_overlap_and_jointly_cover_every_char() {
        let input = ascii(50);
        let offsets = fake_offsets(50);
        let ranges = chunk_char_ranges(&input, &offsets, 10, 2);

        assert!(
            ranges.len() > 1,
            "50 content tokens must need more than one window"
        );
        assert_eq!(ranges.first().unwrap().0, 0, "coverage must start at 0");
        assert_eq!(
            ranges.last().unwrap().1,
            50,
            "coverage must reach the true end"
        );
        // Consecutive windows must overlap (never leave a gap).
        for pair in ranges.windows(2) {
            let (_, prev_end) = pair[0];
            let (next_start, _) = pair[1];
            assert!(
                next_start < prev_end,
                "windows {:?} -> {:?} leave a gap",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn every_window_is_sliceable_even_when_an_offset_lands_inside_a_multibyte_char() {
        // M5-R3. The window edges come from the **tokenizer**; treating them as `char`-aligned is
        // an assumption about a third-party library's output on attacker-influenced input, and
        // `&input[start..end]` *panics* if it's wrong — the one place on the masking path that
        // could, while `decode_entities` (M2-R6), `Vault::mask` and `overlap::materialize` all
        // refuse to. So feed offsets that deliberately cut a 3-byte `€` and a 4-byte `𝄞` in half
        // and assert the ranges come back sliceable anyway (widened, never narrowed).
        let input = "a€b𝄞c€d"; // 1 + 3 + 1 + 4 + 1 + 3 + 1 = 14 bytes
        assert_eq!(input.len(), 14);

        // Offsets that land *inside* the multi-byte chars (2 is mid-`€`, 6/7 mid-`𝄞`, 11 mid-`€`).
        let offsets = vec![(0, 0), (0, 2), (2, 6), (6, 7), (7, 11), (11, 13), (0, 0)];

        // Non-vacuity: the table really does carry the hazard. Without this the test could pass
        // on offsets that happen to be `char`-aligned and prove nothing — the exact way a guard
        // goes quietly blind (the M4-R13/M4-R24 lesson: ask what the corpus holds *constant*).
        assert!(
            offsets
                .iter()
                .any(|&(s, e)| !input.is_char_boundary(s)
                    || !input.is_char_boundary(e.min(input.len()))),
            "the offsets table must actually cut a multi-byte char, or this guard is vacuous"
        );

        let ranges = chunk_char_ranges(input, &offsets, 3, 1);

        for (start, end) in ranges {
            assert!(
                input.get(start..end).is_some(),
                "window {start}..{end} is not sliceable — it would panic in infer_chunked"
            );
        }
    }

    #[test]
    fn a_window_at_least_as_wide_as_the_sequence_produces_one_range() {
        let input = ascii(5);
        let offsets = fake_offsets(5);
        let ranges = chunk_char_ranges(&input, &offsets, 480, 32);
        assert_eq!(ranges, vec![(0, 5)]);
    }
}

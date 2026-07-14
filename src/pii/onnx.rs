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
//! **Chunking (M5, PERF-01).** A field longer than [`MAX_SEQUENCE_TOKENS`] is
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
use super::{DetectError, PiiDetector, PiiEntity};

/// Upper bound on tokens (special tokens included) fed to the model in one call.
/// Conservatively under the picked model's `max_position_embeddings` (514), so it
/// holds for other RoBERTa-family token-classification models too without needing
/// the model to report its own limit.
const MAX_SEQUENCE_TOKENS: usize = 480;

/// Overlap (in tokens) between consecutive chunks, so an entity that would
/// otherwise land right at a chunk boundary is still whole in at least one
/// chunk. Generous relative to a Person/Org/Location span (rarely more than a
/// handful of tokens).
const CHUNK_OVERLAP_TOKENS: usize = 32;

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

        if encoding.get_ids().len() <= MAX_SEQUENCE_TOKENS {
            return self.run_and_decode(input, &encoding);
        }
        self.infer_chunked(input, &encoding)
    }

    /// Split `input` into overlapping windows (by **character** offset, derived
    /// from `full_encoding`'s per-token offsets — themselves always on `char`
    /// boundaries) each within [`MAX_SEQUENCE_TOKENS`] tokens, run each
    /// independently, and merge the results.
    ///
    /// Each window is **re-tokenized from its own text**, not sliced out of
    /// `full_encoding`'s token ids: a middle chunk needs its own `<s>` / `</s>`
    /// framing, which a raw token-id slice would lack. Windows overlap by
    /// [`CHUNK_OVERLAP_TOKENS`], so an entity landing on what would otherwise be
    /// a chunk boundary is still whole in a neighboring window; an exact
    /// duplicate entity from the overlap is deduped by (kind, span, text).
    ///
    /// This is a **recall** mechanism, never a leak-relevant one: structured PII
    /// (the fail-closed layer) is detected independently, over the whole field,
    /// and is never chunked. An entity that falls in the small sliver right at a
    /// window edge without enough overlap to be whole in either window is a
    /// missed name/org/location — the same class of gap `OVL-02`/M2-R7 already
    /// document as accepted for the best-effort NER layer.
    fn infer_chunked(&self, input: &str, full_encoding: &Encoding) -> Result<Vec<PiiEntity>> {
        let ranges = chunk_char_ranges(
            full_encoding.get_offsets(),
            input.len(),
            MAX_SEQUENCE_TOKENS,
            CHUNK_OVERLAP_TOKENS,
        );

        let mut entities = Vec::new();
        for (char_start, char_end) in ranges {
            let chunk = &input[char_start..char_end];
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
    fn run_and_decode(&self, input: &str, encoding: &Encoding) -> Result<Vec<PiiEntity>> {
        let seq = encoding.get_ids().len();
        if seq == 0 {
            return Ok(Vec::new());
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

/// Compute the `(char_start, char_end)` ranges of the overlapping windows that
/// cover a tokenized input, from its **full** per-token char `offsets` (as
/// produced by tokenizing the whole text once) plus the input's byte length.
///
/// Pure and model-independent — no tokenizer or session needed — so, unlike the
/// rest of this module, it is unit-tested without a real ONNX model.
///
/// **The one thing this exists to get right:** the token at `offsets.len() - 1`
/// is always the closing `</s>` added by `encode(_, true)`, whose offset is the
/// sentinel `(0, 0)` — not the real text end. A window reaching the sequence
/// end must use `input_len` for its char end, not that sentinel (measured,
/// `tests/ner_perf.rs`: using the sentinel silently dropped the final window,
/// losing a third of the entities on a large field — an outright bug, not a
/// recall nuance, caught only by testing at a size that exercised the last
/// window's boundary).
fn chunk_char_ranges(
    offsets: &[(usize, usize)],
    input_len: usize,
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
            input_len
        } else {
            offsets[token_end - 1].1.max(char_start)
        };
        ranges.push((char_start, char_end));

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

    #[test]
    fn a_single_window_covers_the_whole_input_when_it_fits() {
        let offsets = fake_offsets(10); // seq = 12 (incl. <s>/</s>)
        let ranges = chunk_char_ranges(&offsets, 10, 480, 32);
        assert_eq!(ranges, vec![(0, 10)]);
    }

    #[test]
    fn the_last_window_reaches_the_true_text_end_not_the_closing_token_sentinel() {
        // seq = 22 (<s> + 20 content + </s>); window=10, overlap=2 → stride=8.
        // Windows (token space): [0,10) [8,18) [16,22). The last one's final
        // token index is 21 = seq-1, the `</s>` sentinel — this is exactly the
        // case that must NOT collapse to a zero-length range.
        let offsets = fake_offsets(20);
        let ranges = chunk_char_ranges(&offsets, 20, 10, 2);

        let last = *ranges.last().unwrap();
        assert_eq!(
            last.1, 20,
            "must reach the real text end, not the sentinel (0,0)"
        );
        assert!(last.1 > last.0, "the last window must not be empty");
    }

    #[test]
    fn windows_overlap_and_jointly_cover_every_char() {
        let offsets = fake_offsets(50);
        let ranges = chunk_char_ranges(&offsets, 50, 10, 2);

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
    fn a_window_at_least_as_wide_as_the_sequence_produces_one_range() {
        let offsets = fake_offsets(5);
        let ranges = chunk_char_ranges(&offsets, 5, 480, 32);
        assert_eq!(ranges, vec![(0, 5)]);
    }
}

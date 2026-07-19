//! GLiNER span detector for **contextual / open-label** PII — milestone M8.
//! Enabled by the `onnx` feature.
//!
//! GLiNER is a *zero-shot, open-label span extractor*, **not** token-classification
//! (that is [`onnx::OnnxNerDetector`](super::onnx)). The entity types are fed to the
//! model **as text** (`"person"`, `"phone number"`, `"address"`, …); it scores every
//! candidate **word-span × type** pair, and detection is a sigmoid threshold + greedy
//! non-overlapping selection ([`gliner_decode`](super::gliner_decode)). It is the path
//! for PII the deterministic layer can't anchor and the XLM-R NER doesn't cover — a bare
//! national phone, a free-form address — see `docs/ROADMAP.md` M8.
//!
//! **The ONNX I/O contract (span_mode `markerV0`, from `gliner_config.json`).** Inputs:
//! `input_ids` + `attention_mask` + `words_mask` (i64 `[1, L]`), `text_lengths`
//! (i64 `[1, 1]`), `span_idx` (i64 `[1, S, 2]`, inclusive start/end **word** indices),
//! `span_mask` (bool `[1, S]`). Output `logits` (f32 `[1, S, T]`, `T` = number of entity
//! types). `S = num_words * max_width`, laid out word-major / width-minor. This contract
//! is pinned to `onnx-community/gliner_multi_pii-v1` and **verified against the real
//! export** by the gated smoke test (`tests/gliner_eval.rs`, `SMOKE-GLINER`) — the S0
//! step: this module is built to the documented contract, the smoke test is what proves
//! the tensors are right on the real model.
//!
//! **The prompt shares the sequence budget.** The type labels are prepended to *every*
//! window (`<<ENT>> type … <<SEP>> word …`), so the usable text budget is
//! `max_len − prompt − specials`, not the whole sequence — [`plan_word_windows`] does that
//! arithmetic (S3). Fail-closed posture is the caller's: an inference error flows through
//! [`try_detect`](PiiDetector::try_detect) exactly like the NER's (M5-R7).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use super::gliner_decode::{decode_spans, split_words, GlinerLabel, SpanScore};
use super::{DetectError, PiiDetector, PiiEntity};

/// Default detection threshold on the per-span sigmoid probability — **0.15, chosen by
/// measurement** (DEVLOG M8, the threshold sweep), well below GLiNER's nominal 0.5 because
/// the **int8-quantized** model's confidences run low: correct entities cluster at
/// 0.15–0.6. On the corpus this is the sweet spot — Location recall 0.909 (matching XLM-R)
/// at precision 1.0, before a lower threshold starts costing Person precision. It suits a
/// privacy tool where **recall beats precision** (an over-mask is never a leak).
/// Overridable via `GLINER_THRESHOLD`. A model swap (esp. fp32) re-opens this number.
pub const DEFAULT_THRESHOLD: f32 = 0.15;

/// Model shape parameters read from `gliner_config.json`. Kept explicit (not hard-coded)
/// so a model swap is a config change, not a code change.
#[derive(Debug, Clone)]
pub struct GlinerParams {
    /// Max sequence length the encoder accepts (mDeBERTa-v3: 384). The whole
    /// `prompt + text` token sequence must fit under this.
    pub max_len: usize,
    /// Max span width in **words** the model scores (default 12).
    pub max_width: usize,
    /// The entity-marker token that precedes each type in the prompt (`<<ENT>>`).
    pub ent_token: String,
    /// The separator token between the type list and the text (`<<SEP>>`).
    pub sep_token: String,
}

impl GlinerParams {
    /// Parse the shape parameters from a GLiNER `gliner_config.json`. Missing fields fall
    /// back to the published `gliner_multi-v2.1` defaults, so a terse config still loads.
    pub fn from_config_json(json: &str) -> Result<Self> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| anyhow!("gliner_config.json: {e}"))?;
        let as_usize = |key: &str, default: usize| -> usize {
            v.get(key)
                .and_then(|x| x.as_u64())
                .map(|n| n as usize)
                .unwrap_or(default)
        };
        let as_str = |key: &str, default: &str| -> String {
            v.get(key)
                .and_then(|x| x.as_str())
                .unwrap_or(default)
                .to_string()
        };
        Ok(Self {
            max_len: as_usize("max_len", 384),
            max_width: as_usize("max_width", 12),
            ent_token: as_str("ent_token", "<<ENT>>"),
            sep_token: as_str("sep_token", "<<SEP>>"),
        })
    }
}

/// GLiNER detector backed by an ONNX Runtime session pool (mirrors
/// [`OnnxNerDetector`](super::onnx::OnnxNerDetector)'s concurrency model).
pub struct GLiNerDetector {
    sessions: Vec<Mutex<Session>>,
    tokenizer: Tokenizer,
    labels: Vec<GlinerLabel>,
    params: GlinerParams,
    threshold: f32,
    next: AtomicUsize,
}

impl GLiNerDetector {
    /// Load model + tokenizer and build a CPU session pool. `labels` is the typed
    /// open-label set (from `GLINER_LABELS` or [`default_gliner_labels`](super::gliner_decode::default_gliner_labels)).
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        model_path: &str,
        tokenizer_path: &str,
        labels: Vec<GlinerLabel>,
        params: GlinerParams,
        threshold: f32,
        pool_size: usize,
        intra_threads: usize,
    ) -> Result<Self> {
        anyhow::ensure!(!labels.is_empty(), "GLiNER needs at least one entity label");
        let tokenizer =
            Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!("load tokenizer: {e}"))?;

        let pool_size = pool_size.max(1);
        let intra_threads = intra_threads.max(1);
        let mut sessions = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let builder = Session::builder().map_err(|e| anyhow!("session builder: {e}"))?;
            let builder = builder
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow!("optimization level: {e}"))?;
            let mut builder = builder
                .with_intra_threads(intra_threads)
                .map_err(|e| anyhow!("intra threads: {e}"))?;
            let session = builder
                .commit_from_file(model_path)
                .map_err(|e| anyhow!("load model {model_path}: {e}"))?;
            sessions.push(Mutex::new(session));
        }

        Ok(Self {
            sessions,
            tokenizer,
            labels,
            params,
            threshold,
            next: AtomicUsize::new(0),
        })
    }

    /// The prompt token count: `<<ENT>> type` per label + one `<<SEP>>`, tokenized. This
    /// is what every text window must leave room for under [`GlinerParams::max_len`].
    fn prompt_token_len(&self) -> Result<usize> {
        let prompt = self.build_prompt_words();
        // Encode WITHOUT special tokens: we only want the prompt's own length here.
        let enc = self
            .tokenizer
            .encode(prompt, false)
            .map_err(|_| anyhow!("tokenizer error"))?;
        Ok(enc.get_ids().len())
    }

    /// The pre-split prompt "words": `[<<ENT>>, type_1, <<ENT>>, type_2, …, <<SEP>>]`.
    /// Each is one element so `is_split_into_words` tokenization tracks word ids cleanly.
    fn build_prompt_words(&self) -> Vec<String> {
        let mut words = Vec::with_capacity(self.labels.len() * 2 + 1);
        for label in &self.labels {
            words.push(self.params.ent_token.clone());
            words.push(label.text.clone());
        }
        words.push(self.params.sep_token.clone());
        words
    }

    /// Detect entities over the whole `text`, chunking into word-windows that each fit the
    /// shared budget ([`plan_word_windows`]). Spans are mapped back to the full text's
    /// bytes and de-duplicated across overlapping windows by the resolver upstream.
    fn infer(&self, text: &str) -> Result<Vec<PiiEntity>> {
        let words = split_words(text);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        // The model's own ceiling: max_len − prompt − the tokenizer's special tokens (mDeBERTa
        // adds [CLS]/[SEP] = 2). Guards against an underflow if the label set is so large the
        // prompt alone fills the model.
        let prompt_len = self.prompt_token_len()?;
        let overhead = prompt_len + SPECIAL_TOKENS;
        let max_from_model = self
            .params
            .max_len
            .checked_sub(overhead)
            .filter(|b| *b >= MIN_TEXT_TOKEN_BUDGET)
            .ok_or_else(|| {
                anyhow!(
                    "GLiNER label set is too large: prompt+specials ({overhead}) leaves < \
                     {MIN_TEXT_TOKEN_BUDGET} tokens for text under max_len {}",
                    self.params.max_len
                )
            })?;
        // **Cap the window well below that ceiling.** GLiNER int8's per-span confidence *dilutes
        // with the window's total context* (measured — a clear name keeps ≳0.2 while its window
        // stays ≲100 text tokens, ~0.15 by ~130; at `max_len` the model returns all-low logits,
        // unusable — DEVLOG M8), and this is position-*independent*: a small window scores an entity
        // at any offset. So a small window is not a latency trade but a **recall** one — it bounds
        // the context every span is scored against. A model swap (esp. fp32) re-opens this number.
        let text_budget = max_from_model.min(MAX_WINDOW_TEXT_TOKENS);

        let windows = plan_word_windows(
            &self.word_token_lens(text, &words)?,
            text_budget,
            WINDOW_OVERLAP_WORDS,
        );
        let mut entities = Vec::new();
        for (w_start, w_end) in windows {
            entities.extend(self.infer_window(text, &words[w_start..w_end])?);
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

    /// Per-word token counts (first-subtoken pooling means we only need the count to plan
    /// windows). Encodes each word once, without special tokens.
    fn word_token_lens(&self, text: &str, words: &[(usize, usize)]) -> Result<Vec<usize>> {
        let mut lens = Vec::with_capacity(words.len());
        for &(s, e) in words {
            let Some(w) = text.get(s..e) else {
                lens.push(1);
                continue;
            };
            let enc = self
                .tokenizer
                .encode(w, false)
                .map_err(|_| anyhow!("tokenizer error"))?;
            lens.push(enc.get_ids().len().max(1));
        }
        Ok(lens)
    }

    /// Run one window of text words through the model and decode its spans. The window's
    /// word spans index the full `text`, so the resulting byte spans are already absolute.
    fn infer_window(&self, text: &str, window_words: &[(usize, usize)]) -> Result<Vec<PiiEntity>> {
        if window_words.is_empty() {
            return Ok(Vec::new());
        }
        let num_words = window_words.len();

        // Build the pre-split input: prompt elements + this window's word strings.
        let mut pieces = self.build_prompt_words();
        let num_prompt = pieces.len();
        for &(s, e) in window_words {
            pieces.push(text.get(s..e).unwrap_or("").to_string());
        }

        let enc = self
            .tokenizer
            .encode(pieces, true)
            .map_err(|_| anyhow!("tokenizer error"))?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&i| i as i64).collect();
        let attn: Vec<i64> = enc.get_attention_mask().iter().map(|&i| i as i64).collect();
        let seq = ids.len();

        // **The single choke point — enforce `max_len` here, before the session (M8-R1, the
        // M5-R7 discipline).** [`infer`] plans windows in the per-word-alone token count, but this
        // re-tokenizes the window **jointly** with the prompt, which can drift at the cut edges. A
        // planned bound is not a checked one: if the joint `seq` ever exceeds the model's usable
        // length the ONNX graph fails outright (the M5 `Expand`-overflow class). Reject it as an
        // `Err` — value-free, counts only — and let the **caller's** posture decide (fail open to
        // structured-only, or block under `NER_REQUIRED`); a detector never decides that itself.
        if seq > self.params.max_len {
            return Err(anyhow!(
                "GLiNER sequence is {seq} tokens, over the model's max_len of {}; refusing to run it \
                 (the posture — fail open, or block under NER_REQUIRED — is the caller's)",
                self.params.max_len
            ));
        }

        // words_mask: 1-based index of the TEXT word a token starts, else 0. Only the
        // first subtoken of each text word is marked (subtoken_pooling = "first").
        let word_ids = enc.get_word_ids();
        let mut words_mask = vec![0i64; seq];
        let mut prev: Option<u32> = None;
        for (t, wid) in word_ids.iter().enumerate() {
            if let Some(w) = wid {
                let is_start = prev != Some(*w);
                prev = Some(*w);
                let w = *w as usize;
                if is_start && w >= num_prompt {
                    words_mask[t] = (w - num_prompt + 1) as i64;
                }
            } else {
                prev = None;
            }
        }

        // Span enumeration (markerV0): every (start, width) over the window's words.
        let max_width = self.params.max_width.max(1);
        let num_spans = num_words * max_width;
        let mut span_idx = Vec::with_capacity(num_spans * 2);
        let mut span_mask = Vec::with_capacity(num_spans);
        for start in 0..num_words {
            for width in 0..max_width {
                let end = start + width;
                let valid = end < num_words;
                span_idx.push(start as i64);
                span_idx.push(if valid { end as i64 } else { start as i64 });
                span_mask.push(valid);
            }
        }

        let input_ids =
            Tensor::from_array(([1, seq], ids)).map_err(|e| anyhow!("input_ids: {e}"))?;
        let attention_mask =
            Tensor::from_array(([1, seq], attn)).map_err(|e| anyhow!("attention_mask: {e}"))?;
        let words_mask_t =
            Tensor::from_array(([1, seq], words_mask)).map_err(|e| anyhow!("words_mask: {e}"))?;
        let text_lengths = Tensor::from_array(([1, 1], vec![num_words as i64]))
            .map_err(|e| anyhow!("text_lengths: {e}"))?;
        let span_idx_t = Tensor::from_array(([1, num_spans, 2], span_idx))
            .map_err(|e| anyhow!("span_idx: {e}"))?;
        let span_mask_t = Tensor::from_array(([1, num_spans], span_mask))
            .map_err(|e| anyhow!("span_mask: {e}"))?;

        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        let mut session = self.sessions[idx]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask,
                "words_mask" => words_mask_t,
                "text_lengths" => text_lengths,
                "span_idx" => span_idx_t,
                "span_mask" => span_mask_t,
            ])
            .map_err(|e| anyhow!("ONNX run: {e}"))?;

        let logits_value = outputs
            .get("logits")
            .ok_or_else(|| anyhow!("model has no `logits` output"))?;
        let (_shape, logits) = logits_value
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("extract logits: {e}"))?;

        let num_types = self.labels.len();
        let expected = num_spans * num_types;
        if logits.len() != expected {
            return Err(anyhow!(
                "GLiNER logits length {} != num_spans*num_types {expected}",
                logits.len()
            ));
        }

        // Read scores for the valid spans only, in the same word-major/width-minor order
        // the span_idx was built, so the flat logits index lines up.
        let mut scores = Vec::new();
        for start in 0..num_words {
            for width in 0..max_width {
                let end = start + width;
                if end >= num_words {
                    continue;
                }
                let span_i = start * max_width + width;
                let base = span_i * num_types;
                for (type_idx, _label) in self.labels.iter().enumerate() {
                    scores.push(SpanScore {
                        start_word: start,
                        end_word: end,
                        type_idx,
                        logit: logits[base + type_idx],
                    });
                }
            }
        }

        // Map window word indices to absolute byte spans via the window's word list.
        Ok(decode_spans(
            text,
            window_words,
            &self.labels,
            &scores,
            self.threshold,
        ))
    }
}

/// The tokenizer's special-token overhead per sequence (mDeBERTa adds `[CLS]` + `[SEP]`).
const SPECIAL_TOKENS: usize = 2;
/// The smallest text-token budget worth running; below this the label set is misconfigured.
const MIN_TEXT_TOKEN_BUDGET: usize = 16;

/// The largest **text**-token window handed to the model, capped **far below** `max_len`.
///
/// GLiNER int8's per-span confidence dilutes with the **total context in the window** — measured
/// (DEVLOG M8): a clear name keeps a score ≳0.2 (above the 0.15 threshold) while its window stays
/// ≲100 text tokens, drifts to ~0.15 by ~130, and at `max_len` (384) the model returns all-low
/// logits (unusable). The dilution is a function of the window's *size*, **not the entity's position
/// in it** — a name is detected at any offset (start, middle, or mid multi-window field) as long as
/// the window is small. So the window is *not* sized to the model's nominal budget: a smaller window
/// is a **recall** choice that bounds the context every span is scored against. (This is a different
/// job from [`WINDOW_OVERLAP_WORDS`], which only guarantees a boundary-crossing entity is *whole* in
/// some window.) Long-field recall is still weaker than short-field — a documented model property,
/// not a bug; the default XLM-R NER covers long system prompts. Tuned to the shipped int8 model; a
/// swap (esp. fp32) re-opens it.
pub const MAX_WINDOW_TEXT_TOKENS: usize = 100;

/// Words of overlap between consecutive windows, so an entity spanning a window boundary is **whole**
/// in at least one window (it is scored across its full span there). Comfortably above a typical
/// multi-word name. Distinct from [`MAX_WINDOW_TEXT_TOKENS`], which is what governs the *score*.
pub const WINDOW_OVERLAP_WORDS: usize = 8;

/// Plan word windows so each window's text tokens fit `budget`, overlapping the previous window by
/// `overlap` words so an entity on a boundary is whole (and near the start) in the next window.
/// `word_token_lens[i]` is word `i`'s subtoken count. Greedy: extend a window until the next word
/// would overflow, then start the next `overlap` words back.
///
/// Pure and model-independent (takes token counts, not a tokenizer) so it is unit-tested without a
/// model — the S3 discipline carried from M5's chunking.
pub fn plan_word_windows(
    word_token_lens: &[usize],
    budget: usize,
    overlap: usize,
) -> Vec<(usize, usize)> {
    let n = word_token_lens.len();
    if n == 0 {
        return Vec::new();
    }
    let budget = budget.max(1);
    let mut windows = Vec::new();
    let mut start = 0usize;
    while start < n {
        let mut used = 0usize;
        let mut end = start;
        while end < n {
            let w = word_token_lens[end].max(1);
            // A single word larger than the budget still goes in its own window (it can't
            // be split further at the word level) rather than looping forever.
            if used + w > budget && end > start {
                break;
            }
            used += w;
            end += 1;
        }
        windows.push((start, end));
        if end >= n {
            break;
        }
        // Step back `overlap` words, but always advance by at least one (else a window whose length
        // is ≤ overlap would loop forever).
        let step_back = overlap.min(end.saturating_sub(start).saturating_sub(1));
        start = end - step_back;
    }
    windows
}

impl PiiDetector for GLiNerDetector {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        self.try_detect(input).unwrap_or_default()
    }

    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        self.infer(input).map_err(|err| DetectError {
            detector: "gliner",
            message: err.to_string(),
        })
    }

    /// Idempotent after the fixpoint's pass 0, exactly like the token-classification NER
    /// (S4): masking a name to `[PERSON_1]` never *reveals* a new name, so re-running the
    /// model on later passes buys no recall — it would only re-tag the fragments it emits.
    /// So the fixpoint converges in O(1) GLiNER passes. The 0-loss recall claim this rests
    /// on is re-measured per model (DEVLOG M8).
    fn redetect(&self, _input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_word_windows, GlinerParams};

    #[test]
    fn params_parse_with_defaults() {
        let p = GlinerParams::from_config_json(
            r#"{"max_len":384,"max_width":12,"ent_token":"<<ENT>>","sep_token":"<<SEP>>"}"#,
        )
        .unwrap();
        assert_eq!(p.max_len, 384);
        assert_eq!(p.max_width, 12);
        assert_eq!(p.ent_token, "<<ENT>>");
        // Missing fields fall back to published defaults.
        let d = GlinerParams::from_config_json("{}").unwrap();
        assert_eq!(d.max_len, 384);
        assert_eq!(d.max_width, 12);
        assert_eq!(d.sep_token, "<<SEP>>");
    }

    #[test]
    fn a_short_text_is_one_window() {
        let lens = vec![1, 2, 1, 3, 1]; // 8 tokens
        assert_eq!(plan_word_windows(&lens, 100, 8), vec![(0, 5)]);
    }

    #[test]
    fn windows_split_on_the_token_budget_and_overlap_by_the_given_words() {
        // Each word = 1 token; budget 5, overlap 2 → 5-word windows stepping back 2.
        let lens = vec![1; 10]; // 10 words
        let w = plan_word_windows(&lens, 5, 2);
        // [0,5) [3,8) [6,10)
        assert_eq!(w, vec![(0, 5), (3, 8), (6, 10)]);
        // Consecutive windows overlap by exactly `overlap` (never leave a gap).
        for pair in w.windows(2) {
            assert!(
                pair[1].0 + 2 == pair[0].1 || pair[1].1 == 10,
                "windows {:?} -> {:?} must overlap by 2",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_single_oversized_word_still_gets_its_own_window() {
        // A 20-token word with budget 5 must not loop forever — it goes in alone (even with a
        // large overlap, the step always advances by ≥1).
        let lens = vec![1, 20, 1];
        let w = plan_word_windows(&lens, 5, 8);
        assert!(!w.is_empty());
        assert_eq!(w.first().unwrap().0, 0);
        assert_eq!(w.last().unwrap().1, 3, "coverage must reach the last word");
    }

    #[test]
    fn empty_input_plans_no_windows() {
        assert!(plan_word_windows(&[], 10, 8).is_empty());
    }
}

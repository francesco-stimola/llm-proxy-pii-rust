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

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, anyhow};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use super::ner_decode::{TokenTag, decode_entities};
use super::{PiiDetector, PiiEntity};

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
    next: AtomicUsize,
}

impl OnnxNerDetector {
    /// Load the model + tokenizer from disk and build a CPU session pool.
    ///
    /// `id2label` is the model's label list (index = class id). `pool_size` is
    /// clamped to at least 1.
    pub fn load(
        model_path: &str,
        tokenizer_path: &str,
        id2label: Vec<String>,
        pool_size: usize,
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
            next: AtomicUsize::new(0),
        })
    }

    /// Tokenize → run → argmax per token → decode into entity spans.
    fn infer(&self, input: &str) -> Result<Vec<PiiEntity>> {
        let encoding = self
            .tokenizer
            .encode(input, true)
            .map_err(|e| anyhow!("tokenize: {e}"))?;

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

        // Round-robin a session out of the pool.
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        let mut session = self.sessions[idx].lock().expect("session mutex poisoned");
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask,
            ])
            .map_err(|e| anyhow!("ONNX run: {e}"))?;

        // logits: [1, seq, num_labels], row-major. Derive num_labels from the
        // flat length + token count (avoids depending on the Shape API).
        let (_shape, logits) = outputs["logits"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("extract logits: {e}"))?;
        let num_labels = logits.len() / seq;
        if num_labels == 0 {
            return Ok(Vec::new());
        }

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
        match self.infer(input) {
            Ok(entities) => entities,
            Err(err) => {
                // A per-request inference error must not crash the request. The
                // structured layer already masked structured PII; log and yield
                // no NER entities. NOTE (fail-closed): if the NER is mandatory,
                // an error should instead block the request — tracked as an M2
                // review item once the model is wired.
                tracing::warn!(error = %err, "ONNX NER inference failed; no NER entities");
                Vec::new()
            }
        }
    }
}

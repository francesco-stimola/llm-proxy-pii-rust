//! ONNX NER detector for UNSTRUCTURED entities (names, organizations,
//! locations) — milestone M2. Enabled by the `onnx` feature.
//!
//! CPU execution provider first (maximum compatibility/reproducibility); GPU
//! (CUDA / DirectML) comes later (M4) and is not automatic — it depends on the
//! model and its quantization.

use super::{PiiDetector, PiiEntity};

/// NER-based detector backed by an ONNX Runtime session.
pub struct OnnxNerDetector {
    // TODO(M2): ort::Session, tokenizer, and the id→label map.
}

impl OnnxNerDetector {
    /// Load a model from disk and initialize the runtime session on CPU.
    pub fn load(_model_path: &str) -> anyhow::Result<Self> {
        // TODO(M2): build the ort Session (CPU execution provider) and load the
        // matching tokenizer.
        todo!()
    }
}

impl PiiDetector for OnnxNerDetector {
    fn detect(&self, _input: &str) -> Vec<PiiEntity> {
        // TODO(M2): tokenize, run the session, decode BIO tags into entities.
        todo!()
    }
}

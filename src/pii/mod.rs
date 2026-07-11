//! PII detection: an engine-agnostic trait plus the entity types.
//!
//! Deterministic recognizers ([`recognizers`]) cover structured PII (M1); an
//! ONNX NER detector ([`onnx`], M2, feature `onnx`) covers unstructured
//! entities (names, organizations, locations). Both implement [`PiiDetector`],
//! so the pipeline never depends on a concrete engine.

pub mod anonymizer;
pub mod recognizers;

#[cfg(feature = "onnx")]
pub mod onnx;

use std::ops::Range;

/// A category of personally identifiable information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PiiKind {
    // Structured (deterministic recognizers — M1)
    Email,
    Phone,
    Ssn,
    CreditCard,
    Iban,
    // Unstructured (ONNX NER — M2)
    Person,
    Organization,
    Location,
}

/// A detected span of PII within a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiiEntity {
    /// What kind of PII this is.
    pub kind: PiiKind,
    /// Byte range of the match within the source text.
    pub span: Range<usize>,
    /// The exact matched text.
    pub text: String,
}

/// Engine-agnostic PII detector.
///
/// Deterministic recognizers and the ML NER model both implement this, so the
/// pipeline can combine or swap engines freely.
pub trait PiiDetector: Send + Sync {
    /// Return all PII entities found in `input`.
    fn detect(&self, input: &str) -> Vec<PiiEntity>;
}

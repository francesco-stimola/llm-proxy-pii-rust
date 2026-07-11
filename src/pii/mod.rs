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

use serde::{Deserialize, Serialize};

/// A category of personally identifiable information.
///
/// `Serialize`/`Deserialize` use the variant names verbatim (e.g. `"Email"`,
/// `"CreditCard"`), which is the encoding the JSON test corpus in
/// `tests/corpus/pii_cases.json` relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PiiKind {
    // Structured (deterministic recognizers — M1)
    Email,
    Phone,
    Ssn,
    CreditCard,
    Iban,
    /// API keys / tokens (e.g. `sk-…`, `sk-ant-…`, `AKIA…`). Deterministic —
    /// the old ML model missed these, so they are treated as structured PII.
    Secret,
    // Unstructured (ONNX NER — M2)
    Person,
    Organization,
    Location,
}

impl PiiKind {
    /// The uppercase label used inside placeholders, e.g. `Email` → `"EMAIL"`
    /// yields the `[EMAIL_1]` token. ASCII and tokenizer-friendly.
    pub fn label(self) -> &'static str {
        match self {
            PiiKind::Email => "EMAIL",
            PiiKind::Phone => "PHONE",
            PiiKind::Ssn => "SSN",
            PiiKind::CreditCard => "CARD",
            PiiKind::Iban => "IBAN",
            PiiKind::Secret => "SECRET",
            PiiKind::Person => "PERSON",
            PiiKind::Organization => "ORG",
            PiiKind::Location => "LOCATION",
        }
    }
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

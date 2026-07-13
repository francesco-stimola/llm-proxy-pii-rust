//! PII detection: an engine-agnostic trait plus the entity types.
//!
//! Deterministic recognizers ([`recognizers`]) cover structured PII (M1); an
//! ONNX NER detector ([`onnx`], M2, feature `onnx`) covers unstructured
//! entities (names, organizations, locations). Both implement [`PiiDetector`],
//! so the pipeline never depends on a concrete engine.

pub mod anonymizer;
pub mod composite;
pub mod ner_decode;
pub mod overlap;
pub mod recognizers;

#[cfg(feature = "onnx")]
pub mod hf;
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
    /// A non-US national identifier (M4) — e.g. Italian Codice Fiscale, UK NINO.
    /// US SSN keeps its own [`Ssn`](Self::Ssn) variant for continuity.
    NationalId,
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
            PiiKind::NationalId => "NATID",
            PiiKind::CreditCard => "CARD",
            PiiKind::Iban => "IBAN",
            PiiKind::Secret => "SECRET",
            PiiKind::Person => "PERSON",
            PiiKind::Organization => "ORG",
            PiiKind::Location => "LOCATION",
        }
    }

    /// Overlap-resolution priority: higher wins when two detected spans overlap
    /// (see [`overlap::resolve_overlaps`]). Deterministic **structured** PII
    /// outranks the ML **NER** entities, so a checksum-backed email/IBAN always
    /// beats an ML guess on the same span. Within structured PII the order is
    /// Email > Secret > Iban > CreditCard > Ssn ≈ NationalId > Phone.
    ///
    /// **Email outranks every other structured recognizer on purpose (M4-R7):** an
    /// `@`-bearing token that parses as an email genuinely *is* an email, and no
    /// other structured kind carries `@` — a card / IBAN / secret / national-ID can
    /// therefore only overlap an email by being a *substring* of its local part
    /// (`4111111111111111@x.com`, `123456789@x.com`). There the whole email is the
    /// correct match and must win, so those recognizers can't fragment it and leak
    /// the `@domain` in clear. (Non-email spans never share this tier, so lifting
    /// Email above them changes nothing outside the containment case.)
    pub fn priority(self) -> u8 {
        match self {
            PiiKind::Email => 6,
            PiiKind::Secret => 5,
            PiiKind::Iban => 4,
            PiiKind::CreditCard => 3,
            // National identifiers (US SSN + other locales) share a tier; they
            // never overlap each other, and ties fall through to span length.
            PiiKind::Ssn | PiiKind::NationalId => 2,
            PiiKind::Phone => 1,
            // NER entities (M2) sit below all structured PII.
            PiiKind::Person | PiiKind::Organization | PiiKind::Location => 0,
        }
    }

    /// Inverse of [`label`](Self::label): recover the kind from a placeholder
    /// label (case-insensitive). Lets the de-masker recognise a token that the
    /// model echoed back so it can warn if that token isn't in the vault.
    pub fn from_label(label: &str) -> Option<Self> {
        let kind = match label.to_ascii_uppercase().as_str() {
            "EMAIL" => PiiKind::Email,
            "PHONE" => PiiKind::Phone,
            "SSN" => PiiKind::Ssn,
            "NATID" => PiiKind::NationalId,
            "CARD" => PiiKind::CreditCard,
            "IBAN" => PiiKind::Iban,
            "SECRET" => PiiKind::Secret,
            "PERSON" => PiiKind::Person,
            "ORG" => PiiKind::Organization,
            "LOCATION" => PiiKind::Location,
            _ => return None,
        };
        Some(kind)
    }
}

/// How sure a recognizer is about a detection.
///
/// For a privacy tool a *structure-only* match is still masked (privacy beats
/// strict validation), but recording the distinction lets downstream code — audit
/// logging now, ML thresholds in M2 — reason about detection quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Backed by a checksum or an unambiguous format (e.g. a Luhn-valid card, a
    /// mod-97-valid IBAN, a well-formed email).
    Verified,
    /// Matched by structure alone — masked anyway, but not checksum-verified
    /// (e.g. a synthetic IBAN whose mod-97 fails).
    Structural,
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
    /// How strongly the recognizer trusts this hit.
    pub confidence: Confidence,
}

/// A detector failure that must be surfaced (e.g. ML inference / config error).
///
/// Carries only a static `detector` label and a category `message` — **never**
/// input-derived text, per the "never log raw PII" rule. Used to let a *required*
/// detector fail the request **closed** instead of silently dropping entities.
#[derive(Debug, Clone)]
pub struct DetectError {
    /// Which detector failed (a static label, never the input).
    pub detector: &'static str,
    /// A category / reason with no input text.
    pub message: String,
}

impl std::fmt::Display for DetectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} detector failed: {}", self.detector, self.message)
    }
}

impl std::error::Error for DetectError {}

/// Engine-agnostic PII detector.
///
/// Deterministic recognizers and the ML NER model both implement this, so the
/// pipeline can combine or swap engines freely.
pub trait PiiDetector: Send + Sync {
    /// Return all PII entities found in `input`. Infallible view — a detector
    /// that can fail returns whatever it could detect (typically empty on error).
    fn detect(&self, input: &str) -> Vec<PiiEntity>;

    /// Fallible detection. The default is infallible ([`detect`](Self::detect));
    /// a detector that can genuinely fail (ML inference, bad config) overrides
    /// this so a *required* detector can fail the request **closed**.
    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        Ok(self.detect(input))
    }
}

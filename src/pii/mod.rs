//! PII detection: an engine-agnostic trait plus the entity types.
//!
//! Deterministic recognizers ([`recognizers`]) cover structured PII (M1); an
//! ONNX NER detector ([`onnx`], M2, feature `onnx`) covers unstructured
//! entities (names, organizations, locations). Both implement [`PiiDetector`],
//! so the pipeline never depends on a concrete engine.

pub mod anonymizer;
pub mod cache;
pub mod composite;
pub mod gliner_decode;
pub mod ner_decode;
pub mod overlap;
pub mod recognizers;

#[cfg(feature = "onnx")]
pub mod bench;
#[cfg(feature = "onnx")]
pub mod gliner;
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

    /// Overlap-resolution priority (see [`overlap::resolve_overlaps`]). Order:
    /// Secret > Iban > CreditCard > Ssn ≈ NationalId > Phone > Email > NER.
    ///
    /// **This ranks *labels*, not survivors.** Two overlapping **structured** spans are
    /// merged into their union rather than one being dropped (the resolver's invariant:
    /// *no structured span's bytes are ever abandoned* — M4-R10/R11), so priority only
    /// decides **which kind names the union**. A lower priority therefore never costs
    /// coverage. It does still pick the survivor for **NER** spans, which keep the
    /// whole-span drop (M2-R7) — losing a `Person` remainder costs recall, never a leak.
    ///
    /// `Email` sits last on purpose. It is the only structured kind carrying `@`, so it
    /// overlaps the others in two shapes: it either **encloses** them (a card/ID as a
    /// substring of its local part, `4111111111111111@x.com`), or it **partially**
    /// overlaps them (`4111 1111 1111 1111@x.com`: a local part can't hold a space, so the
    /// email is only `1111@x.com`). In the second shape the union is better named by the
    /// checksum-backed card than by the fragmentary email — hence last. The first shape is
    /// handled by a **naming rule**, not by priority: when a union is *exactly* an `Email`
    /// span the email keeps the label (see [`overlap::name_of`]). Both spans are masked
    /// either way — nothing structured is ever dropped.
    pub fn priority(self) -> u8 {
        match self {
            PiiKind::Secret => 6,
            PiiKind::Iban => 5,
            PiiKind::CreditCard => 4,
            // National identifiers (US SSN + other locales) share a tier; ties fall
            // through to span length, then to the incumbent — always deterministic.
            PiiKind::Ssn | PiiKind::NationalId => 3,
            PiiKind::Phone => 2,
            // Names a union only when it doesn't contain the other span — see above.
            PiiKind::Email => 1,
            // NER entities (M2) sit below all structured PII.
            PiiKind::Person | PiiKind::Organization | PiiKind::Location => 0,
        }
    }

    /// Whether this kind comes from the deterministic **structured** recognizers
    /// (M1/M4) as opposed to the ML **NER** (`Person` / `Organization` / `Location`).
    /// [`overlap::resolve_overlaps`] uses it to split the two: structured spans are
    /// union-merged (never dropped), NER spans keep the whole-span drop.
    pub fn is_structured(self) -> bool {
        !matches!(
            self,
            PiiKind::Person | PiiKind::Organization | PiiKind::Location
        )
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

    /// Detection for a **fixpoint pass after the first** ([`Vault::mask_all`](crate::pii::anonymizer::Vault::mask_all), S4).
    ///
    /// Masking rewrites the bytes around what it replaced and can *expose* PII that was hidden
    /// before — but only for detectors whose matches are **revealed by splitting a token**: the
    /// structured recognizers, where a masked phone splits a digit run into a Luhn-valid card.
    /// The NER's matches — names, orgs, locations — are never *revealed* by masking a neighbour;
    /// re-running it only re-tags the **sub-word fragments it emitted** (`"lack"` of `"Slack"`),
    /// which on a dense system prompt drove masking past [`MAX_MASK_PASSES`](crate::pii::anonymizer)
    /// into a fail-closed 400 (CC-05/CC-08). So a detector that is **idempotent once the text is
    /// masked** overrides this to return nothing, and the fixpoint converges in O(1) NER passes.
    ///
    /// The default **rescans** (`try_detect`) — the safe direction: a new detector re-runs on
    /// every pass unless it explicitly declares itself masking-idempotent. Skipping a detector
    /// here must be justified by "masking a neighbour cannot reveal one of its matches", and the
    /// no-recall-loss claim that rests on it is measured, not assumed (S4; DEVLOG 2026-07-18).
    fn redetect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        self.try_detect(input)
    }
}

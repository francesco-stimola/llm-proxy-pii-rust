//! Deterministic recognizers for STRUCTURED PII (M1): email, phone, SSN,
//! credit card (Luhn-validated), IBAN. High precision, no ML model.
//!
//! These reproduce (and must pass) the ported reference cases in
//! `tests/reference/old-proxy/`.

use super::{PiiDetector, PiiEntity};

/// The default set of structured-PII recognizers, run together over a text.
pub struct StructuredRecognizers {
    // TODO(M1): compiled `regex::Regex` patterns per category, built once.
}

impl StructuredRecognizers {
    /// Build the recognizer set (compiles the regexes once).
    pub fn new() -> Self {
        // TODO(M1): compile email / phone / SSN / credit-card / IBAN patterns.
        todo!()
    }
}

impl Default for StructuredRecognizers {
    fn default() -> Self {
        Self::new()
    }
}

impl PiiDetector for StructuredRecognizers {
    fn detect(&self, _input: &str) -> Vec<PiiEntity> {
        // TODO(M1): run each recognizer; validate credit-card candidates with
        // `luhn_valid` and IBANs with the mod-97 checksum before accepting.
        todo!()
    }
}

/// Luhn checksum validation for credit-card candidates (rejects false positives
/// like arbitrary 16-digit numbers).
pub fn luhn_valid(_digits: &str) -> bool {
    // TODO(M1): standard Luhn algorithm over the numeric digits.
    todo!()
}

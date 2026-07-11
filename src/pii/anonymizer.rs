//! Reversible anonymization: replace detected PII with typed placeholders and
//! restore them on the way back.
//!
//! The [`Vault`] maps placeholder → original for a single request/response
//! round-trip, so [`Vault::demask`] restores the exact original text.

use std::collections::HashMap;

use super::PiiEntity;

/// Placeholder ↔ original-value store for one request.
#[derive(Debug, Default)]
pub struct Vault {
    /// placeholder (e.g. `«EMAIL_1»`) → original value.
    map: HashMap<String, String>,
    // TODO(M1): per-kind counters to produce stable, low-collision tokens.
}

impl Vault {
    /// Create an empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace each entity in `text` with a typed placeholder, recording the
    /// original in the vault. Returns the anonymized text.
    ///
    /// Text with no entities is returned unchanged.
    pub fn mask(&mut self, _text: &str, _entities: &[PiiEntity]) -> String {
        // TODO(M1): iterate spans right-to-left, allocate a typed placeholder
        // per value, insert into `map`, and splice it into the text.
        let _ = &self.map;
        todo!()
    }

    /// Restore placeholders in `text` back to their original values.
    pub fn demask(&self, _text: &str) -> String {
        // TODO(M1): replace every known placeholder with its original value.
        todo!()
    }
}

//! Reversible anonymization: replace detected PII with typed placeholders and
//! restore them on the way back.
//!
//! The [`Vault`] maps placeholder → original for a single request/response
//! round-trip, so [`Vault::demask`] restores the exact original text. Assignment
//! is **deterministic**: the same real value always maps to the same placeholder
//! within a vault, which lets the downstream model correlate a value across a
//! multi-turn (stateless) conversation.

use std::collections::HashMap;

use super::{PiiEntity, PiiKind};

/// Placeholder ↔ original-value store for one request.
#[derive(Debug, Default)]
pub struct Vault {
    /// placeholder (e.g. `[EMAIL_1]`) → original value.
    to_original: HashMap<String, String>,
    /// original value → placeholder, so a repeated value reuses its token.
    to_placeholder: HashMap<String, String>,
    /// per-kind counter, so tokens are numbered `[EMAIL_1]`, `[EMAIL_2]`, …
    counters: HashMap<PiiKind, usize>,
}

impl Vault {
    /// Create an empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether anything has been masked yet — i.e. no placeholders exist.
    pub fn is_empty(&self) -> bool {
        self.to_original.is_empty()
    }

    /// Replace each entity in `text` with a typed placeholder, recording the
    /// original in the vault. Returns the anonymized text.
    ///
    /// Placeholders are numbered in reading order (left to right), but the
    /// splice happens right to left so earlier byte offsets stay valid. Text
    /// with no entities is returned unchanged.
    pub fn mask(&mut self, text: &str, entities: &[PiiEntity]) -> String {
        if entities.is_empty() {
            return text.to_string();
        }

        // First pass, left→right: assign (or reuse) a placeholder per value so
        // numbering follows reading order.
        let mut ordered: Vec<&PiiEntity> = entities.iter().collect();
        ordered.sort_by_key(|e| e.span.start);
        for entity in &ordered {
            self.placeholder_for(entity);
        }

        // Second pass, right→left: splice placeholders in without shifting the
        // byte offsets of not-yet-processed spans.
        let mut out = text.to_string();
        ordered.sort_by(|a, b| b.span.start.cmp(&a.span.start));
        for entity in ordered {
            let placeholder = self.to_placeholder[&entity.text].clone();
            out.replace_range(entity.span.clone(), &placeholder);
        }
        out
    }

    /// Restore placeholders in `text` back to their original values.
    ///
    /// Order-independent: the `]` terminator means no placeholder is a prefix of
    /// another (`[EMAIL_1]` never matches inside `[EMAIL_11]`).
    pub fn demask(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (placeholder, original) in &self.to_original {
            if out.contains(placeholder.as_str()) {
                out = out.replace(placeholder.as_str(), original);
            }
        }
        out
    }

    /// Look up (or mint) the placeholder for an entity's value.
    fn placeholder_for(&mut self, entity: &PiiEntity) -> String {
        if let Some(existing) = self.to_placeholder.get(&entity.text) {
            return existing.clone();
        }
        let counter = self.counters.entry(entity.kind).or_insert(0);
        *counter += 1;
        let placeholder = format!("[{}_{}]", entity.kind.label(), counter);
        self.to_placeholder
            .insert(entity.text.clone(), placeholder.clone());
        self.to_original
            .insert(placeholder.clone(), entity.text.clone());
        placeholder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::PiiDetector;
    use crate::pii::recognizers::StructuredRecognizers;

    fn mask_roundtrip(input: &str) -> (String, String) {
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect(input);
        let masked = vault.mask(input, &entities);
        let restored = vault.demask(&masked);
        (masked, restored)
    }

    #[test]
    fn no_pii_is_unchanged() {
        let (masked, restored) = mask_roundtrip("Hello world, no PII here");
        assert_eq!(masked, "Hello world, no PII here");
        assert_eq!(restored, "Hello world, no PII here");
    }

    #[test]
    fn masks_and_restores_multiple_pii() {
        let input = "My email is bob@test.com and my phone is 555-111-2222";
        let (masked, restored) = mask_roundtrip(input);
        assert!(!masked.contains("bob@test.com"));
        assert!(!masked.contains("555-111-2222"));
        assert!(masked.contains("[EMAIL_1]"));
        assert!(masked.contains("[PHONE_1]"));
        assert_eq!(restored, input);
    }

    #[test]
    fn same_value_gets_same_placeholder() {
        // VAULT-05: determinism within a text.
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let input = "write to a@b.com, again a@b.com";
        let entities = detector.detect(input);
        let masked = vault.mask(input, &entities);
        assert_eq!(masked, "write to [EMAIL_1], again [EMAIL_1]");
    }
}

//! Reversible anonymization: replace detected PII with typed placeholders and
//! restore them on the way back.
//!
//! The [`Vault`] maps placeholder → original for a single request/response
//! round-trip, so [`Vault::demask`] restores the exact original text. Assignment
//! is **deterministic**: the same real value always maps to the same placeholder
//! within a vault, which lets the downstream model correlate a value across a
//! multi-turn (stateless) conversation.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

use super::{PiiEntity, PiiKind};

/// Tolerant placeholder pattern used on the way back. It accepts the canonical
/// `[EMAIL_1]` **and** the corruptions a model tends to introduce — a space or
/// dash for the underscore, stray inner spaces, a lowercased label:
/// `[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]`. This keeps restore from silently
/// failing when the model reformats a placeholder.
static PLACEHOLDER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\s*([A-Za-z]+)[ _-]*([0-9]+)\s*\]").unwrap());

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
    /// A single tolerant pass (see [`PLACEHOLDER_RE`]): every placeholder-shaped
    /// token is normalized to its canonical `[LABEL_N]` form and looked up. A
    /// token that isn't in the vault is left untouched — and if it still looks
    /// like one of our kinds (so the model probably mangled or invented it), a
    /// warning is logged rather than silently shipping a broken placeholder.
    pub fn demask(&self, text: &str) -> String {
        if self.to_original.is_empty() {
            return text.to_string();
        }
        PLACEHOLDER_RE
            .replace_all(text, |caps: &Captures| {
                let canonical = format!("[{}_{}]", caps[1].to_ascii_uppercase(), &caps[2]);
                if let Some(original) = self.to_original.get(&canonical) {
                    return original.clone();
                }
                if PiiKind::from_label(&caps[1]).is_some() {
                    tracing::warn!(
                        placeholder = %&caps[0],
                        "unresolved PII placeholder in response; left as-is"
                    );
                }
                caps[0].to_string()
            })
            .into_owned()
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

    #[test]
    fn demask_tolerates_model_corrupted_placeholders() {
        // The model may reformat a token; restore must still work.
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect("mail bob@test.com");
        let _ = vault.mask("mail bob@test.com", &entities);

        for corrupted in ["[EMAIL_1]", "[EMAIL 1]", "[email-1]", "[ EMAIL_1 ]"] {
            assert_eq!(
                vault.demask(&format!("sent to {corrupted}.")),
                "sent to bob@test.com."
            );
        }
    }

    #[test]
    fn demask_leaves_unknown_bracketed_text_untouched() {
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect("mail bob@test.com");
        let _ = vault.mask("mail bob@test.com", &entities);

        // Not a known placeholder → passed through verbatim.
        assert_eq!(vault.demask("see [TODO 3] and [EMAIL_1]"), "see [TODO 3] and bob@test.com");
    }
}

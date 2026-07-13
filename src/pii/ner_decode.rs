//! Pure decoding of token-classification NER output into [`PiiEntity`] spans.
//!
//! Kept free of `ort`/`tokenizers` on purpose: the interesting logic — mapping
//! model labels to [`PiiKind`], merging BIO tags, and turning token offsets into
//! character spans — is deterministic and unit-testable **without** a model or
//! the native ONNX runtime. [`onnx`](super::onnx) feeds it real tokens.

use super::{Confidence, PiiEntity, PiiKind};

/// One tokenizer token with its predicted label and byte offsets into the input.
#[derive(Debug, Clone)]
pub struct TokenTag<'a> {
    /// Predicted label, e.g. `B-PER`, `I-LOC`, `O` (BIO prefix optional).
    pub label: &'a str,
    /// Byte offset range of the token in the original text.
    pub start: usize,
    pub end: usize,
}

/// Map a NER label to a [`PiiKind`], stripping any `B-`/`I-` prefix. Returns
/// `None` for `O` / unknown labels, and — deliberately — for a PII-specialist
/// model's **structured** categories (email / phone / card / id / account …):
/// those are owned by the deterministic layer, so the NER should not also mask
/// them. Covers both plain PER/ORG/LOC schemes (XLM-R, WikiNeural) and
/// granular PII schemes (Piiranha: GIVENNAME / SURNAME / CITY …).
pub fn label_to_kind(label: &str) -> Option<PiiKind> {
    let tag = label.split(['-', '_']).next_back().unwrap_or(label);
    match tag.to_ascii_uppercase().as_str() {
        "PER" | "PERSON" | "GIVENNAME" | "SURNAME" | "FIRSTNAME" | "LASTNAME" | "NAME" => {
            Some(PiiKind::Person)
        }
        "ORG" | "ORGANIZATION" | "COMPANY" => Some(PiiKind::Organization),
        "LOC" | "GPE" | "LOCATION" | "CITY" | "STATE" | "COUNTRY" => Some(PiiKind::Location),
        _ => None,
    }
}

/// Whether a label opens a new entity (`B-`/`B_`, case-insensitive). Handles
/// both `-` and `_` BIO separators so `B_PER` counts as a begin, not just `B-PER`
/// (M2-R5). A prefix-less label (e.g. bare `PER`) is not a begin, so consecutive
/// same-kind tokens still merge.
fn is_begin(label: &str) -> bool {
    label
        .split(['-', '_'])
        .next()
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("B"))
}

/// Ensure the configured label list matches the model's class count. A mismatch
/// would let an out-of-range class id silently decode to `O` and drop a real
/// entity, so this fails loudly instead (M2-R3).
pub fn validate_label_count(id2label_len: usize, num_labels: usize) -> Result<(), String> {
    if id2label_len == num_labels {
        Ok(())
    } else {
        Err(format!(
            "NER label count mismatch: {id2label_len} labels vs {num_labels} model classes"
        ))
    }
}

/// Merge BIO-tagged tokens into entity spans. Consecutive tokens of the same
/// kind are joined (a `B-` label always starts a fresh entity, so two adjacent
/// same-type entities aren't glued together). All NER hits are tagged
/// [`Confidence::Structural`] — an ML guess is never checksum-verified.
pub fn decode_entities(text: &str, tokens: &[TokenTag<'_>]) -> Vec<PiiEntity> {
    let mut out = Vec::new();
    let mut current: Option<(PiiKind, usize, usize)> = None;

    let flush = |current: &mut Option<(PiiKind, usize, usize)>, out: &mut Vec<PiiEntity>| {
        if let Some((kind, start, end)) = current.take() {
            match text.get(start..end) {
                Some(slice) => {
                    // Tokenizer offsets can include the leading space
                    // (SentencePiece metaspace, e.g. XLM-R emits `▁Mario` spanning
                    // " Mario"). Trim surrounding whitespace and shift the span to
                    // the real content, so masking preserves the space and the
                    // span text matches the value exactly.
                    let lead = slice.len() - slice.trim_start().len();
                    let trimmed = slice.trim();
                    if trimmed.is_empty() {
                        return;
                    }
                    let start = start + lead;
                    out.push(PiiEntity {
                        kind,
                        span: start..start + trimmed.len(),
                        text: trimmed.to_string(),
                        confidence: Confidence::Structural,
                    });
                }
                // Offsets not on a UTF-8 boundary (e.g. char- vs byte-offset
                // mismatch) — surface it rather than silently dropping a name
                // (M2-R6). Kind only, never the text.
                None => tracing::warn!(
                    kind = ?kind,
                    "NER span offsets off a char boundary; entity dropped"
                ),
            }
        }
    };

    for token in tokens {
        match label_to_kind(token.label) {
            None => flush(&mut current, &mut out),
            Some(kind) => {
                let begins = is_begin(token.label);
                match current {
                    Some((cur_kind, cur_start, _)) if cur_kind == kind && !begins => {
                        current = Some((cur_kind, cur_start, token.end));
                    }
                    _ => {
                        flush(&mut current, &mut out);
                        current = Some((kind, token.start, token.end));
                    }
                }
            }
        }
    }
    flush(&mut current, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag<'a>(label: &'a str, start: usize, end: usize) -> TokenTag<'a> {
        TokenTag { label, start, end }
    }

    #[test]
    fn label_mapping_strips_bio_and_handles_synonyms() {
        assert_eq!(label_to_kind("B-PER"), Some(PiiKind::Person));
        assert_eq!(label_to_kind("I-PERSON"), Some(PiiKind::Person));
        assert_eq!(label_to_kind("B-ORG"), Some(PiiKind::Organization));
        assert_eq!(label_to_kind("GPE"), Some(PiiKind::Location));
        assert_eq!(label_to_kind("B-LOC"), Some(PiiKind::Location));
        assert_eq!(label_to_kind("O"), None);
        assert_eq!(label_to_kind("MISC"), None);
        // Granular PII scheme (Piiranha): names → Person, city → Location…
        assert_eq!(label_to_kind("I-GIVENNAME"), Some(PiiKind::Person));
        assert_eq!(label_to_kind("I-SURNAME"), Some(PiiKind::Person));
        assert_eq!(label_to_kind("I-CITY"), Some(PiiKind::Location));
        // …but structured PII stays with the deterministic layer.
        assert_eq!(label_to_kind("I-EMAIL"), None);
        assert_eq!(label_to_kind("I-TELEPHONENUM"), None);
        assert_eq!(label_to_kind("I-CREDITCARDNUMBER"), None);
    }

    #[test]
    fn merges_multi_token_person() {
        let text = "Mario Rossi called";
        let tokens = [tag("B-PER", 0, 5), tag("I-PER", 6, 11), tag("O", 12, 18)];
        let got = decode_entities(text, &tokens);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Person);
        assert_eq!(got[0].text, "Mario Rossi");
        assert_eq!(got[0].span, 0..11);
        assert_eq!(got[0].confidence, Confidence::Structural);
    }

    #[test]
    fn adjacent_same_type_entities_are_split_by_begin() {
        // "New York" then "London" — two LOCs must not glue into one span.
        let text = "New York London";
        let tokens = [tag("B-LOC", 0, 3), tag("I-LOC", 4, 8), tag("B-LOC", 9, 15)];
        let got = decode_entities(text, &tokens);
        assert_eq!(
            got.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            vec!["New York", "London"]
        );
    }

    #[test]
    fn ignores_o_and_unknown_labels() {
        let text = "just some words";
        let tokens = [tag("O", 0, 4), tag("MISC", 5, 9), tag("O", 10, 15)];
        assert!(decode_entities(text, &tokens).is_empty());
    }

    #[test]
    fn underscore_bio_prefix_also_splits_entities() {
        // M2-R5: a model using `B_LOC`/`I_LOC` must still split adjacent LOCs.
        let text = "New York London";
        let tokens = [tag("B_LOC", 0, 3), tag("I_LOC", 4, 8), tag("B_LOC", 9, 15)];
        let got = decode_entities(text, &tokens);
        assert_eq!(
            got.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            vec!["New York", "London"]
        );
    }

    #[test]
    fn label_count_validation() {
        assert!(validate_label_count(9, 9).is_ok());
        assert!(validate_label_count(3, 9).is_err());
        assert!(validate_label_count(9, 3).is_err());
    }

    #[test]
    fn leading_space_in_token_offset_is_trimmed() {
        // SentencePiece metaspace: the token offset includes the leading space
        // (e.g. XLM-R `▁Acme` spans " Acme"). The decoded span must be trimmed to
        // the real content, with the offsets shifted to match.
        let text = "at Acme Corp";
        let tokens = [tag("B-ORG", 2, 7), tag("I-ORG", 7, 12)];
        let got = decode_entities(text, &tokens);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "Acme Corp");
        assert_eq!(got[0].span, 3..12);
    }
}

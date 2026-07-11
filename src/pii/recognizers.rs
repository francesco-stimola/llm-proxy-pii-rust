//! Deterministic recognizers for STRUCTURED PII (M1): email, phone, SSN,
//! credit card (Luhn-validated), IBAN, and secrets/API keys. High precision,
//! no ML model.
//!
//! These reproduce (and must pass) the ported reference cases in
//! `tests/reference/old-proxy/` and the data-driven corpus in
//! `tests/corpus/pii_cases.json`.
//!
//! ## How detection works
//!
//! Each category is a compiled [`Regex`]. Every regex is run over the input,
//! optional per-category validation is applied (Luhn for credit cards), and the
//! resulting candidate spans are reconciled by [`resolve_overlaps`] so that a
//! single stretch of text is never labelled twice. This directly fixes the old
//! proxy's bug where an IBAN was mis-masked as a phone number: IBAN outranks
//! phone, so it wins any overlap.

use regex::Regex;

use super::overlap::resolve_overlaps;
use super::{Confidence, PiiDetector, PiiEntity, PiiKind};

/// One compiled recognizer: a category, its pattern, and an optional validator
/// applied to each raw match. Overlap priority comes from the kind
/// ([`PiiKind::priority`]).
struct Recognizer {
    kind: PiiKind,
    regex: Regex,
    /// Extra check on the matched text; `None` means "accept every match".
    validate: Option<fn(&str) -> bool>,
}

/// The default set of structured-PII recognizers, run together over a text.
pub struct StructuredRecognizers {
    recognizers: Vec<Recognizer>,
}

impl StructuredRecognizers {
    /// Build the recognizer set, compiling every pattern once.
    ///
    /// Locales covered: **IT + US** (Italian/US phone shapes, US SSN, IBAN
    /// including Italian). The regexes are deliberately conservative — precision
    /// matters more than recall for structured PII, and the ML NER (M2) picks up
    /// the fuzzy cases.
    pub fn new() -> Self {
        // Patterns are simple and readable on purpose; `regex` has no
        // backreferences/lookarounds, and we don't need them here.
        let recognizers = vec![
            // Secrets outrank everything: an API key must never be re-read as
            // something else, and the old ML model missed them entirely.
            Recognizer {
                kind: PiiKind::Secret,
                regex: Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{6,}|AKIA[0-9A-Z]{16})\b").unwrap(),
                validate: None,
            },
            // IBAN before phone/credit-card: its digit groups can otherwise be
            // mistaken for a card or phone number.
            Recognizer {
                kind: PiiKind::Iban,
                // Country (2 letters) + 2 check digits + BBAN, in one of the two
                // canonical shapes: continuous (`IT60X05428…`) or space-grouped
                // in blocks of four (`DE89 3704 0044 …`). Matching the shapes
                // explicitly — instead of "optional space before any char" —
                // stops a match from bleeding into a following ALL-CAPS word
                // (e.g. the `EUR` in `IBAN IT60…456 EUR`). mod-97 is a confidence
                // signal only, not a gate, so synthetic-but-shaped IBANs are
                // still masked (privacy > strict validation).
                regex: Regex::new(
                    r"\b[A-Z]{2}\d{2}(?:[A-Z0-9]{11,30}|(?: [A-Z0-9]{4}){2,7}(?: [A-Z0-9]{1,4})?)\b",
                )
                .unwrap(),
                validate: None,
            },
            // Credit cards: 13–19 digits, either grouped in 4s or continuous,
            // gated by the Luhn checksum to reject look-alikes.
            Recognizer {
                kind: PiiKind::CreditCard,
                regex: Regex::new(r"\b(?:\d{4}[ -]\d{4}[ -]\d{4}[ -]\d{4}|\d{13,19})\b").unwrap(),
                validate: Some(credit_card_valid),
            },
            // US SSN: 3-2-4 digit groups.
            Recognizer {
                kind: PiiKind::Ssn,
                regex: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
                validate: None,
            },
            Recognizer {
                kind: PiiKind::Email,
                regex: Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap(),
                validate: None,
            },
            // Phone, two families:
            //  - US: 3-3-4 with `-`, `.`, space, or `(area)` grouping, optional
            //    `+1` country code — 555-867-5309, (555) 867-5309, 555.867.5309,
            //    +1 555-867-5309.
            //  - International: `+CC` then two canonical shapes — three groups
            //    (+39 333 000 0001) or two groups (+39 333 0000001). Enumerating
            //    the shapes (rather than "1–4 groups") stops the match from
            //    swallowing an unrelated trailing number, e.g. the `12345` in
            //    `+39 333 0000001 12345` — the same class of bug as the IBAN
            //    over-match. Three-group is tried first so it isn't cut short.
            // US is tried first so `+1 …` isn't sliced by the international arm.
            Recognizer {
                kind: PiiKind::Phone,
                regex: Regex::new(
                    r"(?:\+1[ .-]?)?(?:\(\d{3}\)[ .-]?|\d{3}[ .-])\d{3}[ .-]\d{4}|\+\d{1,3} \d{2,4} \d{2,4} \d{3,4}|\+\d{1,3} \d{2,4} \d{5,8}",
                )
                .unwrap(),
                validate: None,
            },
        ];
        Self { recognizers }
    }
}

impl Default for StructuredRecognizers {
    fn default() -> Self {
        Self::new()
    }
}

impl PiiDetector for StructuredRecognizers {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        // Collect every raw candidate, applying the per-category validator as we
        // go; overlaps (and priority) are reconciled by the shared resolver.
        let mut candidates: Vec<PiiEntity> = Vec::new();
        for rec in &self.recognizers {
            for m in rec.regex.find_iter(input) {
                let text = m.as_str();
                if let Some(check) = rec.validate {
                    if !check(text) {
                        continue;
                    }
                }
                candidates.push(PiiEntity {
                    kind: rec.kind,
                    span: m.start()..m.end(),
                    text: text.to_string(),
                    confidence: confidence_of(rec.kind, text),
                });
            }
        }

        let kept = resolve_overlaps(candidates);
        for entity in &kept {
            if entity.confidence == Confidence::Structural {
                // Auditability: a structure-only match (e.g. a mod-97-invalid
                // IBAN) is masked anyway, but flagged. Log the KIND only —
                // never the value.
                tracing::debug!(
                    kind = ?entity.kind,
                    "structure-only PII match (checksum unverified)"
                );
            }
        }
        kept
    }
}

/// Confidence for a raw match. Everything structured is `Verified` (format- or
/// checksum-backed) except an IBAN whose mod-97 fails: that is still masked, but
/// tagged `Structural` so downstream code knows it wasn't checksum-verified.
fn confidence_of(kind: PiiKind, text: &str) -> Confidence {
    match kind {
        PiiKind::Iban if !iban_mod97(text) => Confidence::Structural,
        _ => Confidence::Verified,
    }
}

/// Accept a credit-card candidate only if it has 13–19 digits and passes Luhn.
fn credit_card_valid(matched: &str) -> bool {
    let digit_count = matched.chars().filter(|c| c.is_ascii_digit()).count();
    (13..=19).contains(&digit_count) && luhn_valid(matched)
}

/// Luhn checksum validation for credit-card candidates (rejects false positives
/// like arbitrary 16-digit numbers). Non-digit characters are ignored, so it
/// works on both grouped (`4111 1111 …`) and continuous inputs.
pub fn luhn_valid(input: &str) -> bool {
    let digits: Vec<u32> = input.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.is_empty() {
        return false;
    }
    let mut sum = 0u32;
    for (i, &d) in digits.iter().rev().enumerate() {
        if i % 2 == 1 {
            let doubled = d * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += d;
        }
    }
    sum % 10 == 0
}

/// ISO 13616 IBAN mod-97 checksum. Whitespace is ignored and letters are folded
/// to uppercase. Used as a *confidence signal* for IBAN detection, not a hard
/// gate — see [`StructuredRecognizers::new`].
pub fn iban_mod97(iban: &str) -> bool {
    let compact: String = iban
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if compact.len() < 4 {
        return false;
    }
    // Move the first four characters (country + check digits) to the end.
    let rearranged = format!("{}{}", &compact[4..], &compact[..4]);

    // Fold letters to 10..35 and reduce mod 97 incrementally to avoid big ints.
    let mut remainder: u32 = 0;
    for c in rearranged.chars() {
        let value = if c.is_ascii_digit() {
            c as u32 - '0' as u32
        } else if c.is_ascii_alphabetic() {
            c as u32 - 'A' as u32 + 10
        } else {
            return false;
        };
        remainder = if value >= 10 {
            (remainder * 100 + value) % 97
        } else {
            (remainder * 10 + value) % 97
        };
    }
    remainder == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<(PiiKind, String)> {
        StructuredRecognizers::new()
            .detect(input)
            .into_iter()
            .map(|e| (e.kind, e.text))
            .collect()
    }

    #[test]
    fn iban_beats_phone_and_card() {
        // Regression guard REG-01: the grouped IBAN must be a single IBAN span,
        // never split into a phone or credit-card match.
        let got = kinds("Transfer to DE89 3704 0044 0532 0130 00 today");
        assert_eq!(
            got,
            vec![(PiiKind::Iban, "DE89 3704 0044 0532 0130 00".to_string())]
        );
    }

    #[test]
    fn iban_does_not_absorb_a_following_word() {
        // M1 code review: the IBAN span must stop at the value; a trailing
        // ALL-CAPS token (e.g. a currency) must be left untouched.
        assert_eq!(
            kinds("IBAN IT60X0542811101000000123456 EUR"),
            vec![(PiiKind::Iban, "IT60X0542811101000000123456".to_string())]
        );
        assert_eq!(
            kinds("Pay DE89 3704 0044 0532 0130 00 EUR now"),
            vec![(PiiKind::Iban, "DE89 3704 0044 0532 0130 00".to_string())]
        );
    }

    #[test]
    fn structural_iban_is_masked_but_flagged() {
        // Synthetic (mod-97-invalid) IBAN is still detected, tagged Structural.
        let entities = StructuredRecognizers::new().detect("IT00A0000000000000000000001 ok");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].kind, PiiKind::Iban);
        assert_eq!(entities[0].confidence, Confidence::Structural);
    }

    #[test]
    fn phone_shape_variants_are_detected() {
        for phone in [
            "555-867-5309",
            "(555) 867-5309",
            "555.867.5309",
            "+1 555-867-5309",
            "+39 333 0000001",
            "+39 333 000 0001",
        ] {
            let got = kinds(&format!("call {phone} please"));
            assert_eq!(
                got,
                vec![(PiiKind::Phone, phone.to_string())],
                "phone shape {phone:?}"
            );
        }
    }

    #[test]
    fn non_luhn_16_digits_is_not_a_card() {
        // REG-04.
        assert!(kinds("tracking 1111 1111 1111 1111").is_empty());
    }

    #[test]
    fn luhn_accepts_known_cards_rejects_near_misses() {
        assert!(luhn_valid("4111111111111111"));
        assert!(luhn_valid("5105105105105100"));
        assert!(!luhn_valid("1111111111111111"));
        assert!(!luhn_valid("4111111111111112"));
    }

    #[test]
    fn iban_mod97_accepts_valid_rejects_invalid() {
        assert!(iban_mod97("DE89370400440532013000"));
        assert!(iban_mod97("IT60X0542811101000000123456"));
        assert!(!iban_mod97("DE00370400440532013000"));
    }
}

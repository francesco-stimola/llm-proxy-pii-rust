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
    /// Build the default recognizer set: the universal recognizers plus the
    /// **default locales (IT + US)**. Backward-compatible with the M1–M3 behavior.
    pub fn new() -> Self {
        Self::with_locales(&["it", "us"])
    }

    /// Build with an explicit list of locale codes (M4). Three tiers:
    ///
    /// - **Universal** — email, secret, credit card, IBAN (any country + mod-97),
    ///   phone (US + `+CC`) — always on.
    /// - **National identifiers** — US SSN, IT Codice Fiscale, GB NINO, ES DNI/NIE,
    ///   FR NIR — **always on regardless of `locales`** (M4-R1, privacy-first: a
    ///   national ID that reaches the proxy is masked even if its country isn't
    ///   configured). Each is specific enough (checksums / prefix rules) to stay
    ///   near-zero false-positive when always on.
    /// - **FP-prone** — ambiguous recognizers (e.g. national *phone* formats with
    ///   no `+CC`) — opt-in per locale via `locales`.
    ///
    /// So `locales` (from `PII_LOCALES`) gates *ambiguous* recognizers, not "which
    /// countries" — national IDs are never gated off.
    pub fn with_locales<S: AsRef<str>>(locales: &[S]) -> Self {
        let mut recognizers = universal_recognizers();
        recognizers.extend(national_id_recognizers());
        for locale in locales {
            recognizers.extend(fp_prone_recognizers(locale.as_ref()));
        }
        Self { recognizers }
    }
}

/// Locale-agnostic recognizers — valid regardless of the active locale set.
fn universal_recognizers() -> Vec<Recognizer> {
    // Patterns are simple and readable on purpose; `regex` has no
    // backreferences/lookarounds, and we don't need them here.
    vec![
        // Secrets outrank everything: an API key must never be re-read as
        // something else, and the old ML model missed them entirely.
        Recognizer {
            kind: PiiKind::Secret,
            regex: Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{6,}|AKIA[0-9A-Z]{16})\b").unwrap(),
            validate: None,
        },
        // IBAN before phone/credit-card: its digit groups can otherwise be
        // mistaken for a card or phone number. Country (2 letters) + 2 check
        // digits + BBAN, continuous (`IT60X05428…`) or space-grouped in blocks of
        // four (`DE89 3704 0044 …`); matching the shapes explicitly stops a match
        // from bleeding into a following ALL-CAPS word (the `EUR` in `IBAN IT60…456
        // EUR`). Already covers every country; mod-97 is a confidence signal only.
        Recognizer {
            kind: PiiKind::Iban,
            regex: Regex::new(
                r"\b[A-Z]{2}\d{2}(?:[A-Z0-9]{11,30}|(?: [A-Z0-9]{4}){2,7}(?: [A-Z0-9]{1,4})?)\b",
            )
            .unwrap(),
            validate: None,
        },
        // Credit cards: 13–19 digits, either grouped in 4s or continuous, gated by
        // the Luhn checksum to reject look-alikes.
        Recognizer {
            kind: PiiKind::CreditCard,
            regex: Regex::new(r"\b(?:\d{4}[ -]\d{4}[ -]\d{4}[ -]\d{4}|\d{13,19})\b").unwrap(),
            validate: Some(credit_card_valid),
        },
        Recognizer {
            kind: PiiKind::Email,
            regex: Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap(),
            validate: None,
        },
        // Phone, two families (US first so `+1 …` isn't sliced by the intl arm):
        //  - US: 3-3-4 with `-`, `.`, space, or `(area)` grouping, optional `+1`.
        //  - International: `+CC` then two canonical shapes — three groups
        //    (+39 333 000 0001) or two groups (+39 333 0000001). Enumerating the
        //    shapes stops the match from swallowing an unrelated trailing number.
        Recognizer {
            kind: PiiKind::Phone,
            regex: Regex::new(
                r"(?:\+1[ .-]?)?(?:\(\d{3}\)[ .-]?|\d{3}[ .-])\d{3}[ .-]\d{4}|\+\d{1,3} \d{2,4} \d{2,4} \d{3,4}|\+\d{1,3} \d{2,4} \d{5,8}",
            )
            .unwrap(),
            validate: None,
        },
    ]
}

/// National-identifier recognizers — **always on** (M4-R1), independent of the
/// configured locales: a national ID that reaches the proxy is masked even if its
/// country isn't in `PII_LOCALES` (privacy-first, "a miss is a leak"). Each pattern
/// is specific (interleaved letters/digits, prefix rules, checksums) to keep the
/// always-on false-positive rate near zero.
fn national_id_recognizers() -> Vec<Recognizer> {
    vec![
        // US SSN: 3-2-4 digit groups (keeps its own `Ssn` kind / `[SSN_N]`).
        Recognizer {
            kind: PiiKind::Ssn,
            regex: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
            validate: None,
        },
        // Italian Codice Fiscale: 6 letters, 2 digits, letter, 2 digits, letter,
        // 3 digits, letter (16 chars). The interleaved digits make it specific.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"\b[A-Za-z]{6}\d{2}[A-Za-z]\d{2}[A-Za-z]\d{3}[A-Za-z]\b").unwrap(),
            validate: None,
        },
        // UK National Insurance Number: 2 prefix letters, 6 digits, a suffix letter
        // A–D — compact (`AB123456C`) or space-grouped (`AB 12 34 56 C`). The prefix
        // rules (M4-R2) reject look-alikes like an order code `PO123456A`.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(
                r"\b[A-Za-z]{2}\d{6}[A-Da-d]\b|\b[A-Za-z]{2} \d{2} \d{2} \d{2} [A-Da-d]\b",
            )
            .unwrap(),
            validate: Some(nino_prefix_valid),
        },
        // Spanish DNI (8 digits) / NIE (X/Y/Z + 7 digits), each with a mod-23 check
        // letter that must match — so a random 8-digit+letter token won't pass.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"\b(?:[XYZxyz]\d{7}|\d{8})[A-Za-z]\b").unwrap(),
            validate: Some(es_dni_nie_valid),
        },
        // French NIR (social security): 15 digits — sex + YY + MM + geo/order + a
        // mod-97 key that must check out (Corsica's 2A/2B letter form is a follow-up).
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"\b[12]\d{2}(?:0[1-9]|1[0-2])\d{10}\b").unwrap(),
            validate: Some(fr_nir_valid),
        },
    ]
}

/// **FP-prone** recognizers for one locale (M4-R1) — ambiguous patterns opted in
/// via `PII_LOCALES` (unlike national IDs, which are always on). None yet: the seam
/// is kept for national *phone* formats (numbers with no `+CC`), which need careful
/// precision work before they can run globally. Unknown codes yield nothing.
fn fp_prone_recognizers(code: &str) -> Vec<Recognizer> {
    match code.trim().to_ascii_lowercase().as_str() {
        // e.g. "gb" => vec![ UK national phone formats ] — deferred (see ROADMAP M4).
        _ => Vec::new(),
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

/// Validate a UK NINO's two-letter prefix (M4-R2): the shape regex alone masks any
/// `AB123456C`-looking token (e.g. an order code `PO123456A`), so the official
/// prefix rules narrow it. First letter ∉ {D,F,I,Q,U,V}; second ∉ {D,F,I,O,Q,U,V};
/// the pair is not one of the administratively invalid ones. Keeps precision high
/// enough for the always-on national-ID tier (M4-R1).
fn nino_prefix_valid(matched: &str) -> bool {
    let mut letters = matched.chars().filter(|c| c.is_ascii_alphabetic());
    let (Some(first), Some(second)) = (letters.next(), letters.next()) else {
        return false;
    };
    let first = first.to_ascii_uppercase();
    let second = second.to_ascii_uppercase();

    const INVALID_FIRST: &[char] = &['D', 'F', 'I', 'Q', 'U', 'V'];
    const INVALID_SECOND: &[char] = &['D', 'F', 'I', 'O', 'Q', 'U', 'V'];
    const INVALID_PAIRS: &[[char; 2]] = &[
        ['B', 'G'],
        ['G', 'B'],
        ['K', 'N'],
        ['N', 'K'],
        ['N', 'T'],
        ['T', 'N'],
        ['Z', 'Z'],
    ];

    !INVALID_FIRST.contains(&first)
        && !INVALID_SECOND.contains(&second)
        && !INVALID_PAIRS.contains(&[first, second])
}

/// Spanish DNI / NIE check letter (ISO mod-23). The 8-digit body (a NIE's X/Y/Z
/// prefix maps to 0/1/2) indexes a fixed 23-letter table; the final letter must
/// match. Rejects a random 8-digit+letter token — key to the always-on tier (M4).
fn es_dni_nie_valid(matched: &str) -> bool {
    const CONTROL: &[u8; 23] = b"TRWAGMYFPDXBNJZSQVHLCKE";
    let bytes = matched.to_ascii_uppercase().into_bytes();
    if bytes.len() != 9 {
        return false;
    }
    let check = bytes[8];
    // NIE prefix letter → leading digit; DNI starts straight at the digits.
    let (start, mut number) = match bytes[0] {
        b'X' => (1, String::from("0")),
        b'Y' => (1, String::from("1")),
        b'Z' => (1, String::from("2")),
        _ => (0, String::new()),
    };
    for &b in &bytes[start..8] {
        if !b.is_ascii_digit() {
            return false;
        }
        number.push(b as char);
    }
    match number.parse::<u64>() {
        Ok(n) => CONTROL[(n % 23) as usize] == check,
        Err(_) => false,
    }
}

/// French NIR (social-security number) mod-97 key check: the last 2 of the 15
/// digits are `97 - (first-13-digits mod 97)`. The key makes a plain 15-digit run
/// pass only ~1/97 of the time — specific enough for the always-on tier (M4).
fn fr_nir_valid(matched: &str) -> bool {
    let digits: Vec<u8> = matched.bytes().filter(u8::is_ascii_digit).collect();
    if digits.len() != 15 {
        return false;
    }
    let parse = |slice: &[u8]| std::str::from_utf8(slice).ok()?.parse::<u64>().ok();
    let (Some(body), Some(key)) = (parse(&digits[..13]), parse(&digits[13..])) else {
        return false;
    };
    97 - (body % 97) == key
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

    fn kinds_with<S: AsRef<str>>(locales: &[S], input: &str) -> Vec<(PiiKind, String)> {
        StructuredRecognizers::with_locales(locales)
            .detect(input)
            .into_iter()
            .map(|e| (e.kind, e.text))
            .collect()
    }

    #[test]
    fn italian_codice_fiscale_detected_by_default() {
        // IT is a default locale, so `new()` covers Codice Fiscale (M4).
        assert_eq!(
            kinds("codice RSSMRA85T10A562S grazie"),
            vec![(PiiKind::NationalId, "RSSMRA85T10A562S".to_string())]
        );
    }

    #[test]
    fn national_ids_are_always_on() {
        // M4-R1: national IDs run regardless of PII_LOCALES — a US-only set still
        // masks an Italian CF and a UK NINO (each specific enough to be safe
        // globally); an unrelated `fr` locale still gets US SSN. Privacy-first.
        assert_eq!(
            kinds_with(&["us"], "codice RSSMRA85T10A562S"),
            vec![(PiiKind::NationalId, "RSSMRA85T10A562S".to_string())]
        );
        assert_eq!(
            kinds_with(&["us"], "NINO AB123456C on file"),
            vec![(PiiKind::NationalId, "AB123456C".to_string())]
        );
        assert_eq!(
            kinds_with(&["fr"], "ssn 123-45-6789"),
            vec![(PiiKind::Ssn, "123-45-6789".to_string())]
        );
    }

    #[test]
    fn spanish_dni_nie_check_letter() {
        // Valid DNI + NIE (mod-23 letter matches)…
        assert!(es_dni_nie_valid("12345678Z"));
        assert!(es_dni_nie_valid("X1234567L"));
        // …wrong check letter rejected.
        assert!(!es_dni_nie_valid("12345678A"));
        assert!(!es_dni_nie_valid("X1234567M"));
    }

    #[test]
    fn spanish_dni_detected_and_lookalike_rejected() {
        assert_eq!(
            kinds("dni 12345678Z ok"),
            vec![(PiiKind::NationalId, "12345678Z".to_string())]
        );
        // A same-shaped token with the wrong check letter must not be masked.
        assert!(kinds("ref 12345678A").is_empty());
    }

    /// Append the correct mod-97 key to a 13-digit NIR body.
    fn fr_nir(body13: &str) -> String {
        let body: u64 = body13.parse().unwrap();
        let key = 97 - (body % 97);
        format!("{body13}{key:02}")
    }

    #[test]
    fn french_nir_key_check() {
        let nir = fr_nir("1851275116001");
        assert!(fr_nir_valid(&nir));
        // Wrong key (00 is never the real key, which is 1..=97) is rejected.
        assert!(!fr_nir_valid("185127511600100"));
    }

    #[test]
    fn french_nir_detected() {
        let nir = fr_nir("1851275116001");
        // Masked (kind may resolve to CreditCard if the number is also Luhn-valid,
        // but it must never be left in clear).
        let got = kinds(&format!("NIR {nir} today"));
        assert_eq!(got.len(), 1, "the NIR must be detected: {got:?}");
        assert_eq!(got[0].1, nir);
    }

    #[test]
    fn uk_nino_prefix_rules_reject_lookalikes() {
        // M4-R2: the shape alone would mask any 2-letter+6-digit+A–D token, so the
        // prefix rules must reject look-alikes (an order code, an invalid pair).
        // National IDs are always on, so this holds with the default locales.
        assert!(
            kinds("order PO123456A shipped").is_empty(),
            "second letter O is invalid — must not mask"
        );
        assert!(kinds("ref GB123456A").is_empty(), "GB is an invalid prefix pair");
        assert!(kinds("DA123456A").is_empty(), "first letter D is invalid");
        // A valid NINO still masks.
        assert_eq!(
            kinds("JT123456C"),
            vec![(PiiKind::NationalId, "JT123456C".to_string())]
        );
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

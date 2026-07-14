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
//!
//! ## Word boundaries are **ASCII** — `(?-u:\b)`, never a bare `\b` (M4-R13)
//!
//! Every anchored recognizer uses `(?-u:\b)`. Rust `regex`'s default `\b` is
//! **Unicode-aware**, in which a Han / Kana / Cyrillic / Greek / Arabic letter *is* a
//! word character — so there is **no boundary between a CJK character and a digit**.
//! Chinese and Japanese have no inter-word spaces, so the glued form is the *natural*
//! way to write it, not an evasion; with a Unicode `\b` these recognizers were simply
//! **inert** in CJK prose and the PII went upstream in clear:
//!
//! ```text
//! 我的信用卡号是4111111111111111   → a Luhn-valid card, matched by NOTHING
//! 我的身份证号是11010519491231002X → the zh Resident ID pack never fired
//! 密钥sk-abcdef123456              → an API secret, in clear
//! ```
//!
//! `(?-u:\b)` treats only `[0-9A-Za-z_]` as word characters, so a boundary appears
//! between a non-ASCII letter and a digit while the deliberate anti-false-positive
//! guarantee is preserved **exactly**: an ID still cannot fire inside a longer *ASCII*
//! token (`card4111111111111111`, an API key / hash / UUID / base64 blob stay unmatched).
//!
//! `Email` and `Phone` are unaffected — they are anchored by character classes, not `\b`.

use std::ops::Range;

use regex::Regex;

use super::overlap::resolve_overlaps;
use super::{Confidence, PiiDetector, PiiEntity, PiiKind};

/// One compiled recognizer: a category, its pattern, an optional validator applied to
/// each raw match, and how the scan advances after one. Overlap priority comes from the
/// kind ([`PiiKind::priority`]).
struct Recognizer {
    kind: PiiKind,
    regex: Regex,
    /// Extra check on the matched text; `None` means "accept every match".
    validate: Option<fn(&str) -> bool>,
    /// Whether this pattern's length is bounded — see [`Scan`].
    scan: Scan,
}

/// Where a recognizer resumes scanning after a match. **This is the M4-R17 / M4-R19
/// tradeoff, and it is decided by one property of the pattern: is its match length
/// bounded?**
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// Resume one `char` past the match's **start**, so a value that begins *inside* an
    /// earlier match of the same recognizer still becomes a candidate (M4-R17 — otherwise
    /// the resolver never learns it exists and every invariant over the candidate set is
    /// satisfied *vacuously* for it).
    ///
    /// It probes O(n) start positions, each costing at most one maximal match, so the cost
    /// is **O(n · L)** for a pattern whose longest match is L. That is linear only while
    /// **L is bounded** — which it is for every pattern below that uses this: card ≤ 19
    /// digits, IBAN ≤ 44 chars, phone ≤ 20, and every national ID ≤ 18.
    Overlapping,
    /// Resume at the match's **end** (plain `find_iter` semantics).
    ///
    /// For the two patterns with **no length bound** — `Email` (`[…]+@[…]+\.[…]{2,}`) and
    /// `Secret` (`sk-[…]{6,}`). There L = O(n), so the rescan above degenerates to
    /// **O(n²)**: a ~1 MB `content` field (far under the 16 MiB body limit) pegged a core
    /// for *minutes* on an unauthenticated path — 151 s at 200 KB, ~18,000× slower than a
    /// plain scan (**M4-R19**, an algorithmic-complexity DoS).
    ///
    /// **And the rescan buys these two nothing.** A same-recognizer match that starts
    /// inside an earlier one is *contained* in it, so it adds no bytes:
    /// - `Secret` — both `sk-…` and `AKIA…` run greedily to the end of the same maximal
    ///   `[A-Za-z0-9_-]` run and must end on an ASCII word boundary, so a nested hit ends
    ///   exactly where the outer one does (`sk-abcsk-defghi`). Containment, always.
    /// - `Email` — a hit starting inside the local part shares the same `@` and domain
    ///   (containment). The one shape that *isn't* contained is a chained
    ///   `a@b.com@c.com`, where the local part of the second hit lies inside the first
    ///   hit's **domain**. Its remainder is masked anyway, one pass later: masking the
    ///   first email leaves `[EMAIL_1]@c.com`, and [`Vault::mask_all`] re-detects to a
    ///   **fixpoint**, so anything still `local@domain`-shaped is caught then. What is
    ///   left over is a bare `@domain` — not PII (M4-R11).
    ///
    /// So the two mechanisms are complementary, and the DoS costs no coverage: the
    /// **bounded** recognizers get the rescan, the **unbounded** ones get the fixpoint.
    /// Pinned by `an_email_chain_leaves_nothing_detectable` and `tests/complexity.rs`.
    Sequential,
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
            regex: Regex::new(r"(?-u:\b)(?:sk-[A-Za-z0-9_-]{6,}|AKIA[0-9A-Z]{16})(?-u:\b)").unwrap(),
            validate: None,
            // Unbounded (`{6,}`) → no overlap rescan; a nested hit is always contained.
            scan: Scan::Sequential,
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
                r"(?-u:\b)[A-Z]{2}\d{2}(?:[A-Z0-9]{11,30}|(?: [A-Z0-9]{4}){2,7}(?: [A-Z0-9]{1,4})?)(?-u:\b)",
            )
            .unwrap(),
            validate: None,
            scan: Scan::Overlapping, // bounded: ≤ 44 chars
        },
        // Credit cards: 13–19 digits, either grouped in 4s or continuous, gated by
        // the Luhn checksum to reject look-alikes.
        Recognizer {
            kind: PiiKind::CreditCard,
            regex: Regex::new(r"(?-u:\b)(?:\d{4}[ -]\d{4}[ -]\d{4}[ -]\d{4}|\d{13,19})(?-u:\b)").unwrap(),
            validate: Some(credit_card_valid),
            scan: Scan::Overlapping, // bounded: ≤ 19 digits — and this is the M4-R17 repro
        },
        Recognizer {
            kind: PiiKind::Email,
            regex: Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap(),
            validate: None,
            // Unbounded (`+` on both sides of the `@`) → no overlap rescan; the
            // mask-to-a-fixpoint pass catches a chained `a@b.com@c.com` remainder.
            scan: Scan::Sequential,
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
            scan: Scan::Overlapping, // bounded: ≤ 20 chars
        },
    ]
}

/// National-identifier recognizers — **always on** (M4-R1), independent of the
/// configured locales: a national ID that reaches the proxy is masked even if its
/// country isn't in `PII_LOCALES` (privacy-first, "a miss is a leak"). Each pattern
/// is specific (interleaved letters/digits, prefix rules, checksums) to keep the
/// always-on false-positive rate near zero.
///
/// Every pattern here is **fixed-length** (≤ 18 chars), so they all take the
/// [`Scan::Overlapping`] rescan — bounded, hence linear (M4-R19).
fn national_id_recognizers() -> Vec<Recognizer> {
    vec![
        // US SSN: 3-2-4 digit groups (keeps its own `Ssn` kind / `[SSN_N]`).
        Recognizer {
            kind: PiiKind::Ssn,
            regex: Regex::new(r"(?-u:\b)\d{3}-\d{2}-\d{4}(?-u:\b)").unwrap(),
            validate: None,
            scan: Scan::Overlapping,
        },
        // Italian Codice Fiscale: 6 letters, 2 digits, letter, 2 digits, letter,
        // 3 digits, letter (16 chars). The final letter is a checksum (M4-R3), so a
        // wrong-checksum look-alike is rejected — consistent with the other IDs.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)[A-Za-z]{6}\d{2}[A-Za-z]\d{2}[A-Za-z]\d{3}[A-Za-z](?-u:\b)").unwrap(),
            validate: Some(cf_check_valid),
            scan: Scan::Overlapping,
        },
        // UK National Insurance Number: 2 prefix letters, 6 digits, a suffix letter
        // A–D — compact (`AB123456C`) or space-grouped (`AB 12 34 56 C`). The prefix
        // rules (M4-R2) reject look-alikes like an order code `PO123456A`.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(
                r"(?-u:\b)[A-Za-z]{2}\d{6}[A-Da-d](?-u:\b)|(?-u:\b)[A-Za-z]{2} \d{2} \d{2} \d{2} [A-Da-d](?-u:\b)",
            )
            .unwrap(),
            validate: Some(nino_prefix_valid),
            scan: Scan::Overlapping,
        },
        // Spanish DNI (8 digits) / NIE (X/Y/Z + 7 digits), each with a mod-23 check
        // letter that must match — so a random 8-digit+letter token won't pass.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)(?:[XYZxyz]\d{7}|\d{8})[A-Za-z](?-u:\b)").unwrap(),
            validate: Some(es_dni_nie_valid),
            scan: Scan::Overlapping,
        },
        // French NIR (social security): 15 digits — sex + YY + MM + geo/order + a
        // mod-97 key that must check out. The month admits INSEE special codes
        // (`20` unknown/born-abroad; `30–42`/`50–99` provisional SANDIA) so those
        // real NIRs aren't missed on the always-on tier (M4-R5). Corsica's `2A`/`2B`
        // department (letters in the body) is a documented gap — docs/reviews/M4.md#m4-r5.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)[12]\d{2}(?:0[1-9]|1[0-2]|20|3\d|4[0-2]|[5-9]\d)\d{10}(?-u:\b)").unwrap(),
            validate: Some(fr_nir_valid),
            scan: Scan::Overlapping,
        },
        // Nine-digit national IDs: NL BSN (11-proef) or PT NIF (mod-11). One
        // recognizer, either checksum accepts (both are 9 digits). Accepted FP
        // tradeoff (M4-R6): a mod-11 checksum still passes ~1/11 of arbitrary
        // 9-digit numbers per scheme (BSN ∪ NIF ≈ 2/11 ≈ 18%), so an ordinary
        // standalone 9-digit token that happens to check out is masked. That is the
        // privacy-first choice (over-mask, never leak) — context-gating it would
        // reintroduce leaks (M4-R1). The clean precision path is the contextual
        // GLiNER detector (Backlog), not a keyword gate.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{9}(?-u:\b)").unwrap(),
            validate: Some(nine_digit_id_valid),
            scan: Scan::Overlapping,
        },
        // Eleven-digit national IDs: DE Steuer-ID (ISO 7064 Mod 11,10 + one repeated
        // digit) or LV personal code (mod-11 / post-2017 `32…` random form). Same
        // accepted-FP tradeoff as the 9-digit recognizer above (M4-R6): DE ∪ LV
        // still masks a fraction of arbitrary 11-digit numbers (incl. the
        // unconditional LV `32…` ~1%) — privacy-first, over-mask never leak.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{11}(?-u:\b)").unwrap(),
            validate: Some(eleven_digit_id_valid),
            scan: Scan::Overlapping,
        },
        // LV personal code, classic dashed form `DDMMYY-NNNNC` (mod-11 checksum).
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{6}-\d{5}(?-u:\b)").unwrap(),
            validate: Some(lv_code_valid),
            scan: Scan::Overlapping,
        },
        // China Resident Identity Card: 17 digits + a check char (digit or X),
        // ISO 7064 MOD 11-2. 18 chars → near-zero false positives.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{17}[0-9Xx](?-u:\b)").unwrap(),
            validate: Some(zh_resident_id_valid),
            scan: Scan::Overlapping,
        },
    ]
}

/// **FP-prone** recognizers for one locale (M4-R1) — ambiguous patterns opted in
/// via `PII_LOCALES` (unlike national IDs, which are always on). None yet: the seam
/// is kept for national *phone* formats (numbers with no `+CC`), which need careful
/// precision work before they can run globally. Unknown codes yield nothing.
///
/// The `match` **is** the seam, which is why it stays even though every arm but the
/// wildcard is still missing: dissolving it (as clippy suggests) would drop `code` on the
/// floor and leave the next locale nowhere to land.
#[allow(clippy::match_single_binding)]
fn fp_prone_recognizers(code: &str) -> Vec<Recognizer> {
    match code.trim().to_ascii_lowercase().as_str() {
        // e.g. "gb" => vec![ UK national phone formats ] — deferred to the
        // docs/ROADMAP.md **Backlog** ("Locale phone national formats").
        _ => Vec::new(),
    }
}

impl Default for StructuredRecognizers {
    fn default() -> Self {
        Self::new()
    }
}

impl StructuredRecognizers {
    /// Every raw match, **before** overlap resolution — each recognizer's hits with its
    /// validator applied. Overlapping and duplicate spans are expected here; the shared
    /// [`resolve_overlaps`] reconciles them.
    ///
    /// Exposed (crate-internal) so the resolver's invariant can be tested directly: every
    /// structured candidate's bytes must end up covered in the resolved output — see
    /// `every_structured_candidate_byte_is_covered` (M4-R10 / M4-R11).
    pub(crate) fn raw_candidates(&self, input: &str) -> Vec<PiiEntity> {
        let mut candidates: Vec<PiiEntity> = Vec::new();
        for rec in &self.recognizers {
            rec.push_candidates(input, &mut candidates);
        }
        candidates
    }
}

impl Recognizer {
    /// Every match of this recognizer — for a [`Scan::Overlapping`] pattern, **including
    /// ones that overlap an earlier match** (M4-R17).
    ///
    /// `Regex::find_iter` is leftmost-**non-overlapping**: after a hit it resumes at the
    /// match's *end*. A real value that **starts inside** an earlier match of the *same*
    /// recognizer is therefore never emitted as a candidate at all — and an invariant over
    /// the candidate set (PROP-03) is then *vacuously* satisfied for it, because the
    /// resolver never learns the value exists. **An invariant is only ever as strong as the
    /// candidate set it quantifies over.** Concretely:
    ///
    /// ```text
    /// 4111 1111 1111 1111@123-45-6789-4111 1111 1111 1111
    ///                         └── the shifted window `6789-4111 1111 1111` is Luhn-valid
    ///                             and matches first, so the REAL trailing card (which
    ///                             starts inside it) is never a candidate → its last group
    ///                             ` 1111` was forwarded in clear.
    /// ```
    ///
    /// So a bounded pattern resumes one `char` past the match's **start**, not its end,
    /// which surfaces a value hidden behind an earlier match. An **unbounded** one
    /// ([`Scan::Sequential`] — Email, Secret) must not: there that rescan is O(n²) and buys
    /// no coverage (M4-R19). Read [`Scan`] before changing either.
    ///
    /// Overlapping hits of one recognizer are **coalesced into maximal runs** as we go.
    /// That keeps the candidate set bounded on pathological input (a long row of 4-digit
    /// groups yields a match at every group boundary — thousands of overlapping windows) at
    /// no cost to the guarantee: the resolver needs the *coverage*, and it would union
    /// those spans anyway. Each hit is validated (Luhn / checksum) **before** it joins a
    /// run, so a run is a union of genuine matches, never a free pass.
    fn push_candidates(&self, input: &str, out: &mut Vec<PiiEntity>) {
        let mut runs: Vec<Range<usize>> = Vec::new();
        let mut at = 0usize;
        while at <= input.len() {
            let Some(m) = self.regex.find_at(input, at) else {
                break;
            };
            if self.validate.is_none_or(|check| check(m.as_str())) {
                match runs.last_mut() {
                    // Hits arrive in non-decreasing start order, so only the last run can
                    // still grow.
                    Some(run) if m.start() < run.end => run.end = run.end.max(m.end()),
                    _ => runs.push(m.start()..m.end()),
                }
            }
            // Where the next scan starts — the whole M4-R17 / M4-R19 tradeoff, see [`Scan`].
            let resume = match self.scan {
                Scan::Overlapping => next_char_boundary(input, m.start()),
                Scan::Sequential => m.end(),
            };
            // Forward progress, always: a zero-width match (or one at the very end of the
            // input, where `next_char_boundary` clamps) would otherwise spin forever.
            if resume <= at {
                break;
            }
            at = resume;
        }

        out.extend(runs.into_iter().map(|span| {
            let text = &input[span.clone()];
            PiiEntity {
                kind: self.kind,
                text: text.to_string(),
                confidence: confidence_of(self.kind, text),
                span,
            }
        }));
    }
}

/// The first `char` boundary strictly after `i` (clamped to `input.len()`), so a scan can
/// always make forward progress without ever splitting a multi-byte character.
fn next_char_boundary(input: &str, i: usize) -> usize {
    let mut next = i + 1;
    while next < input.len() && !input.is_char_boundary(next) {
        next += 1;
    }
    next.min(input.len())
}

impl PiiDetector for StructuredRecognizers {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        let kept = resolve_overlaps(input, self.raw_candidates(input));
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
/// checksum-backed) except an IBAN that fails **either** its mod-97 checksum **or**
/// its country's expected length (M4): still masked (privacy-first), but tagged
/// `Structural` so downstream code knows it wasn't fully verified.
fn confidence_of(kind: PiiKind, text: &str) -> Confidence {
    match kind {
        PiiKind::Iban => {
            if iban_mod97(text) && iban_length_ok(text) {
                Confidence::Verified
            } else {
                Confidence::Structural
            }
        }
        _ => Confidence::Verified,
    }
}

/// Whether an IBAN's length matches its country's fixed length (ISO 13616). An
/// unknown country code isn't penalized (we can't check it — rely on mod-97).
fn iban_length_ok(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let cc = compact.get(..2).unwrap_or("").to_ascii_uppercase();
    match iban_country_length(&cc) {
        Some(len) => compact.len() == len,
        None => true,
    }
}

/// Fixed IBAN length per country (the SEPA / common set; extend as needed).
///
/// Hand-aligned as a lookup **table** — rustfmt would give each country its own line and
/// turn six scannable rows into thirty-six.
#[rustfmt::skip]
fn iban_country_length(cc: &str) -> Option<usize> {
    Some(match cc {
        "AD" => 24, "AE" => 23, "AT" => 20, "BE" => 16, "BG" => 22, "CH" => 21,
        "CY" => 28, "CZ" => 24, "DE" => 22, "DK" => 18, "EE" => 20, "ES" => 24,
        "FI" => 18, "FR" => 27, "GB" => 22, "GR" => 27, "HR" => 21, "HU" => 28,
        "IE" => 22, "IS" => 26, "IT" => 27, "LI" => 21, "LT" => 20, "LU" => 20,
        "LV" => 21, "MC" => 27, "MT" => 31, "NL" => 18, "NO" => 15, "PL" => 28,
        "PT" => 25, "RO" => 24, "SE" => 24, "SI" => 19, "SK" => 24, "SM" => 27,
        _ => return None,
    })
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

/// Italian Codice Fiscale check character (M4-R3). Each of the first 15 chars maps
/// through an odd/even table (odd for 1-indexed odd positions); the sum mod 26
/// yields the final letter. A wrong-checksum look-alike is rejected.
fn cf_check_valid(matched: &str) -> bool {
    // Value of a char as its "even" code: digits 0-9, letters A-Z → 0-25.
    fn even_val(c: u8) -> Option<u32> {
        match c {
            b'0'..=b'9' => Some((c - b'0') as u32),
            b'A'..=b'Z' => Some((c - b'A') as u32),
            _ => None,
        }
    }
    // Odd-position value, indexed by the even code (0 ≡ '0'/'A', … 9 ≡ '9'/'J').
    const ODD: [u32; 26] = [
        1, 0, 5, 7, 9, 13, 15, 17, 19, 21, 2, 4, 18, 20, 11, 3, 6, 8, 12, 14, 16, 10, 22, 25, 24,
        23,
    ];

    let bytes = matched.to_ascii_uppercase().into_bytes();
    if bytes.len() != 16 {
        return false;
    }
    let mut sum = 0u32;
    for (i, &c) in bytes[..15].iter().enumerate() {
        let Some(even) = even_val(c) else {
            return false;
        };
        // Char index 0 is 1-indexed position 1 → odd.
        sum += if i % 2 == 0 { ODD[even as usize] } else { even };
    }
    bytes[15] == b'A' + (sum % 26) as u8
}

/// A nine-digit national ID: NL BSN (11-proef) or PT NIF (mod-11).
fn nine_digit_id_valid(matched: &str) -> bool {
    nl_bsn_valid(matched) || pt_nif_valid(matched)
}

/// Dutch BSN 11-proef: Σ dᵢ·wᵢ ≡ 0 (mod 11), weights 9,8,…,2,−1; nonzero.
fn nl_bsn_valid(matched: &str) -> bool {
    let d: Vec<i32> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as i32)
        .collect();
    if d.len() != 9 {
        return false;
    }
    let weights = [9, 8, 7, 6, 5, 4, 3, 2, -1];
    let sum: i32 = d.iter().zip(weights).map(|(x, w)| x * w).sum();
    sum != 0 && sum % 11 == 0
}

/// Portuguese NIF mod-11 control digit (weights 9..2; control ≥10 → 0).
fn pt_nif_valid(matched: &str) -> bool {
    let d: Vec<u32> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as u32)
        .collect();
    if d.len() != 9 {
        return false;
    }
    let sum: u32 = (0..8).map(|i| d[i] * (9 - i as u32)).sum();
    let r = sum % 11;
    let control = if r < 2 { 0 } else { 11 - r };
    control == d[8]
}

/// An eleven-digit national ID: DE Steuer-ID (ISO 7064 Mod 11,10) or LV code.
fn eleven_digit_id_valid(matched: &str) -> bool {
    de_steuerid_valid(matched) || lv_code_valid(matched)
}

/// German Steuer-IdNr: first digit nonzero, exactly one digit value repeated (2–3×)
/// among the first 10 (the structural rule), and the ISO 7064 Mod 11,10 check digit.
/// Per the 2016+ spec (M4-R8), a digit that appears three times must **not** occupy
/// three *consecutive* positions.
fn de_steuerid_valid(matched: &str) -> bool {
    let d: Vec<u32> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as u32)
        .collect();
    if d.len() != 11 || d[0] == 0 {
        return false;
    }
    let mut counts = [0u8; 10];
    for &x in &d[..10] {
        counts[x as usize] += 1;
    }
    let repeated = counts.iter().filter(|&&c| c >= 2).count();
    let max = *counts.iter().max().unwrap();
    if repeated != 1 || max > 3 {
        return false;
    }
    // 2016+ rule: a 3× digit must not sit in three consecutive positions.
    if max == 3 {
        let triple = counts.iter().position(|&c| c == 3).unwrap() as u32;
        if d[..10].windows(3).any(|w| w == [triple, triple, triple]) {
            return false;
        }
    }
    let mut product = 10u32;
    for &x in &d[..10] {
        let mut sum = (x + product) % 10;
        if sum == 0 {
            sum = 10;
        }
        product = (sum * 2) % 11;
    }
    (11 - product) % 10 == d[10]
}

/// Latvian personal code: the post-2017 randomized form starts with `32` and has no
/// checksum (shape-only); the classic form carries a mod-11 check digit.
fn lv_code_valid(matched: &str) -> bool {
    let d: Vec<i64> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as i64)
        .collect();
    if d.len() != 11 {
        return false;
    }
    if d[0] == 3 && d[1] == 2 {
        return true; // randomized form — no checksum to verify
    }
    let w = [1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
    let sum: i64 = (0..10).map(|i| d[i] * w[i]).sum();
    let check = (1 - sum).rem_euclid(11).rem_euclid(10);
    check == d[10]
}

/// China Resident Identity Card, ISO 7064 MOD 11-2: 17 weighted digits → a check
/// char (a digit or `X`). 18 chars make this near-zero false-positive.
fn zh_resident_id_valid(matched: &str) -> bool {
    let chars: Vec<char> = matched.chars().collect();
    if chars.len() != 18 {
        return false;
    }
    const WEIGHTS: [u32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
    let mut sum = 0u32;
    for (i, c) in chars[..17].iter().enumerate() {
        match c.to_digit(10) {
            Some(v) => sum += v * WEIGHTS[i],
            None => return false,
        }
    }
    const TABLE: [char; 11] = ['1', '0', 'X', '9', '8', '7', '6', '5', '4', '3', '2'];
    TABLE[(sum % 11) as usize] == chars[17].to_ascii_uppercase()
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
    sum.is_multiple_of(10)
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
    fn french_nir_special_month_is_not_missed() {
        // M4-R5: INSEE special month `20` (born abroad / unknown) must still match.
        let nir = fr_nir("1852075116001");
        assert!(
            !kinds(&format!("NIR {nir}")).is_empty(),
            "special-month NIR must not be missed"
        );
    }

    #[test]
    fn italian_codice_fiscale_checksum_rejects_broken() {
        // M4-R3: the valid CF masks; flipping its check character rejects it.
        assert!(cf_check_valid("RSSMRA85T10A562S"));
        assert!(!cf_check_valid("RSSMRA85T10A562A"));
        assert!(kinds("cf RSSMRA85T10A562A").is_empty());
    }

    #[test]
    fn german_steuer_id_check() {
        assert!(de_steuerid_valid("86095742719"));
        assert!(!de_steuerid_valid("86095742718")); // wrong check digit
        assert!(!de_steuerid_valid("12345678901")); // no repeated digit (structural)
        assert_eq!(
            kinds("StId 86095742719"),
            vec![(PiiKind::NationalId, "86095742719".to_string())]
        );
    }

    #[test]
    fn de_steuerid_rejects_a_consecutive_triple() {
        // M4-R8: the 2016+ structural rule — a digit repeated three times must not
        // sit in three *consecutive* positions. Both numbers below carry a correct
        // Mod 11,10 check digit and exactly one repeated digit; only placement
        // differs, so this isolates the consecutive-triple rule (not the checksum).
        fn with_check(first10: &str) -> String {
            let d: Vec<u32> = first10.bytes().map(|b| (b - b'0') as u32).collect();
            let mut product = 10u32;
            for &x in &d {
                let mut sum = (x + product) % 10;
                if sum == 0 {
                    sum = 10;
                }
                product = (sum * 2) % 11;
            }
            format!("{first10}{}", (11 - product) % 10)
        }
        // Three consecutive `1`s → rejected despite the valid checksum.
        assert!(!de_steuerid_valid(&with_check("1112345678")));
        // The same digit three times but *not* consecutive → still accepted (proves
        // we reject the placement, not every triple).
        assert!(de_steuerid_valid(&with_check("1121345678")));
    }

    #[test]
    fn dutch_bsn_and_portuguese_nif_check() {
        assert!(nl_bsn_valid("111222333"));
        assert!(pt_nif_valid("123456789"));
        assert!(!nine_digit_id_valid("111222334")); // neither checksum
        assert_eq!(
            kinds("bsn 111222333"),
            vec![(PiiKind::NationalId, "111222333".to_string())]
        );
    }

    #[test]
    fn latvian_code_random_form_and_reject() {
        assert!(lv_code_valid("32012345678")); // post-2017 randomized (shape-only)
        assert!(!eleven_digit_id_valid("00000000000")); // neither DE nor LV
        assert_eq!(
            kinds("kods 32012345678"),
            vec![(PiiKind::NationalId, "32012345678".to_string())]
        );
    }

    #[test]
    fn numeric_email_local_part_is_not_hijacked_by_a_national_id() {
        // `123456789` is a valid PT NIF, but as an email local part the whole email
        // must win — Email outranks the numeric national IDs (no fragmentation).
        assert_eq!(
            kinds("write to 123456789@example.com now"),
            vec![(PiiKind::Email, "123456789@example.com".to_string())]
        );
    }

    #[test]
    fn email_beats_a_card_iban_or_secret_local_part() {
        // M4-R7: an `@`-token that parses as an email *is* an email, so a card /
        // IBAN / secret that is only a *substring* of its local part must not
        // fragment it (which would forward the `@domain` in clear). Email now
        // outranks Card/Iban/Secret too — generalizing the Email>national-ID fix.
        assert_eq!(
            kinds("card 4111111111111111@example.com"),
            vec![(PiiKind::Email, "4111111111111111@example.com".to_string())]
        );
        assert_eq!(
            kinds("iban DE89370400440532013000@example.com"),
            vec![(
                PiiKind::Email,
                "DE89370400440532013000@example.com".to_string()
            )]
        );
        assert_eq!(
            kinds("key sk-abcdef123456@example.com"),
            vec![(PiiKind::Email, "sk-abcdef123456@example.com".to_string())]
        );
    }

    #[test]
    fn grouped_forms_attached_to_a_domain_do_not_leak() {
        // M4-R9 — the counterpart to the containment case above. An email local part
        // cannot contain spaces, so against a *grouped* card / IBAN / NINO the email
        // regex grabs only the trailing group + `@domain` (`1111@example.com`) and
        // merely PARTIALLY overlaps the structured span. Neither side may be dropped
        // (M4-R10/R11): the two merge into their **union**, labelled with the
        // higher-priority kind — so the card/IBAN/NINO *and* the trailing email are
        // masked by one placeholder, and nothing is left in clear.
        assert_eq!(
            kinds("card 4111 1111 1111 1111@example.com"),
            vec![(
                PiiKind::CreditCard,
                "4111 1111 1111 1111@example.com".to_string()
            )],
            "a grouped card + the overlapping email must merge, not drop either side"
        );
        assert_eq!(
            kinds("iban DE89 3704 0044 0532 0130 00@example.com"),
            vec![(
                PiiKind::Iban,
                "DE89 3704 0044 0532 0130 00@example.com".to_string()
            )],
            "a grouped IBAN + the overlapping email must merge"
        );
        assert_eq!(
            kinds("nino AB 12 34 56 C@example.com"),
            vec![(PiiKind::NationalId, "AB 12 34 56 C@example.com".to_string())],
            "a spaced NINO + the overlapping email must merge"
        );
    }

    #[test]
    fn a_partially_overlapping_email_is_never_abandoned() {
        // M4-R11: with `Email` the lowest structured tier, a structured span that only
        // *partially* overlaps a REAL email used to win and drop the whole email —
        // leaving its **local part** (a person's name/handle) plus domain in clear.
        // The union-merge masks both. A left-over bare `@domain` would be acceptable;
        // a left-over local part is not.
        let got = kinds("call 555 867 5309john.doe@example.com now");
        assert_eq!(
            got,
            vec![(
                PiiKind::Phone,
                "555 867 5309john.doe@example.com".to_string()
            )],
            "the email's local part must not be abandoned in clear"
        );

        let got = kinds("tel +39 333 1234567.mario.rossi@example.com");
        assert_eq!(
            got.len(),
            1,
            "phone + email must merge into one span: {got:?}"
        );
        assert!(
            got[0].1.contains("mario.rossi@example.com"),
            "the email must be inside the masked span: {got:?}"
        );
    }

    #[test]
    fn a_span_enclosed_by_an_email_is_never_stranded() {
        // M4-R10: the card sits inside the email's local part, and the phone partially
        // overlaps that email. An earlier revision *deleted* the enclosed card up front;
        // the email then lost on priority and was dropped too, so the already-deleted card
        // was forwarded IN CLEAR. Nothing is deleted now — the enclosed span merges into
        // its enclosing email — so its bytes always stay covered.
        let got = kinds("call 555 867 5309.4111111111111111@x.com");
        assert_eq!(got.len(), 1, "expected one merged span, got {got:?}");
        assert!(
            got[0].1.contains("4111111111111111"),
            "the contained card must be covered by the merged span: {got:?}"
        );

        let got = kinds("call 555 867 5309.sk-abcdef123456@x.com");
        assert_eq!(got.len(), 1, "expected one merged span, got {got:?}");
        assert!(
            got[0].1.contains("sk-abcdef123456"),
            "the contained secret must be covered by the merged span: {got:?}"
        );
        // M4-R15: and it must be NAMED by the secret — the highest-priority raw candidate
        // the union covers — not by the phone that happened to survive. Otherwise the
        // model is told `[PHONE_1]` stands for a phone when it stands for a secret blob,
        // and the kind-only audit log under-reports a Secret as a Phone.
        assert_eq!(
            got[0].0,
            PiiKind::Secret,
            "the union must be named by the enclosed Secret: {got:?}"
        );
    }

    #[test]
    fn a_value_hidden_behind_an_earlier_match_is_still_a_candidate() {
        // M4-R17: `find_iter` is leftmost-NON-OVERLAPPING, so a real value that starts
        // inside an earlier match of the SAME recognizer was never emitted as a candidate —
        // and the resolver's invariant was then *vacuously* satisfied for it. Here the
        // shifted window `6789-4111 1111 1111` is Luhn-valid and matched first, hiding the
        // real trailing card that starts inside it; its last group ` 1111` was left in clear
        // (masked output was `[CARD_1]@[CARD_2] 1111`). Scanning from match.start()+1 finds
        // the hidden card, which then merges into the union.
        let input = "4111 1111 1111 1111@123-45-6789-4111 1111 1111 1111";
        let detector = StructuredRecognizers::new();
        let raw = detector.raw_candidates(input);
        assert!(
            raw.iter()
                .any(|c| c.kind == PiiKind::CreditCard && c.span.end == input.len()),
            "the card hidden behind the earlier match must reach the candidate set: {raw:?}"
        );

        let mut vault = crate::pii::anonymizer::Vault::new();
        let masked = vault.mask_all(input, &detector).unwrap();
        assert!(
            !masked.contains("1111"),
            "a card digit group was left in clear: {masked}"
        );
        assert_eq!(vault.demask(&masked), input, "round-trip must stay exact");
    }

    #[test]
    fn masking_a_phone_must_not_expose_a_card() {
        // M4-R17 (found by PROP-04). A card glued straight onto a phone forms ONE 19-digit
        // run, which is not Luhn-valid — so the card is correctly NOT a candidate (an ID
        // never fires inside a longer token). But masking the phone SPLITS the run and
        // leaves `4111111111111111` standing alone: a clean, Luhn-valid card that would go
        // upstream in clear. Masking must therefore re-detect to a fixpoint.
        let input = "4111111111111111555 867 5309";
        let detector = StructuredRecognizers::new();
        let mut vault = crate::pii::anonymizer::Vault::new();
        let masked = vault.mask_all(input, &detector).unwrap();

        assert!(
            !masked.contains("4111111111111111"),
            "masking the phone exposed the card in clear: {masked}"
        );
        assert!(
            detector.detect(&masked).is_empty(),
            "PII survived: {masked}"
        );
        assert_eq!(vault.demask(&masked), input, "round-trip must stay exact");
    }

    #[test]
    fn an_email_chain_leaves_nothing_detectable() {
        // M4-R19. Email and Secret are the two UNBOUNDED patterns, so they don't get the
        // M4-R17 overlap rescan (it would be O(n²) — an unauthenticated DoS). This pins the
        // one shape where that rescan used to add coverage: a *chained* `a@b.com@c.com`,
        // whose second email starts inside the first one's DOMAIN and reaches past its end.
        //
        // The fixpoint covers it instead: pass 1 masks the leftmost email, and whatever
        // `local@domain` the rewrite leaves behind is detected on pass 2. So the two
        // mechanisms are complementary — bounded recognizers rescan, unbounded ones iterate.
        let detector = StructuredRecognizers::new();
        for input in [
            "a@b.com@c.com",
            "a@b.com+x@c.com",
            "x@y.com@z.com@w.com",
            "sk-abcdefsk-123456@x.com",
        ] {
            let mut vault = crate::pii::anonymizer::Vault::new();
            let masked = vault.mask_all(input, &detector).unwrap();
            assert!(
                detector.detect(&masked).is_empty(),
                "PII survived masking of {input:?} → {masked:?}"
            );
            assert_eq!(vault.demask(&masked), input, "round-trip must stay exact");
        }

        // A left-over bare `@domain` is explicitly NOT PII (M4-R11) — but the local part,
        // which is the identifying half, must be gone.
        let mut vault = crate::pii::anonymizer::Vault::new();
        let masked = vault.mask_all("a@b.com@c.com", &detector).unwrap();
        assert!(
            !masked.contains("a@b.com"),
            "the email must be masked: {masked}"
        );
    }

    #[test]
    fn a_secret_hidden_inside_a_secret_is_still_covered() {
        // M4-R19's other half. Dropping Secret's overlap rescan is only safe because a
        // nested `sk-…` is *contained* in the outer one (both run greedily to the end of the
        // same maximal `[A-Za-z0-9_-]` run), so it adds no bytes. Pin that: the inner secret
        // is inside the masked span, and nothing survives in clear.
        let detector = StructuredRecognizers::new();
        let input = "key sk-abcdef-sk-ghijkl123 end";
        let mut vault = crate::pii::anonymizer::Vault::new();
        let masked = vault.mask_all(input, &detector).unwrap();

        assert!(
            !masked.contains("sk-"),
            "no secret fragment may survive: {masked}"
        );
        assert!(
            detector.detect(&masked).is_empty(),
            "PII survived: {masked}"
        );
        assert_eq!(vault.demask(&masked), input, "round-trip must stay exact");
    }

    #[test]
    fn cjk_prose_does_not_hide_structured_pii() {
        // M4-R13: Rust regex's default `\b` is Unicode-aware, so a Han character is a word
        // character and there is NO boundary between it and a digit. CJK has no inter-word
        // spaces, so this is the natural way to write it — and every anchored recognizer
        // was inert. `(?-u:\b)` (ASCII boundaries) fixes it.
        assert_eq!(
            kinds("我的信用卡号是4111111111111111"),
            vec![(PiiKind::CreditCard, "4111111111111111".to_string())]
        );
        assert_eq!(
            kinds("我的身份证号是11010519491231002X"),
            vec![(PiiKind::NationalId, "11010519491231002X".to_string())],
            "the zh Resident ID pack shipped in M4 must actually fire in Chinese"
        );
        assert_eq!(
            kinds("密钥sk-abcdef123456"),
            vec![(PiiKind::Secret, "sk-abcdef123456".to_string())]
        );
        // The anti-false-positive guarantee still holds inside a longer ASCII token.
        assert!(kinds("订单号是card4111111111111111").is_empty());
        assert!(kinds("哈希值abc4111111111111111abc").is_empty());
    }

    #[test]
    fn bare_numeric_national_ids_are_masked_by_design() {
        // M4-R6: the always-on 9-/11-digit recognizers gate on a real checksum, but
        // a checksum alone still accepts ~1/11 of arbitrary numbers per scheme, so
        // an ordinary standalone number that happens to pass is masked *on purpose*
        // — privacy-first (over-mask, never leak). This pins that as intended, not a
        // regression; the checksum still filters the majority that don't pass.
        assert!(pt_nif_valid("524287244")); // an arbitrary number that checks out
        assert_eq!(
            kinds("order 524287244 shipped"),
            vec![(PiiKind::NationalId, "524287244".to_string())]
        );
        // A neighbouring 9-digit that fails both checksums is left in clear — the
        // recognizer is not a blanket "mask every number".
        assert!(!nine_digit_id_valid("524287245"));
        assert!(kinds("order 524287245 shipped").is_empty());
    }

    #[test]
    fn china_resident_id_check() {
        assert!(zh_resident_id_valid("11010519491231002X"));
        assert!(!zh_resident_id_valid("11010519491231002Y")); // wrong check char
        assert_eq!(
            kinds("id 11010519491231002X"),
            vec![(PiiKind::NationalId, "11010519491231002X".to_string())]
        );
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
        assert!(
            kinds("ref GB123456A").is_empty(),
            "GB is an invalid prefix pair"
        );
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

    #[test]
    fn iban_per_country_length_gates_confidence() {
        // Correct country length…
        assert!(iban_length_ok("IT60X0542811101000000123456")); // 27, IT
        assert!(iban_length_ok("DE89370400440532013000")); // 22, DE
                                                           // …wrong length for a known country → not length-ok (masked, but Structural).
        assert!(!iban_length_ok("DE8937040044053201300")); // 21
                                                           // Unknown country → not penalized (rely on mod-97 alone).
        assert!(iban_length_ok("ZZ0012345678"));
        // A real, correctly-sized, mod-97-valid IBAN stays Verified.
        let e = StructuredRecognizers::new().detect("IBAN IT60X0542811101000000123456");
        assert_eq!(e[0].confidence, Confidence::Verified);
    }

    /// Real PII values, deliberately including the **grouped** shapes (spaces inside the
    /// value) that make a recognizer *partially* overlap an email instead of sitting
    /// inside it — the pathology behind M4-R7 / M4-R9 / M4-R10 / M4-R11.
    const PII_SAMPLES: &[&str] = &[
        "john.doe@example.com",
        "555 867 5309",
        "+39 333 1234567",
        "4111111111111111",
        "4111 1111 1111 1111",
        "DE89 3704 0044 0532 0130 00",
        "IT60X0542811101000000123456",
        "AB 12 34 56 C",
        "AB123456C",
        "123-45-6789",
        "sk-abcdef123456",
        "RSSMRA85T10A562S",
        "123456789",
        // M4-R16: values already embedded in a non-ASCII context, so the invariant is
        // exercised on multi-byte input even when the glue happens to be ASCII.
        "我的身份证号是11010519491231002X",
        "カード番号は4111111111111111です",
    ];

    /// Separators, including the ones that glue two values into one token and so
    /// manufacture partial overlaps.
    ///
    /// **Non-ASCII glue is mandatory here (M4-R16).** These tables were ASCII-only, which is
    /// the *exact* blind spot that let M4-R13 (every anchored recognizer inert in CJK) live
    /// through four reviews — and it was sitting in the one test the whole no-abandoned-bytes
    /// guarantee rests on. Multi-byte glue is what exercises `widen_to_char_boundaries`, the
    /// union re-slice, and a union endpoint landing *inside* a multi-byte character.
    const GLUE: &[&str] = &[
        "",
        ".",
        "-",
        " ",
        "@",
        ", ",
        "x", // ASCII
        "我的信用卡号是",
        "です",
        "、",
        "，",
        "Карта",
        "café",
        "—", // multi-byte
    ];

    proptest::proptest! {
        /// **PROP-03 — the resolver's invariant (M4-R10 / M4-R11): no structured span's
        /// bytes are ever abandoned.**
        ///
        /// Glue PII values together in arbitrary orders and separators, then assert that
        /// every *raw* structured candidate (pre-resolution) is **fully covered** by some
        /// resolved span — i.e. every one of its bytes is replaced in the masked output,
        /// never forwarded in clear — and that the round-trip is still exact.
        ///
        /// This is the guard the earlier fixes lacked: M4-R7 and M4-R9 each passed their
        /// own hand-written case while silently abandoning the *other* side of a partial
        /// overlap. A per-byte invariant can't be satisfied by picking a winner.
        #[test]
        fn every_structured_candidate_byte_is_covered(
            picks in proptest::collection::vec(0usize..PII_SAMPLES.len(), 1..4),
            glues in proptest::collection::vec(0usize..GLUE.len(), 3),
        ) {
            let mut input = String::new();
            for (i, &pick) in picks.iter().enumerate() {
                if i > 0 {
                    input.push_str(GLUE[glues[i % glues.len()]]);
                }
                input.push_str(PII_SAMPLES[pick]);
            }

            let detector = StructuredRecognizers::new();
            let raw = detector.raw_candidates(&input);
            let resolved = detector.detect(&input);

            for candidate in &raw {
                let covered = resolved
                    .iter()
                    .any(|r| r.span.start <= candidate.span.start && candidate.span.end <= r.span.end);
                proptest::prop_assert!(
                    covered,
                    "{:?} at {:?} is left in clear in {:?} — resolved: {:?}",
                    candidate.kind, candidate.span, input, resolved
                );
            }

            // Masking the resolved set must still round-trip exactly (a merged union is
            // masked and restored verbatim).
            let mut vault = crate::pii::anonymizer::Vault::new();
            let masked = vault.mask(&input, &resolved);
            proptest::prop_assert_eq!(vault.demask(&masked), input);
        }

        /// **PROP-04 — nothing PII-shaped survives masking (M4-R17).**
        ///
        /// Re-run the detector on the **masked output**: it must find no structured entity.
        ///
        /// PROP-03 quantifies over the *candidate set*, so it is only ever as strong as that
        /// set — a value the recognizer never emitted (because `find_iter` is
        /// leftmost-non-overlapping and an earlier match hid it) satisfies PROP-03
        /// *vacuously*. This one quantifies over the **output bytes** instead, which no
        /// candidate-generation gap can hide, and so is the natural companion guard.
        #[test]
        fn masking_leaves_nothing_detectable(
            picks in proptest::collection::vec(0usize..PII_SAMPLES.len(), 1..4),
            glues in proptest::collection::vec(0usize..GLUE.len(), 3),
        ) {
            let mut input = String::new();
            for (i, &pick) in picks.iter().enumerate() {
                if i > 0 {
                    input.push_str(GLUE[glues[i % glues.len()]]);
                }
                input.push_str(PII_SAMPLES[pick]);
            }

            let detector = StructuredRecognizers::new();
            let mut vault = crate::pii::anonymizer::Vault::new();
            let masked = vault.mask_all(&input, &detector).unwrap();

            let leftovers = detector.detect(&masked);
            proptest::prop_assert!(
                leftovers.is_empty(),
                "structured PII survived masking of {:?} → {:?}: {:?}",
                input, masked, leftovers
            );
        }
    }
}

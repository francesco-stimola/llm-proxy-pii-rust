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

use phonenumber::country::Id;
use regex::Regex;

use super::overlap::resolve_overlaps;
use super::{Budget, Confidence, DetectError, PiiDetector, PiiEntity, PiiKind};

/// A recognizer's extra check on the matched text, charged against the request's [`Budget`].
///
/// A **boxed closure**, not a bare `fn` pointer (M10) — see [`Recognizer::validate`].
///
/// **The budget is a parameter rather than something the scan loop deducts (M10-R29).** It used to
/// be charged once per candidate in [`Recognizer::push_candidates`], which meant *every* validator
/// paid the phone tier's rate — including the nine always-on national-ID checksums, whose cost is
/// pure arithmetic on ≤ 18 bytes. Fifty thousand of those is five milliseconds of work, refused as
/// if it were half a second. Handing the budget to the validator instead lets each one charge what
/// it actually costs: the phone families spend one unit per `phonenumber::parse()`, and the
/// checksums spend nothing at all.
type Validator = Box<dyn Fn(&str, &Budget) -> bool + Send + Sync>;

/// A validator whose cost is negligible — a checksum over at most 18 bytes — adapted to the
/// [`Validator`] shape without charging the request's budget (M10-R29).
///
/// Every national-ID and card check goes through here. It is a named function rather than a closure
/// at each call site so that *not* charging is a visible, single decision rather than nine
/// independently-written `|s, _|`s that a tenth recognizer might silently join.
fn free<F: Fn(&str) -> bool + Send + Sync + 'static>(check: F) -> Validator {
    Box::new(move |matched: &str, _budget: &Budget| check(matched))
}

/// One compiled recognizer: a category, its pattern, an optional validator applied to
/// each raw match, and how the scan advances after one. Overlap priority comes from the
/// kind ([`PiiKind::priority`]).
struct Recognizer {
    kind: PiiKind,
    regex: Regex,
    /// Extra check on the matched text; `None` means "accept every match".
    ///
    /// A **boxed closure**, not a bare `fn` pointer (M10). The national-phone validator
    /// has to carry the *set of enabled regions*, which a `fn(&str) -> bool` cannot — that
    /// is why GB and DE each needed a hand-written wrapper before, and why "one recognizer
    /// per region" was the only shape a fn pointer allowed. Measured, that shape costs a
    /// full `Scan::Overlapping` pass per region (O(n·L) each) while doing byte-identical
    /// validation work: **strictly more work**, ~2× at nine regions. So the region loop
    /// moves *into* the validator and the scan count stays constant — see
    /// [`national_phone_recognizers`].
    validate: Option<Validator>,
    /// Whether this pattern's length is bounded — see [`Scan`].
    scan: Scan,
    /// Whether a **rejected** match should be retried one digit group shorter, until the
    /// validator accepts a prefix starting at the same byte — see
    /// [`shrink_to_a_valid_prefix`](Recognizer::shrink_to_a_valid_prefix).
    ///
    /// Only the phone recognizers set it. A checksum recognizer must not: a shorter prefix
    /// of a non-Luhn-valid digit run can be Luhn-valid by coincidence, and that would trade a
    /// documented, measured false-positive rate for an unmeasured one.
    shrink_on_reject: bool,
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
    /// Build the default recognizer set: the universal recognizers, the always-on
    /// national identifiers, and the domestic-phone recognizers for **every vetted
    /// region** ([`PHONE_REGIONS`]).
    ///
    /// **The default detects domestic numbers (M10).** Until M10 it did not: the default
    /// was the string `"it,us"`, chosen by [M4](../../../docs/ROADMAP.md#m4) when the
    /// FP-prone tier was *empty* and never revisited when
    /// [M8.1](../../../docs/ROADMAP.md#m81) filled it with `gb` and `de` — so the shipped
    /// configuration named two locales that mapped to no recognizer at all, and
    /// `06 69821234` went upstream in clear. "Everything off unless you knew to switch it
    /// on" is not a defensible default for a privacy tool.
    pub fn new() -> Self {
        Self::with_regions(&vetted_phone_regions())
    }

    /// Build with an explicit list of locale codes (M4, `PII_LOCALES`). Three tiers:
    ///
    /// - **Universal** — email, secret, credit card, IBAN (any country + mod-97),
    ///   phone (US + `+CC`) — always on.
    /// - **National identifiers** — US SSN, IT Codice Fiscale, GB NINO, ES DNI/NIE,
    ///   FR NIR — **always on regardless of `locales`** (M4-R1, privacy-first: a
    ///   national ID that reaches the proxy is masked even if its country isn't
    ///   configured). Each is specific enough (checksums / prefix rules) to stay
    ///   near-zero false-positive when always on.
    /// - **Domestic phone** — the FP-prone tier: numbers written with no `+CC`. Every
    ///   [vetted region](PHONE_REGIONS) is on by default; passing `locales` **replaces**
    ///   that set rather than adding to it, so an operator who set `PII_LOCALES` keeps
    ///   exactly the behaviour they asked for.
    ///
    /// A code outside the vetted set contributes nothing (it has no measured FP-rate, and
    /// an unmeasured region is not a region we ship). That is also what keeps the old
    /// `it,us` default honest if it is still in someone's environment: `us` has no
    /// domestic trunk form at all, and `it` now resolves to a real recognizer.
    pub fn with_locales<S: AsRef<str>>(locales: &[S]) -> Self {
        let regions: Vec<Id> = locales
            .iter()
            .filter_map(|code| phone_region_for_locale(code.as_ref()))
            .map(|region| region.id)
            .collect();
        Self::with_regions(&regions)
    }

    /// Build with an explicit region set for the domestic-phone tier. The seam the
    /// measurement harness and the coverage guard drive directly.
    pub fn with_regions(regions: &[Id]) -> Self {
        let pairs: Vec<(Id, &[PhoneShape])> = PHONE_REGIONS
            .iter()
            .filter(|r| regions.contains(&r.id))
            .map(|r| (r.id, r.shapes))
            .collect();
        Self::with_shapes(&pairs)
    }

    /// Build with an explicit **(region, shapes)** set — the seam a guard needs in order to
    /// ask "is this declared shape load-bearing?", which it cannot do while the shapes are
    /// welded to the region table.
    pub fn with_shapes(regions: &[(Id, &[PhoneShape])]) -> Self {
        let mut recognizers = universal_recognizers();
        recognizers.extend(national_id_recognizers());
        // Always on, beside the national IDs and for the same reason — never gated by
        // `locales` (M11 Track A, decision 2).
        recognizers.extend(vat_recognizers());
        recognizers.extend(national_phone_recognizers(regions));
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
            shrink_on_reject: false,
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
                r"(?-u:\b)[A-Za-z]{2}\d{2}(?:[A-Za-z0-9]{11,30}|(?: [A-Za-z0-9]{4}){2,7}(?: [A-Za-z0-9]{1,4})?)(?-u:\b)",
            )
            .unwrap(),
            validate: Some(free(iban_case_gate)),
            scan: Scan::Overlapping, // bounded: ≤ 44 chars
            shrink_on_reject: false,
        },
        // Credit cards: 13–19 digits, either grouped in 4s or continuous, gated by
        // the Luhn checksum to reject look-alikes.
        Recognizer {
            kind: PiiKind::CreditCard,
            regex: Regex::new(r"(?-u:\b)(?:\d{4}[ -]\d{4}[ -]\d{4}[ -]\d{4}|\d{13,19})(?-u:\b)").unwrap(),
            validate: Some(free(credit_card_valid)),
            scan: Scan::Overlapping, // bounded: ≤ 19 digits — and this is the M4-R17 repro
            shrink_on_reject: false,
        },
        Recognizer {
            kind: PiiKind::Email,
            regex: Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap(),
            validate: None,
            // Unbounded (`+` on both sides of the `@`) → no overlap rescan; the
            // mask-to-a-fixpoint pass catches a chained `a@b.com@c.com` remainder.
            scan: Scan::Sequential,
            shrink_on_reject: false,
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
            shrink_on_reject: false,
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
            shrink_on_reject: false,
        },
        // Italian Codice Fiscale: 6 letters, 2 digits, letter, 2 digits, letter,
        // 3 digits, letter (16 chars). The final letter is a checksum (M4-R3), so a
        // wrong-checksum look-alike is rejected — consistent with the other IDs.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)[A-Za-z]{6}\d{2}[A-Za-z]\d{2}[A-Za-z]\d{3}[A-Za-z](?-u:\b)").unwrap(),
            validate: Some(free(cf_check_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
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
            validate: Some(free(nino_prefix_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // Spanish DNI (8 digits) / NIE (X/Y/Z + 7 digits), each with a mod-23 check
        // letter that must match — so a random 8-digit+letter token won't pass.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)(?:[XYZxyz]\d{7}|\d{8})[A-Za-z](?-u:\b)").unwrap(),
            validate: Some(free(es_dni_nie_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // French NIR (social security): 15 digits — sex + YY + MM + geo/order + a
        // mod-97 key that must check out. The month admits INSEE special codes
        // (`20` unknown/born-abroad; `30–42`/`50–99` provisional SANDIA) so those
        // real NIRs aren't missed on the always-on tier (M4-R5). Corsica's `2A`/`2B`
        // department (letters in the body) is a documented gap — docs/reviews/M4.md#m4-r5.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)[12]\d{2}(?:0[1-9]|1[0-2]|20|3\d|4[0-2]|[5-9]\d)\d{10}(?-u:\b)").unwrap(),
            validate: Some(free(fr_nir_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // Nine-digit national IDs: NL BSN (11-proef) or PT NIF (mod-11). One
        // recognizer, either checksum accepts (both are 9 digits). Accepted FP
        // tradeoff (M4-R6): a mod-11 checksum still passes ~1/11 of arbitrary
        // 9-digit numbers per scheme (BSN ∪ NIF ≈ 2/11 ≈ 18%), so an ordinary
        // standalone 9-digit token that happens to check out is masked. That is the
        // privacy-first choice (over-mask, never leak) — context-gating it would
        // reintroduce leaks (M4-R1). The clean precision path is the contextual
        // GLiNER detector (M8), not a keyword gate.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{9}(?-u:\b)").unwrap(),
            validate: Some(free(nine_digit_id_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // Eleven-digit national IDs: DE Steuer-ID (ISO 7064 Mod 11,10 + one repeated
        // digit) or LV personal code (mod-11 / post-2017 `32…` random form). Same
        // accepted-FP tradeoff as the 9-digit recognizer above (M4-R6): DE ∪ LV
        // still masks a fraction of arbitrary 11-digit numbers (incl. the
        // unconditional LV `32…` ~1%) — privacy-first, over-mask never leak.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{11}(?-u:\b)").unwrap(),
            validate: Some(free(eleven_digit_id_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // LV personal code, classic dashed form `DDMMYY-NNNNC` (mod-11 checksum).
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{6}-\d{5}(?-u:\b)").unwrap(),
            validate: Some(free(lv_code_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // China Resident Identity Card: 17 digits + a check char (digit or X),
        // ISO 7064 MOD 11-2. 18 chars → near-zero false positives.
        Recognizer {
            kind: PiiKind::NationalId,
            regex: Regex::new(r"(?-u:\b)\d{17}[0-9Xx](?-u:\b)").unwrap(),
            validate: Some(free(zh_resident_id_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
    ]
}

/// Business **tax identifiers** — Italian Partita IVA and EU VAT numbers (M11 Track A).
/// **Always on**, like the national IDs they join and for the same reason (M4-R1): a VAT
/// number that reaches the proxy is masked whether or not its country is in `PII_LOCALES`.
/// No configuration variable gates this tier, deliberately — see ROADMAP → M11 Track A,
/// decision 2.
///
/// **Every pattern is checksum-backed and ≤ 14 chars**, so they all take the bounded
/// [`Scan::Overlapping`] rescan (M4-R19) and none of them sets `shrink_on_reject` — a shorter
/// prefix of a rejected digit run can satisfy a checksum by coincidence, which would trade a
/// measured false-positive rate for an unmeasured one.
///
/// ## Two shapes, and why only Italy gets the bare one
///
/// A **prefixed** form (`IT00905811006`, `DE136695976`) is the VIES canonical rendering and is
/// what every country here is recognized by. A **bare** form — the digits alone — ships for
/// **Italy only**, because a P.IVA is genuinely written that way in Italian text (`P.IVA
/// 00905811006`) and because that is the form the ROADMAP scoped. For the other countries the
/// bare digits are already claimed by the always-on national-ID tier and adding a second,
/// unprefixed claim on them would buy coverage nobody measured.
///
/// ## The country prefix must be UPPERCASE, and that is an anti-false-positive decision
///
/// Lowercase would make `it` a matchable prefix — and `it` is one of the commonest words in
/// English prose. `"call it 12345678901"` with a mod-10-valid tail would then produce a
/// **14-character** span swallowing an ordinary English word. VAT numbers are written
/// uppercase by convention and VIES requires it, so nothing real is lost. Pinned by
/// `lowercase_country_prefix_is_not_a_vat_number`.
///
/// ## No space between prefix and digits
///
/// `IT 00905811006` is not matched *as a prefixed VAT*; for Italy the bare recognizer catches
/// the digits anyway, and for the rest the always-on national-ID tier already covers most bare
/// forms. Allowing an optional space would put `DE`/`IT`/`GB` — all live English abbreviations —
/// one space away from a digit run, and buys a rendering that VIES itself does not use.
///
/// ## Which countries ship, and which deliberately do not
///
/// The `PHONE-NAT` rule applies unchanged: **a category ships when it is measured.** Five do —
/// 🇮🇹 🇩🇪 🇬🇧 🇳🇱 🇵🇹. Three do not, and are named rather than silently missing:
/// - **🇪🇸 ES** — the person forms reuse a check this repo already measures, but a Spanish
///   *company's* VAT is the CIF form, whose control character follows a different rule that is
///   not measured here. Shipping the person half alone would be a VAT recognizer that misses
///   the case it exists for.
/// - **🇫🇷 FR** — the two-character key over the SIREN could not be confirmed against a
///   trustworthy real pair, and an unverified checksum is exactly what this tier must not carry.
/// - **🇱🇻 LV** — the legal-entity VAT checksum is a different algorithm from the personal code
///   in [`lv_code_valid`], and it is not measured here.
fn vat_recognizers() -> Vec<Recognizer> {
    vec![
        // 🇮🇹 Partita IVA, bare — 11 digits, mod-10 with position doubling. The shape that
        // collides with **two** other always-on tiers, and the second one is the larger:
        //
        //   * `\d{11}` is the DE Steuer-ID / LV personal-code pattern. Measured overlap with
        //     valid P.IVAs: 0.0998 (VAT-10).
        //   * `\b0\d{6,11}\b` is the domestic-phone `Trunk` family's separator-free arm, so
        //     every **0-leading** P.IVA — which is to say every issuable one — is also a phone
        //     candidate in all nine vetted regions. Measured against the shipped default:
        //     **most of them are named `[PHONE_n]`** (VAT-17).
        //
        // In both cases both recognizers fire, the spans are identical, the resolver merges
        // them, and `PiiKind::priority` decides only which kind *names* the union — every byte
        // is masked either way (M4-R10/R11).
        //
        // **What this recognizer adds, stated correctly (M11-R2).** The line here used to read
        // "the collision costs no coverage", naming only the national-ID overlap. That is true
        // of *masking* and false of *labelling*: what reaches the model as `[TAXID_n]` is every
        // P.IVA that is not also a valid Steuer-ID, an LV code, **or an assigned domestic number
        // in one of the nine vetted regions** — and that last clause takes most of the issuable
        // 0-leading space with it. The five real published P.IVAs in this repo's corpus are all
        // `00…`-leading, which libphonenumber reads as an international access code and
        // rejects, so every existing guard sits inside the immune sub-shape. VAT-17 measures the
        // rest; the ordering itself is deliberate and argued on `PiiKind::priority`.
        Recognizer {
            kind: PiiKind::TaxId,
            regex: Regex::new(r"(?-u:\b)\d{11}(?-u:\b)").unwrap(),
            validate: Some(free(it_piva_valid)),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // 🇮🇹 Partita IVA, VIES form.
        Recognizer {
            kind: PiiKind::TaxId,
            regex: Regex::new(r"(?-u:\b)[Ii][Tt]\d{11}(?-u:\b)").unwrap(),
            validate: Some(free(|s| it_piva_valid(vat_body(s)))),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // 🇩🇪 USt-IdNr — `DE` + 9 digits, ISO 7064 Mod 11,10. Same arithmetic this file already
        // trusts for the Steuer-ID, minus that scheme's own "exactly one repeated digit" rule,
        // which is specific to the Steuer-ID and not part of the VAT spec.
        Recognizer {
            kind: PiiKind::TaxId,
            regex: Regex::new(r"(?-u:\b)[Dd][Ee]\d{9}(?-u:\b)").unwrap(),
            validate: Some(free(|s| de_vat_valid(vat_body(s)))),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // 🇬🇧 VAT — `GB` + 9 digits, mod-97 or mod-97-55 (the two eras of the scheme; a number
        // issued under either is live, so both are accepted). **Not a VIES number since Brexit**,
        // but GB is in the national-ID tier this track takes its country list from, and the
        // identifier is checksum-verifiable — which is the tier's actual criterion. The 12-digit
        // branch-trader form and the `GBGD`/`GBHA` government forms are documented gaps.
        Recognizer {
            kind: PiiKind::TaxId,
            regex: Regex::new(r"(?-u:\b)[Gg][Bb]\d{9}(?-u:\b)").unwrap(),
            validate: Some(free(|s| gb_vat_valid(vat_body(s)))),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // 🇵🇹 NIF/VAT — `PT` + 9 digits, the same mod-11 the national-ID tier already measures.
        // A Portuguese company's VAT number *is* its NIF, so this is one identifier under two
        // names, not two schemes.
        Recognizer {
            kind: PiiKind::TaxId,
            regex: Regex::new(r"(?-u:\b)[Pp][Tt]\d{9}(?-u:\b)").unwrap(),
            validate: Some(free(|s| pt_nif_valid(vat_body(s)))),
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
        // 🇳🇱 btw-id — `NL` + 9 digits + a literal `B` + 2 digits. **The one recognizer here whose
        // acceptance is format-only**, and it says so through [`Confidence`]: the 9 digits are an
        // RSIN (11-proef checkable) for a legal entity, but the 2020 sole-trader btw-id is
        // deliberately randomized and carries no checksum at all. So a mod-11 pass is `Verified`
        // and a fail is `Structural` — masked either way, exactly as an IBAN whose mod-97 fails
        // (M4). The format itself is the defence: 14 chars, a mandatory `NL`, and a literal `B`
        // pinned at position 11 is not a shape ordinary text produces.
        Recognizer {
            kind: PiiKind::TaxId,
            regex: Regex::new(r"(?-u:\b)[Nn][Ll]\d{9}[Bb]\d{2}(?-u:\b)").unwrap(),
            validate: None,
            scan: Scan::Overlapping,
            shrink_on_reject: false,
        },
    ]
}

/// How many tax/VAT recognizers this build ships — the length of [`vat_recognizers`].
///
/// Public for one reason: the over-mask guard's positive control (`VAT-16`) has to hold one
/// live control **per shipped scheme**, and a hand-kept list of six would silently stay six
/// when a seventh recognizer lands — leaving the new scheme unwatched by the guard whose whole
/// job is to prove the tier is switched on. That is M10-R7's defect verbatim, one tier over:
/// there, a control set covering one shape family out of three passed with the other two
/// deleted. Counting the set the recognizers are actually built from is the chokepoint; a list
/// somebody remembers to extend is not.
pub fn shipped_tax_recognizer_count() -> usize {
    vat_recognizers().len()
}

/// One region whose **domestic phone numbers** (written with no `+CC`) this build detects.
pub struct PhoneRegion {
    /// The `PII_LOCALES` code that selects it — ISO 3166-1 alpha-2.
    pub code: &'static str,
    /// The libphonenumber region the validator parses against.
    pub id: Id,
    /// The candidate **shapes this region's numbers are actually written in** — and the most
    /// important precision decision in M10, made by measurement rather than by symmetry.
    ///
    /// **Why a region must not see a shape it does not use.** libphonenumber's
    /// `parse(Some(region), …)` accepts a national number *with or without* the trunk prefix,
    /// because in a trunk-prefix country you really can dial a local number that way. So
    /// offering un-anchored digit groups to a trunk-prefix region asks *"could this be a
    /// same-area local dial in Berlin?"* — true of an enormous slice of ordinary numeric
    /// text. Measured on digit-shaped non-phones (ports, offsets, HTTP codes, byte sizes,
    /// IDs), with every region seeing every shape: **DE 7 hits of 24, FR 4, NL 3**, against 0
    /// for the regions that genuinely have no trunk prefix. Restricting them to
    /// [`Trunk`] took all three to 0 and cost no real rendering.
    ///
    /// **The same argument one level finer, and it is worth the extra column.** A single
    /// non-trunk flag still handed China the *prefix + long block* shape (`347 1234567`),
    /// which only Italian mobiles are written in — and China's plan accepts 10-digit runs
    /// starting `1…`, so file offsets and byte counts (`offset 100 1000000 in file`) became
    /// `Phone` spans: **0.250 of the offsets pool, 0.156 of sizes**. Declaring shapes per
    /// region takes those to 0 while Chinese mobiles (`138 0013 8000`, a *groups* shape) stay
    /// covered.
    ///
    /// Each entry is load-bearing: `every_declared_shape_is_needed_by_a_real_rendering` fails
    /// if a shape is listed that no rendering of that country requires.
    pub shapes: &'static [PhoneShape],
}

/// The regions this build ships — **on by default** (M10).
///
/// **The set is bounded by a principle, not by taste: exactly the countries the tool
/// already claims.** The ten national-ID packs plus the NER model's language set, which
/// between them are what "we cover this country" means everywhere else in this detector.
/// That answers "why not Belgium?" with the same rule that answers it for every other
/// layer, instead of a judgement call per country.
///
/// **US is deliberately absent and that is not a gap:** it has no trunk-`0` domestic form,
/// and the universal `NNN NNN NNNN` arm already covers it. `ar` gets nothing for the same
/// reason it gets no national-ID pack — the language spans ~20 countries with different
/// plans, so there is no single "Arabic" numbering plan to enable.
///
/// > **Rejected: defaulting to the *host's* locale.** It contradicts
/// > [M4-R1](../../../docs/reviews/M4.md#m4-r1), which made national IDs always-on
/// > *regardless of configuration* precisely because **what matters is the data that
/// > arrives, not where the proxy runs** — a proxy in a Frankfurt datacenter routinely
/// > carries Italian users' data. It would also make masking machine-dependent: same
/// > request, two boxes, two results, nothing in the logs to explain it.
///
/// A region cannot be added here without corpus cases: `phone_regions_all_have_corpus_cases`
/// (`tests/pii_corpus.rs`) enumerates this table and fails if any row has none.
/// One-line-per-region on purpose — rustfmt would give each field its own line and turn nine
/// scannable rows into forty-five.
#[rustfmt::skip]
pub const PHONE_REGIONS: &[PhoneRegion] = &[
    // `030 12345678`, `0171 1234567` — every German number carries the trunk 0.
    PhoneRegion { code: "de", id: Id::DE, shapes: &[Trunk] },
    // `91 123 45 67`, `912 345 678`, `612 34 56 78` — no trunk prefix at all.
    PhoneRegion { code: "es", id: Id::ES, shapes: &[Groups] },
    // `01 23 45 67 89` is the standard rendering; `0123456789` the compact one.
    PhoneRegion { code: "fr", id: Id::FR, shapes: &[Trunk, TrunkPairs] },
    // `020 7946 0958`, `07911 123456`, `0800 1111` — trunk 0 throughout.
    PhoneRegion { code: "gb", id: Id::GB, shapes: &[Trunk] },
    // The only region needing all three: landlines keep their leading 0 (`011 5627111`),
    // mobiles have none and are written prefix + block (`347 1234567`) or in groups
    // (`320 123 4567`).
    PhoneRegion { code: "it", id: Id::IT, shapes: &[Trunk, Groups, LongBlock] },
    // `67 22 33 44`, `67 123 456` — eight digits, no trunk prefix.
    PhoneRegion { code: "lv", id: Id::LV, shapes: &[Groups] },
    // `020 123 4567`, `0343 123456`, `06 12345678`.
    PhoneRegion { code: "nl", id: Id::NL, shapes: &[Trunk] },
    // `21 123 4567`, `210 123 456`, `912 345 678` — nine digits, no trunk prefix.
    PhoneRegion { code: "pt", id: Id::PT, shapes: &[Groups] },
    // Landlines carry the trunk 0 (`010 12345678`); mobiles are written in groups
    // (`138 0013 8000`). **Deliberately NOT `LongBlock`** — that shape is Italian-mobile
    // only, and China's plan accepts 10-digit runs starting `1…`, so offering it here turned
    // file offsets and byte counts into `Phone` spans (offsets 0.250, sizes 0.156 of the
    // measured pools). Chinese mobiles are unaffected: they are a `Groups` shape.
    PhoneRegion { code: "cn", id: Id::CN, shapes: &[Trunk, Groups] },
];

/// Every vetted region — the default domestic-phone set.
pub fn vetted_phone_regions() -> Vec<Id> {
    PHONE_REGIONS.iter().map(|r| r.id).collect()
}

/// Map a `PII_LOCALES` code to a vetted phone region, or `None` if we do not ship that
/// region (an unmeasured region is not a region we ship).
///
/// Accepts the ISO 3166-1 alpha-2 code and the one language alias this project already
/// uses for a country elsewhere (`zh` for China, as in the NER's language set). `uk` is
/// **not** accepted for the United Kingdom — it is Ukraine's ISO code, and quietly reading
/// it as GB is the kind of guess this detector does not make.
fn phone_region_for_locale(code: &str) -> Option<&'static PhoneRegion> {
    let code = code.trim().to_ascii_lowercase();
    let code = if code == "zh" { "cn" } else { code.as_str() };
    PHONE_REGIONS.iter().find(|r| r.code == code)
}

/// Which candidate **shape family** a pattern belongs to — and, therefore, which regions may
/// validate its candidates ([`PhoneRegion::shapes`]).
///
/// A shape is a *rendering*, not a country: several countries write numbers the same way, and
/// one country (Italy) writes them three ways. Keeping the two apart is what lets a region be
/// added without widening every other region's candidate set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhoneShape {
    /// Leading `0`, compact or 2–3 groups — `020 7946 0958`, `030 12345678`, `011 5627111`.
    /// The family M8.1 measured (GB precision 1.000, DE 0.909).
    Trunk,
    /// Leading `0`, five 2-digit pairs — the standard French rendering, `01 23 45 67 89`.
    TrunkPairs,
    /// No trunk prefix, 2–4 groups — `91 123 45 67`, `912 345 678`, `138 0013 8000`.
    Groups,
    /// No trunk prefix, a 3-digit prefix and one 6–8-digit block — `347 1234567`. Italian
    /// mobiles are written this way; nothing else in the shipped set is.
    LongBlock,
}

use PhoneShape::{Groups, LongBlock, Trunk, TrunkPairs};

/// The candidate shape families, and the whole reason this tier can grow past GB + DE.
///
/// Before M10 there was one family and it required a leading `0` — so a country that
/// dropped the trunk prefix proposed **no candidate at all**, and adding it to the enabled
/// set would have been a silent no-op. Generalizing the anchor is the actual work.
///
/// **Every family's match length must stay bounded** — [`Scan::Overlapping`] costs
/// O(n · L), so an open `(?:[ -]?\d)+` turns the masking path back into the M4-R19
/// complexity DoS. The longest match any family below can produce is ~18 chars. Do not add
/// an unbounded arm; `tests/complexity.rs` is the guard.
///
/// Each family is one [`Recognizer`], and the *validator* loops the enabled regions — so
/// the number of scans over a field is fixed at this table's length no matter how many
/// regions are on.
const PHONE_SHAPES: &[(PhoneShape, &str)] = &[
    // Trunk `0`, compact or 2/3 groups — GB `020 7946 0958`, DE `030 12345678`,
    // IT `011 5627111`, NL `020 123 4567`, CN `010 12345678`. **Unchanged from M8.1**:
    // GB and DE must keep exactly the behaviour that milestone measured.
    (
        PhoneShape::Trunk,
        r"(?-u:\b)0\d{1,4}[ -]\d{3,4}[ -]\d{3,4}(?-u:\b)|(?-u:\b)0\d{1,4}[ -]\d{4,8}(?-u:\b)|(?-u:\b)0\d{6,11}(?-u:\b)",
    ),
    // Trunk `0`, five 2-digit pairs — the standard French rendering (`01 23 45 67 89`,
    // also written with `.` or `-`). No arm above matches it: they top out at three groups.
    (
        PhoneShape::TrunkPairs,
        r"(?-u:\b)0\d[ .-]\d{2}[ .-]\d{2}[ .-]\d{2}[ .-]\d{2}(?-u:\b)",
    ),
    // **No trunk prefix**, 2–4 groups — ES `91 123 45 67`, PT `912 345 678`,
    // LV `67 22 33 44`, CN mobiles `138 0013 8000`. The leading digit is non-zero: a `0`
    // start belongs to the trunk families, and letting both match the same text would only
    // double the candidate set.
    //
    // **Separators are required, and that is a deliberate precision/recall trade.** A bare
    // compact run (`3471234567`) is indistinguishable from an order number, a unix
    // timestamp or a numeric ID — and bare 9- and 11-digit runs are *already* over-masked
    // by the national-ID tier under the M4-R6 tradeoff. Requiring grouping costs the
    // compact rendering of a non-trunk mobile (a documented recall gap, DEVLOG M10) and
    // buys back the FP rate that made this family shippable on by default at all.
    //
    // **The first group stays 2–3 digits, and that too was measured.** Widening it to 4
    // makes a 4-digit-prefixed pair (`2010 2020`, `2048 4096`, `2000 3000`) an 8-digit
    // candidate, and Latvia's plan — 8 digits, `2…` mobile / `6…` landline — accepts
    // essentially all of them: four new false positives out of the same 24-case negative
    // set, for the one Latvian rendering (`6712 3456`) it buys. LV is still covered by its
    // `67 22 33 44` and `67 123 456` forms.
    (
        PhoneShape::Groups,
        r"(?-u:\b)[1-9]\d{1,2}(?:[ -]\d{2,4}){1,3}(?-u:\b)",
    ),
    // **No trunk prefix**, prefix + one long block — the usual Italian mobile rendering
    // (`347 1234567`), which the arm above cannot reach: its groups top out at 4 digits.
    (
        PhoneShape::LongBlock,
        r"(?-u:\b)[1-9]\d{2}[ -]\d{6,8}(?-u:\b)",
    ),
];

/// The domestic-phone recognizers for a set of regions — **one per shape family**, each
/// validated against the enabled regions that family applies to (M10, shape (b)).
///
/// **Why the region loop lives in the validator.** The alternative — one recognizer per
/// region — needs no type change, and it is *strictly more work*: the two do byte-identical
/// validation, and the per-region shape additionally scans the text N−1 more times at
/// O(n · L) each. Measured on a 22 KiB realistic turn with 9 regions (release profile):
/// **12.28 ms** per-region vs **6.02 ms** shared-regex-validate-all vs **2.40 ms** shared
/// regex with `.any()`'s early exit — 5.12× overall, and the 2.04× between the first two is
/// purely the extra scans. That term grows linearly with every region added, on the
/// deterministic path that is today the *fast* one (~20 ms for a whole turn).
///
/// **Region granularity is not traded away**: regions are still enabled individually and
/// still bounded by [`PHONE_REGIONS`]. Only the dispatch changed.
///
/// **What makes an un-anchored digit run safe to propose at all** is the validator, not the
/// regex: [`national_phone_valid`] accepts a candidate only if `phonenumber`
/// (libphonenumber) deems it a REAL, **assigned** number for some enabled region — a check
/// against the country's actual numbering plan that no hand-written regex can do. This is
/// the [M4-R1](../../../docs/reviews/M4.md#m4-r1) "national phone with no `+CC` is FP-prone"
/// objection, defused by measurement rather than argued away (M8.1: GB precision 1.000,
/// DE 0.909).
///
/// A shape family's match can still cross the boundary between two **adjacent** numbers —
/// the trunk family's greedy `\d{3,4}` can take the *trunk of the next* number
/// (`0800 1111 0800`), an over-long span `is_valid` then rejects, which shadows the first
/// number's shorter valid form within a single [`detect`](PiiDetector::detect) pass (the
/// overlapping rescan resumes *forward* of the rejected match, so `find_at` cannot return
/// the shorter one). **That costs no coverage on the request path:** masking runs through
/// [`Vault::mask_all`](crate::pii::anonymizer::Vault::mask_all), whose **fixpoint**
/// re-detects — masking the second number un-shadows the first, and the next pass masks it
/// (M8-R8, pinned by a `mask_all`-level test). So these recognizers must **never** override
/// `redetect` to skip later passes. Every arm uses ASCII `(?-u:\b)` (M4-R13), so a digit run
/// inside a longer ASCII token (`user0207946095@…`) is not a candidate.
fn national_phone_recognizers(regions: &[(Id, &[PhoneShape])]) -> Vec<Recognizer> {
    PHONE_SHAPES
        .iter()
        .filter_map(|(shape, pattern)| {
            // **Only the regions that are actually written in this shape.** A region seeing a
            // rendering its numbers never take adds no coverage and real false positives —
            // see [`PhoneRegion::shapes`] for the measurement that made this per-shape rather
            // than one coarse trunk/non-trunk flag.
            //
            // Deduplicated so the same set costs the same and short-circuits the same
            // however it was written (`PII_LOCALES=it,it`).
            let mut ids: Vec<Id> = Vec::new();
            for (id, shapes) in regions {
                if shapes.contains(shape) && !ids.contains(id) {
                    ids.push(*id);
                }
            }
            let applicable: std::sync::Arc<[Id]> = ids.into();
            // No enabled region uses this family — don't scan for it at all.
            if applicable.is_empty() {
                return None;
            }
            Some(Recognizer {
                kind: PiiKind::Phone,
                regex: Regex::new(pattern).unwrap(),
                validate: Some(Box::new(move |matched: &str, budget: &Budget| {
                    // **No cheap pre-filter here, and that is a decision with a measurement
                    // behind it (M10-R13).** A digit-count gate derived from libphonenumber's
                    // `possible_length` metadata was added to make rejection cheap — the
                    // expensive verdict, since `.any()` short-circuits only on *accept*. It
                    // was **wrong and it bought nothing**:
                    //
                    // - Wrong, because `parse` strips an international prefix and a bare
                    //   country calling code as well as the national prefix, so a candidate
                    //   may legitimately be several digits longer than any `possible_length`.
                    //   `39 3332 2673 8858` is a real Italian number written with its country
                    //   code and the gate refused it — a **recall gap, i.e. a miss**, which is
                    //   the one direction a privacy filter may never fail in.
                    // - Nothing, because the per-scan memoization in `push_candidates` already
                    //   collapses the *repeating* case, which is what every measurement then
                    //   available exercised. Measured with the gate forced open, on the inputs
                    //   it was introduced for: 384 ms vs 382, 257 vs 258.
                    //
                    // The rule that outlives it: **a cheap filter in front of a validator must
                    // be derived from what the *validator* accepts, not from what the metadata
                    // describes** — and it has to be proved differentially, against generated
                    // inputs, never against a list the author expected it to allow.
                    //
                    // **What bounds the cost instead is a fail-closed budget**, not a filter —
                    // see [`MAX_PHONE_VALIDATIONS_PER_REQUEST`], and note why: on a body whose
                    // candidates are *distinct* the memo does nothing at all, and a validator
                    // call is ~6.5 µs per region however it is asked (M10-R20).
                    //
                    // **The charge happens here, one unit per `parse()`, and both halves of that
                    // are M10-R29.** It happens *here* because this is the only validator whose
                    // cost the budget was sized for: the nine always-on national-ID checksums
                    // are pure arithmetic on ≤ 18 bytes, and charging them the same rate refused
                    // legal bodies in 45 ms — with the phone tier not even loaded — where the
                    // previous release masked and forwarded them. And it is *per `parse()`*
                    // because that is the ~6.5 µs the number was derived from; charging once per
                    // candidate made one unit mean anywhere from one region to nine, so the
                    // published bound could not be true in any single unit. `.any()` short-circuits on
                    // accept, so a real number is charged only for the regions actually tried.
                    applicable.iter().any(|region| {
                        budget.spend();
                        national_phone_valid(*region, matched)
                    })
                })),
                scan: Scan::Overlapping,
                // **Only the un-anchored families, and the asymmetry is the point (M10-R1).**
                // The trunk anchor *constrains* a candidate's start; it does not forbid a
                // mid-value one (M10-R26/R31): inside `020 7946 0958` the `0958` is a `0` on
                // an ASCII word boundary. What differs is the consequence — a shifted trunk
                // span is **accepted**, so it overlaps the real number, the resolver unions
                // the two and the bytes stay covered. The outcome is an over-mask, never a
                // truncation, and `mask_all`'s fixpoint (M8-R8) has nothing to recover.
                // Shrinking here would buy nothing and cost something
                // real: it accepts mid-number prefixes that are valid in *some* region, which
                // bridges two adjacent numbers into one coalesced span. Measured on
                // `020 7946 0958 0161 496 0000`, that turned PHONE-NAT-04's two spans into
                // one. Privacy-safe either way, but M8.1's measured GB/DE behaviour is a
                // promise this milestone made, and one placeholder where the model needs two
                // is a fidelity loss for nothing.
                shrink_on_reject: matches!(shape, Groups | LongBlock),
            })
        })
        .collect()
}

/// Whether `phonenumber` confirms `matched` is a real, **assigned** number for `country`
/// (`is_valid` — assigned prefix *and* length, not merely a *possible* length).
///
/// This is the whole precision story of the tier: sequential digits, all-zeros, sort codes,
/// order refs and unassigned prefixes all *parse* and are **not valid**, so they are
/// rejected.
fn national_phone_valid(country: Id, matched: &str) -> bool {
    phonenumber::parse(Some(country), matched)
        .map(|number| number.is_valid())
        .unwrap_or(false)
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
    #[cfg(test)]
    pub(crate) fn raw_candidates(&self, input: &str) -> Vec<PiiEntity> {
        // Unbounded on purpose: this exists to inspect the *resolver's* invariant over every
        // candidate, so a budget refusal would silently shorten the very list under test.
        let budget = Budget::unlimited();
        let mut candidates: Vec<PiiEntity> = Vec::new();
        for rec in &self.recognizers {
            rec.push_candidates(input, &mut candidates, &budget);
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
    ///
    /// **Validator results are memoized for the duration of one scan (M10-R2).** The rescan
    /// probes O(n) start positions, so a digit-dense field asks the same question about the
    /// same bytes over and over — and for the phone recognizers each question costs up to
    /// five `phonenumber::parse().is_valid()` calls. Measured before: a legal 12 MiB body of
    /// repeated digit groups burned **105 s** of CPU on an unauthenticated path. The map is
    /// local to this call, so there is no lock and no cross-request state; the validator is a
    /// pure function of the matched bytes, so a hit cannot change a verdict.
    ///
    /// **`budget` is what the memo cannot do (M10-R20).** A memo only helps candidates that
    /// *repeat*; on a body of **distinct** digit groups every one is a cache miss at ~6.5 µs
    /// per region. The budget is decremented per miss and shared across the whole field, so an
    /// exhausted budget means the caller (`try_detect`) blocks the request instead of shipping
    /// a partial scan. It is deliberately **not** consulted by the regex loop's structure —
    /// once it hits zero the scan stops accepting, and the error is raised one level up, so
    /// there is exactly one place that decides what a truncated scan means.
    fn push_candidates(&self, input: &str, out: &mut Vec<PiiEntity>, budget: &Budget) {
        let mut runs: Vec<Range<usize>> = Vec::new();
        let mut memo: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
        let mut at = 0usize;
        while at <= input.len() {
            // **Stop the moment the allowance is gone, whichever recognizer spent it.** The field
            // is going to be refused by `try_detect` regardless, so there is nothing to buy
            // by finishing the scan — and the alternative, letting the loop run on with a
            // saturated budget, is how a partial scan would end up looking like a complete one.
            if budget.is_exhausted() && self.validate.is_some() {
                break;
            }
            let Some(m) = self.regex.find_at(input, at) else {
                break;
            };
            let accepted = match &self.validate {
                None => Some(m.start()..m.end()),
                Some(check) => {
                    let verdict = match memo.get(m.as_str()) {
                        Some(known) => *known,
                        None => {
                            // The validator charges the budget itself, at its own rate — see
                            // [`Validator`] (M10-R29). The memo is what keeps a *repeating*
                            // candidate from being charged twice; on distinct candidates it does
                            // nothing, which is exactly the case the budget exists for.
                            let v = check(m.as_str(), budget);
                            memo.insert(m.as_str(), v);
                            v
                        }
                    };
                    if verdict {
                        Some(m.start()..m.end())
                    } else {
                        self.shrink_to_a_valid_prefix(
                            input,
                            m.start()..m.end(),
                            check,
                            &mut memo,
                            budget,
                        )
                    }
                }
            };
            if let Some(span) = accepted {
                match runs.last_mut() {
                    // Hits arrive in non-decreasing start order, so only the last run can
                    // still grow.
                    Some(run) if span.start < run.end => run.end = run.end.max(span.end),
                    _ => runs.push(span),
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

    /// A rejected match, cut back one digit group at a time, until the validator accepts a
    /// **prefix starting at the same byte** — or there is nothing left to cut (M10-R1).
    ///
    /// **This is what the trunk `0` used to provide for free, and removing the anchor took it
    /// away.** The anchor *constrains* a candidate's start without forbidding a mid-value one
    /// (M10-R26/R31) — inside `020 7946 0958` the `0958` is a `0` on an ASCII word boundary, so
    /// a trunk span can start there. What the anchor buys is the **consequence**: that shifted
    /// span is *accepted*, so it overlaps the real number and the resolver unions the two — an
    /// over-mask, and no byte is left in clear. An **un-anchored** candidate has no such pin,
    /// and there a rejected over-match yields a **truncated** value instead:
    ///
    /// ```text
    /// 912 345 678 913 456 789      two real Portuguese numbers, one space apart
    ///   ↑ start 0  : `912 345 678 913`  → 12 digits, rejected
    ///   ↑ start 4  : `345 678 913 456`  → accepted, and it BEGINS INSIDE number 1
    ///   → masking yields `912 [PHONE_1]` — three digits of a real number, upstream in clear
    /// ```
    ///
    /// The fixpoint cannot repair that: **it recovers a value it did not touch, never one the
    /// mask ate.** Nor could any guard see it — the M8-R8 test asserts `detect(masked)` is
    /// empty, which the orphaned `912` *satisfies*; PROP-03 quantifies over accepted
    /// candidates, and those bytes belonged to a **rejected** one.
    ///
    /// Cutting at separators rather than one byte at a time is not just an optimisation: the
    /// group boundary is exactly where a shorter *rendering of the same number* ends, so at
    /// most four attempts are made (five groups is the widest family).
    ///
    /// **Applied to the un-anchored families only** ([`Recognizer::shrink_on_reject`]), and the
    /// asymmetry was measured rather than assumed. The trunk families do not need it — their
    /// anchor already pins the start, so the fixpoint recovers what they shadow — and giving it
    /// to them costs something real: it accepts mid-number prefixes that are valid in *some*
    /// region, which bridged `020 7946 0958 0161 496 0000` into a single coalesced span and
    /// turned PHONE-NAT-04's two numbers into one placeholder. Privacy-safe, but a fidelity
    /// loss for no gain, and a break of the GB/DE behaviour M8.1 measured.
    fn shrink_to_a_valid_prefix<'a>(
        &self,
        input: &'a str,
        span: Range<usize>,
        check: &Validator,
        memo: &mut std::collections::HashMap<&'a str, bool>,
        budget: &Budget,
    ) -> Option<Range<usize>> {
        if !self.shrink_on_reject {
            return None;
        }
        let mut end = span.end;
        while end > span.start {
            // The last separator strictly inside the candidate — i.e. drop one group.
            let cut = input[span.start..end].rfind([' ', '-', '.'])?;
            end = span.start + cut;
            if end <= span.start {
                return None;
            }
            let prefix = &input[span.start..end];
            let verdict = match memo.get(prefix) {
                Some(known) => *known,
                None => {
                    // Give up rather than shrink further on an empty allowance. Returning `None`
                    // here is a *miss*, not a silent pass: the caller's budget is already spent,
                    // so `try_detect` refuses the whole field below.
                    if budget.is_exhausted() {
                        return None;
                    }
                    let v = check(prefix, budget);
                    memo.insert(prefix, v);
                    v
                }
            };
            if verdict {
                return Some(span.start..end);
            }
        }
        None
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

/// How many `phonenumber::parse()` calls one **request** may make before the structured layer
/// refuses it (M10-R20, re-scoped by M10-R28).
///
/// **This is a fail-closed bound on an unauthenticated CPU cost, not an optimisation** — the
/// distinction matters because M10 already tried the optimisation and it was a leak (M10-R13).
///
/// `phonenumber::parse().is_valid()` costs **~6.5 µs per region** and there is no sound way to
/// answer it more cheaply: any pre-filter must be derived from what the *validator* accepts,
/// and the one attempt at that refused real numbers. The per-scan memo removes the cost only
/// for candidates that **repeat** — which every DoS figure this milestone published happened to
/// exercise, because they were all built with `unit.repeat(n)`. On a body whose candidates are
/// genuinely **distinct** the memo does nothing, and a legal 15 MiB body cost **64 s** of CPU
/// on an unauthenticated path.
///
/// So the work is bounded, and exceeding the bound is an `Err` on the
/// [`try_detect`](PiiDetector::try_detect) channel: the request is **blocked**, never forwarded
/// with a partial scan. That is the same call M5-R7 settled — *a detector may degrade its own
/// recall, but it may never decide for the caller that degraded output is acceptable.*
///
/// **`_PER_REQUEST`, and the name is the finding.** M10 first spelled this `_PER_FIELD` and meant
/// it: [`try_detect`](PiiDetector::try_detect) minted a fresh allowance on every call, and
/// `PrivacyStage` calls it once per text field, `Vault::mask_all` up to five times per field. So the
/// real ceiling was `50,000 × fields × passes`, every factor of it chosen by the client — the same
/// 15.6 MiB that is refused in one field answered `200` in **57 s** across 78 legal
/// `messages[].content` fields, indistinguishable from the build with no budget at all (M10-R28),
/// and the published per-field figure was not true in any unit (M10-R30). One [`Budget`] per
/// request, created by `PrivacyStage` and **taken** by
/// [`try_detect`](PiiDetector::try_detect) / [`redetect`](PiiDetector::redetect) rather than passed
/// to an optional sibling of theirs (M10-R35), is what makes the name honest.
///
/// **The number is a CPU ceiling on validation, and it was chosen against a legal payload rather
/// than an adversarial one.** One unit is one `parse()`, measured at **~3 µs** on the shipped release
/// build (not the ~6.5 µs the library's own docs suggest), so 500,000 bounds a request's
/// domestic-phone *validation* at **~1.5 s**.
///
/// The **legal** payload it is measured against is a database tool result with one phone column,
/// which is what an agent produces by accident. These are `DOS-BUD`'s printed rows, through the whole
/// fixpoint as a request pays it — **not** re-typed, and not a single-pass measurement:
///
/// | tool result | rows | units | verdict |
/// |---|---|---|---|
/// | 357 KB, `347 XXXXXXX` column | 5,000 | 5,000 | masked |
/// | 3.7 MB, same rendering | 50,000 | 50,000 | masked |
/// | **16 MiB** (`MAX_BODY_BYTES`), same rendering | 221,941 | 221,941 | masked |
/// | **16 MiB, the same numbers written `3XX XXX XXXX`** | 219,095 | **500,000** | **refused** |
///
/// **The allowance is a count of numbers, and how many depends on how they are written (M10-R56).**
/// `national_phone_valid` is `.any()` over the regions whose plans use that candidate's *shape
/// family*, so the accept path is cheap; but `Scan::Overlapping` resumes one `char` past each
/// match's **start**, so a grouped or pair-separated number also proposes sub-candidates from inside
/// itself, and each of those is rejected and pays its family's whole region list.
///
/// Measured per row in a column of 20,000 — the figure a real payload meets: `347 XXXXXXX` **1.00**,
/// any `+CC` form 1.02, `0X XX XX XX XX` (FR) 3.27, `6XX XX XX XX` (ES) 3.27, `3XX XXX XXXX`
/// (**IT grouped**) **8.00**. So 500,000 units is ≈**62,500 phone numbers per request** at the most
/// expensive column rendering and ≈500,000 at the cheapest.
///
/// **The bytes are a property of the layout, not of the limit.** The same 62,500 grouped numbers are
/// refused at **793 KB** as a bare column, **2.0 MB** as `name,phone` and **4.45 MB** as a six-column
/// export. An ordinary 5,000-row export spends 8%; the M7 22 KiB turn spends 0.
///
/// *(Per candidate **in isolation** the numbers differ — `06 12 34 56 78` costs 46 on its own, a
/// `+CC` form costs 0 because that recognizer has no validator — and the difference is the per-scan
/// memo, which absorbs repeated sub-candidate prefixes in a column. Isolation measures how a
/// rendering behaves; the column measures what a body costs.)*
///
/// *(Four published versions of this band were wrong, all optimistic: an eleven-digit column that
/// masked nothing (M10-R49); `347 XXXXXXX` alone, the cheapest legal phone in the shipped set
/// (M10-R53); a "2.6 MB" probe whose generator repeated (M10-R56). Each measurement was correct and
/// each **generalization** was not. **A conclusion drawn from one point of a grid is a fact about
/// that point.**)*
///
/// An earlier draft of this constant read 50,000 units and would have refused the 367 KB row — an
/// entirely ordinary `tool_result`. *A fail-closed threshold whose refusal is a routine event is the
/// wrong threshold:* every refusal costs the agent a turn, and a bound that fires on legal traffic
/// teaches its operator to raise it rather than to trust it. Headroom against a conversation is
/// total — the M7 22 KiB Claude Code turn spends **0** units, pinned by PHONE-BUD.
///
/// **What this does not bound, stated because leaving it out was a finding twice.** The allowance
/// caps *validator calls*. Regex scanning and the mask rewrite are linear in body size and entity
/// count, bounded only by `MAX_BODY_BYTES` — 229 ms for 16 MiB with the tier off. Across every shape
/// measured a request costs at most about **3 s**, against **57 s** before the allowance became
/// per-request. The DoS is closed by removing a multiplier the client chose, not by making the ceiling
/// small. See `docs/ARCHITECTURE.md` for the full table.
///
/// (`cargo test` builds unoptimized, where the same calls take far longer — which is why DOS-06's
/// refusal case is not wrapped in a wall-clock budget. The number must come from the product, not
/// from the profile the guard happens to run in.)
pub const MAX_PHONE_VALIDATIONS_PER_REQUEST: usize = 500_000;

impl PiiDetector for StructuredRecognizers {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        // Infallible view, **not on the request path**. It mints its own allowance because there is
        // no request to charge — the per-call semantics M10-R20 shipped. It must NOT silently return
        // a partial scan (that is a miss, and a miss is a leak), so an exhausted budget yields
        // nothing here and the fallible callers — which is what the request path uses — see the error.
        self.try_detect(input, &Budget::new(MAX_PHONE_VALIDATIONS_PER_REQUEST))
            .unwrap_or_default()
    }

    /// **The only place in the tree that does budgeted work, and — since M10-R35 — the only fallible
    /// method this type overrides.** [`redetect`](PiiDetector::redetect) inherits the trait default,
    /// which forwards the *same* budget here, so the fixpoint's later passes are charged by
    /// construction rather than by an override somebody has to remember.
    fn try_detect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        let mut candidates: Vec<PiiEntity> = Vec::new();
        for rec in &self.recognizers {
            rec.push_candidates(input, &mut candidates, budget);
        }
        if budget.is_exhausted() {
            // **Actionable, because an unactionable refusal is a badly chosen threshold
            // (M10-R27).** This reaches the client verbatim: `privacy.rs` blocks the request
            // with `DetectError`'s `Display`, which becomes the 400 body. And the client is
            // usually an *agent*, which cannot connect the failure to its cause on its own —
            // the 400 arrives when it tries to send a turn to the model, not from the tool
            // whose output is oversized, so its instinct is to retry the identical request
            // and fail identically. Saying what to change is the difference between a task
            // that adapts and a task that wedges.
            //
            // Value-free, as `DetectError` requires: the two numbers are a byte count and a
            // constant, never input-derived content — and E2E-05 pins that structurally, by
            // checking every digit run in this string against the request body.
            //
            // **It says "request", and after M10-R28 that is the truth rather than a widening.**
            // The allowance is spent across every field of the request, so the field that trips
            // it is where the budget ran out, not necessarily the one that consumed it — a body
            // of many medium digit-dense fields reaches it just as a single huge one does. The
            // advice has to cover both, or it sends an agent to shrink the wrong thing (M10-R29
            // is what a confidently-misdirected refusal costs).
            //
            // **Constructed as `budget_exhausted`, not as a bare `DetectError` (M10-R41).** This is
            // the one error in the codebase that `FailOpen` must *not* swallow, and it now says so
            // on the value instead of leaving the wrapper to infer it from a global side-condition.
            return Err(DetectError::budget_exhausted(
                "structured",
                format!(
                    "this request exhausted the domestic-phone validation budget of {} number \
                     checks; the allowance is per request and ran out while scanning a {} byte \
                     field. The request was blocked rather than forwarded with a partially \
                     scanned body. Retrying it unchanged will fail identically — send less \
                     digit-dense text instead: for an oversized tool result, add a LIMIT to the \
                     query or return fewer rows per call, and drop that turn rather than \
                     resending it. Note that many medium digit-dense fields exhaust the \
                     allowance just as one very large field does, so splitting the same content \
                     across more fields will not help.",
                    budget.initial(),
                    input.len()
                ),
            ));
        }
        Ok(self.finish(input, candidates))
    }
}

impl StructuredRecognizers {
    /// Resolve overlaps and emit the audit lines — the tail shared by both entry points.
    fn finish(&self, input: &str, candidates: Vec<PiiEntity>) -> Vec<PiiEntity> {
        let kept = resolve_overlaps(input, candidates);
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
/// checksum-backed) except two cases that are masked anyway but tagged `Structural` so
/// downstream code knows the arithmetic did not confirm them:
///
/// - an **IBAN** failing **either** its mod-97 checksum **or** its country's expected
///   length (M4);
/// - a **Dutch VAT number** whose 9-digit body fails the 11-proef (M11 Track A). The
///   2020 sole-trader `btw-id` is randomized by design and has no checksum to pass, so
///   this is the honest tag for it rather than a `Verified` that would be a claim the
///   scheme cannot support. Every other VAT country here is checksum-gated at the
///   recognizer, so reaching this function at all means it already passed.
fn confidence_of(kind: PiiKind, text: &str) -> Confidence {
    match kind {
        PiiKind::Iban => {
            if iban_mod97(text) && iban_length_ok(text) {
                Confidence::Verified
            } else {
                Confidence::Structural
            }
        }
        // Only the NL pattern reaches here unchecked — it is the one VAT recognizer with
        // `validate: None`, because its scheme has nothing to validate.
        PiiKind::TaxId if text.len() == 14 && text[..2].eq_ignore_ascii_case("NL") => {
            if nl_bsn_valid(text.get(2..11).unwrap_or("")) {
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

/// The body of a VIES-form VAT number — everything after the two-letter country prefix.
///
/// `get(2..)` rather than `[2..]`: the patterns in [`vat_recognizers`] all begin with two
/// ASCII letters so the index is always a char boundary today, but a panic here would reach
/// the masking path, where it is caught as a **blocked request** (M4-R19's fail-closed
/// posture). A recognizer must not be one regex edit away from refusing traffic, and an
/// empty body simply fails every checksum below.
fn vat_body(matched: &str) -> &str {
    matched.get(2..).unwrap_or("")
}

/// Could **any** pattern in [`vat_recognizers`] match `token` — ignoring every checksum?
///
/// Not used on the request path. This is the grammar the VIES-form patterns share, stated once
/// so that `VAT-04`'s *absence* assertions can be checked for **reachability** before they are
/// believed (M11-R1): every prefixed pattern is `[A-Z]{2}` immediately followed by an unbroken
/// run of ASCII alphanumerics, with ASCII word boundaries at both ends. A negative written in a
/// shape this returns `false` for is absent because of its punctuation, not because its country
/// does not ship — and asserting *that* absence proves nothing at all.
///
/// It lives here, beside the patterns, rather than in the test module, because it is a statement
/// about the grammar: a pattern that stops being `[A-Z]{2}`-prefixed makes this function wrong,
/// and this is where somebody making that change will be looking.
///
/// **Byte-indexed on purpose.** The length floor runs first, so `bytes[..2]` and `bytes[2..]`
/// cannot panic, and comparing bytes rather than `char`s means a multi-byte character can never
/// land mid-index. The floor is 9 — the shortest shipped form is `DE` + 9 digits — which is also
/// what rules out the degenerate inputs (`""`, `"ES"`) where a `chars().take(2)` reads as
/// vacuously true.
/// `#[cfg(test)]`: this is a statement *about* the patterns, not a step in matching them.
/// Gating it keeps the shipped build's footprint honest and the suite warning-free.
#[cfg(test)]
fn vat_grammar_could_match(token: &str) -> bool {
    let bytes = token.as_bytes();
    if bytes.len() < 9 {
        return false;
    }
    bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..].iter().all(u8::is_ascii_alphanumeric)
}

/// Italian **Partita IVA** check digit (M11 Track A) — 11 digits, mod-10 with position
/// doubling (the Luhn family, but indexed from the left over a fixed length rather than
/// from the right over a variable one, so [`luhn_valid`] cannot be reused).
///
/// Odd 1-indexed positions contribute their digit; even ones contribute the digit doubled,
/// less 9 when that exceeds 9. The eleventh digit must be the amount that rounds the sum up
/// to a multiple of ten.
///
/// **Measured against four real published P.IVAs** — ENI `00905811006`, Ferrari
/// `00159560366`, TIM `00488410010`, Luxottica `00891030272` — pinned in
/// `italian_piva_accepts_real_published_numbers`. Four independent agreements is what makes
/// this an implementation of the scheme rather than a plausible transcription of it.
///
/// **Accepted false-positive cost, measured not asserted:** ~1 in 10 arbitrary 11-digit
/// numbers satisfies a mod-10 check, so an ordinary 11-digit token can be masked. That is
/// the same tradeoff M4-R6 took for the 9- and 11-digit national-ID recognizers, on the same
/// grounds (over-mask, never leak — and the vault restores it on the response path).
/// `vat_over_mask_rate_on_arbitrary_eleven_digit_numbers` measures it rather than guessing.
fn it_piva_valid(matched: &str) -> bool {
    let d: Vec<u32> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as u32)
        .collect();
    if d.len() != 11 {
        return false;
    }
    let mut sum = 0;
    for (i, &x) in d[..10].iter().enumerate() {
        // 1-indexed odd positions are the even indices here.
        sum += if i % 2 == 0 {
            x
        } else if x * 2 > 9 {
            x * 2 - 9
        } else {
            x * 2
        };
    }
    (10 - sum % 10) % 10 == d[10]
}

/// German **USt-IdNr** check digit — ISO 7064 Mod 11,10 over the first 8 of 9 digits.
///
/// The identical loop already runs in [`de_steuerid_valid`]; what is *not* carried over is
/// that scheme's "exactly one digit repeated among the first ten" structural rule, which
/// belongs to the Steuer-ID and is no part of the VAT specification. Verified against the
/// German tax administration's own documented test vector `136695976`.
fn de_vat_valid(matched: &str) -> bool {
    let d: Vec<u32> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as u32)
        .collect();
    if d.len() != 9 {
        return false;
    }
    let mut product = 10;
    for &x in &d[..8] {
        let mut sum = (x + product) % 10;
        if sum == 0 {
            sum = 10;
        }
        product = (sum * 2) % 11;
    }
    (11 - product) % 10 == d[8]
}

/// UK **VAT** check digits — the last 2 of 9, weights 8..2 over the first 7.
///
/// Two eras of the same scheme are live simultaneously: the original **mod-97** and the
/// post-2010 **mod-97-55** (the same sum offset by 55). A number issued under either is
/// valid, so both are accepted — which doubles the incidental acceptance rate to ~2/97 and
/// is why the mandatory `GB` prefix is what actually keeps this safe, not the checksum alone.
///
/// Verified against `123456782` (the documented worked example) and Tesco's `220430231`.
fn gb_vat_valid(matched: &str) -> bool {
    let d: Vec<i32> = matched
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as i32)
        .collect();
    if d.len() != 9 {
        return false;
    }
    let weights = [8, 7, 6, 5, 4, 3, 2];
    let sum: i32 = (0..7).map(|i| d[i] * weights[i]).sum();
    let stated = d[7] * 10 + d[8];
    let expect = |offset: i32| (97 - (sum + offset).rem_euclid(97)).rem_euclid(97);
    stated == expect(0) || stated == expect(55)
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
/// Is this IBAN rendering acceptable, given its **letter case** (M11-R10)?
///
/// **The leak this closes.** The pattern used to spell its letters `[A-Z]`, so
/// `it60x0542811101000000123456` — and, worse, `IT60x0542811101000000123456`, an otherwise
/// canonical IBAN with **one** lowercase letter — matched nothing at all and was forwarded to the
/// provider in clear. The letters are ASCII word characters, so there was no `(?-u:\b)` for a
/// shorter recognizer to fall back on: the span did not shrink, it disappeared. The `iban_mod97`
/// doc comment has said "letters are folded to uppercase" since M1 — the *validator* was written
/// for input the *regex* could never deliver, which is how long this went unnoticed.
///
/// **Why folding case needs a gate at all, with the number.** Measured over 341.1 MB / 16 380
/// files of uncurated third-party source: the uppercase pattern matches **1** string, the
/// case-folded one **150**. Those extra 149 are hex digests, base64 blobs and type names — and an
/// IBAN has *no hard checksum gate* (M4 masks a structurally valid IBAN even when mod-97 fails, on
/// purpose), so without a gate every one of them would be masked. Masking a hex digest inside a
/// `tool_use.input` is exactly the functional harm M10 spent nine rounds bounding.
///
/// **So the rule is split by rendering, and it costs nothing.** A **canonical** (all-uppercase)
/// rendering keeps M4's rule untouched: structurally valid is masked, mod-97 only sets
/// [`Confidence`]. A rendering carrying **any** lowercase letter is accepted only if it is fully
/// verifiable — mod-97 *and* the ISO 13616 length. Measured residue of that gate on the same
/// corpus: **0 of the 149**. (Note `iban_length_ok` returns `true` for a country code it does not
/// know, so for 145 of those the gate is mod-97 alone — the zero survives that weaker reading,
/// which is the one the code actually implements.)
fn iban_case_gate(matched: &str) -> bool {
    if !matched.bytes().any(|b| b.is_ascii_lowercase()) {
        return true;
    }
    iban_mod97(matched) && iban_length_ok(matched)
}

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
        let masked = vault
            .mask_all(input, &detector, &Budget::per_call())
            .unwrap();
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
        let masked = vault
            .mask_all(input, &detector, &Budget::per_call())
            .unwrap();

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
            let masked = vault
                .mask_all(input, &detector, &Budget::per_call())
                .unwrap();
            assert!(
                detector.detect(&masked).is_empty(),
                "PII survived masking of {input:?} → {masked:?}"
            );
            assert_eq!(vault.demask(&masked), input, "round-trip must stay exact");
        }

        // A left-over bare `@domain` is explicitly NOT PII (M4-R11) — but the local part,
        // which is the identifying half, must be gone.
        let mut vault = crate::pii::anonymizer::Vault::new();
        let masked = vault
            .mask_all("a@b.com@c.com", &detector, &Budget::per_call())
            .unwrap();
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
        let masked = vault
            .mask_all(input, &detector, &Budget::per_call())
            .unwrap();

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

    // ---- M8.1 / M10: domestic phone recognizers via `phonenumber` ----

    /// Assert `input` masks to exactly one `Phone` span whose text is the full `number`.
    /// (Where the universal 3-3-4 phone arm also fires on a sub-span, the resolver unions
    /// the two same-kind spans back to the full number — so this stays exact.)
    fn assert_phone<S: AsRef<str>>(locales: &[S], input: &str, number: &str) {
        assert_eq!(
            kinds_with(locales, input),
            vec![(PiiKind::Phone, number.to_string())],
            "input {input:?}"
        );
    }

    #[test]
    fn gb_national_phone_detected_when_gb_enabled() {
        // The un-anchored domestic forms the universal (US 3-3-4 / +CC) arm misses:
        // 3-4-4 grouping, 5-6 mobile, compact, freephone.
        for (input, number) in [
            ("call 020 7946 0958 now", "020 7946 0958"),
            ("mob 07911 123456 pls", "07911 123456"),
            ("compact 02079460958 ok", "02079460958"),
            ("free 0800 1111 line", "0800 1111"),
            ("leeds 0113 496 0000 x", "0113 496 0000"),
        ] {
            assert_phone(&["gb"], input, number);
        }
    }

    #[test]
    fn de_national_phone_detected_when_de_enabled() {
        for (input, number) in [
            ("Berlin 030 12345678 heute", "030 12345678"),
            ("mobil 0171 1234567 bitte", "0171 1234567"),
            ("Frankfurt 069 90009000 x", "069 90009000"),
            ("kompakt 03012345678 ok", "03012345678"),
        ] {
            assert_phone(&["de"], input, number);
        }
    }

    #[test]
    fn national_phone_is_gated_by_pii_locales() {
        // `020 7946 0958` is a 3-4-4 GB number the UNIVERSAL arm does not match (it needs
        // 3-3-4 or a `+CC`), and its spaces break the contiguous 9/11-digit ID patterns —
        // so with no region enabled it is masked by nothing, proving the gate is real and
        // not a coincidental universal hit.
        //
        // **`us` alone is the honest "gate off" case now (M10).** It used to be `it,us`,
        // and that is precisely the defect this milestone closed: both codes mapped to no
        // recognizer, so the assertion passed while the *shipped default* masked no
        // domestic number at all. `it` now resolves to a real region — one whose plan
        // happens to accept this number — so keeping it here would assert the opposite of
        // what it reads as.
        assert!(
            kinds_with(&["us"], "call 020 7946 0958 now").is_empty(),
            "GB national phone must not be masked when no region enabling it is configured"
        );
        // A code we do not ship is not a region: it contributes nothing, rather than
        // silently falling back to the default set.
        assert!(kinds_with(&["zz"], "call 020 7946 0958 now").is_empty());
        // And a *different* region whose plan rejects this number leaves it alone.
        assert!(kinds_with(&["es"], "call 020 7946 0958 now").is_empty());
        // Turning GB on masks it.
        assert_phone(&["gb"], "call 020 7946 0958 now", "020 7946 0958");
    }

    /// **The floor M10 exists to establish: a proxy started with NO configuration must
    /// detect a domestic number.**
    ///
    /// This is the regression that shipped for two milestones — [M4](docs/ROADMAP.md#m4)
    /// introduced `PII_LOCALES` with the placeholder default `it,us` while the FP-prone
    /// tier was *empty*, and [M8.1](docs/ROADMAP.md#m81) filled that tier with `gb`/`de`
    /// without revisiting the default. Nothing regressed; the gap was simply never filled.
    /// It stayed invisible because every test that exercised a domestic number passed the
    /// region in explicitly.
    ///
    /// So this test builds the detector the way `Config`'s default does — from **no
    /// environment at all** — and asserts a real number from each shipped region is masked.
    /// "Everything off unless you knew to switch it on" cannot come back silently.
    #[test]
    fn the_default_configuration_detects_a_domestic_number_in_every_shipped_region() {
        let detector = StructuredRecognizers::new();
        for (region, input, number) in [
            ("de", "Berlin 030 12345678 heute", "030 12345678"),
            ("es", "llama al 612 34 56 78 ahora", "612 34 56 78"),
            ("fr", "appelle le 01 23 45 67 89 svp", "01 23 45 67 89"),
            ("gb", "call 020 7946 0958 now", "020 7946 0958"),
            ("it", "chiama 06 69821234 oggi", "06 69821234"),
            ("it", "cellulare 347 1234567 grazie", "347 1234567"),
            ("lv", "zvani 67 22 33 44 tagad", "67 22 33 44"),
            ("nl", "bel 0343 123456 nu", "0343 123456"),
            ("pt", "liga 210 123 456 agora", "210 123 456"),
            ("cn", "拨打 138 0013 8000 电话", "138 0013 8000"),
        ] {
            let got: Vec<(PiiKind, String)> = detector
                .detect(input)
                .into_iter()
                .map(|e| (e.kind, e.text))
                .collect();
            assert_eq!(
                got,
                vec![(PiiKind::Phone, number.to_string())],
                "the DEFAULT detector must mask the {region} domestic number in {input:?}"
            );
        }
    }

    /// The default set and the explicit set agree — so `PII_LOCALES` **replaces** the
    /// default rather than being the only way to get anything, and an operator who spells
    /// out every region gets exactly the shipped behaviour.
    #[test]
    fn the_default_region_set_is_the_vetted_table() {
        let all: Vec<&str> = PHONE_REGIONS.iter().map(|r| r.code).collect();
        let input = "chiama 06 69821234 oggi";
        assert_eq!(
            kinds_with(&all, input),
            StructuredRecognizers::new()
                .detect(input)
                .into_iter()
                .map(|e| (e.kind, e.text))
                .collect::<Vec<_>>()
        );
        assert_eq!(vetted_phone_regions().len(), PHONE_REGIONS.len());
    }

    /// **The over-masks we know about, asserted to still happen.**
    ///
    /// Each of these is a digit run that a real numbering plan accepts, so the detector is
    /// behaving correctly and the cost is real: a masked byte size or date inside
    /// `tool_use.input` hands the model `[PHONE_1]` where it needed the number. They are
    /// pinned rather than deleted because the alternative — quietly dropping a corpus
    /// negative that stopped passing — is how a measured cost becomes an unmeasured one.
    ///
    /// If one of these stops being masked, this test fails and somebody re-reads the numbers
    /// in ARCHITECTURE → *Domestic phone coverage* instead of finding them silently stale.
    #[test]
    fn known_over_masks_are_still_over_masks() {
        let detector = StructuredRecognizers::new();
        for (input, span, why) in [
            (
                "sizes 512 1024 2048 4096 bytes",
                "512 1024 2048",
                "area code 512 is Suzhou; this is the shape of a real Chinese landline",
            ),
            (
                "scadenza 28 01 2026 confermata",
                "28 01 2026",
                "eight digits starting 2 is a valid Latvian mobile",
            ),
            (
                "scadenza 01 02 2026 confermata",
                "02 2026",
                "02 is Milan's area code and Italian subscriber numbers can be short",
            ),
        ] {
            let found: Vec<String> = detector
                .detect(input)
                .into_iter()
                .filter(|e| e.kind == PiiKind::Phone)
                .map(|e| e.text)
                .collect();
            assert_eq!(
                found,
                vec![span.to_string()],
                "the documented over-mask of {input:?} changed — {why}. That is not \
                 necessarily wrong, but the published FP numbers now describe something else."
            );
        }
    }

    /// **Every shape a region declares must be one its own numbers need (M10-R3).**
    ///
    /// The table is the precision knob: each extra shape widens that region's candidate set,
    /// and a shape listed "for symmetry" costs false positives for no coverage — which is
    /// exactly what handing China the Italian-mobile `LongBlock` rendering did (file offsets
    /// and byte counts, 0.250 of the offsets pool). So the entries are held to the same
    /// standard as the region set itself: earn your place with a case.
    ///
    /// Removing any declared shape must change what one of [`REGION_RENDERINGS`] detects.
    ///
    /// **The renderings are a list in this file, not the corpus** — a unit test cannot read
    /// `tests/corpus/pii_cases.json`, and saying otherwise in the test's *name* would be the
    /// same class of overclaim this milestone keeps finding. Every one of them is also a
    /// corpus positive; keeping the two in step is review's job, not this test's.
    #[test]
    fn every_declared_shape_is_needed_by_a_real_rendering() {
        for region in PHONE_REGIONS {
            for shape in region.shapes {
                let reduced: Vec<PhoneShape> = region
                    .shapes
                    .iter()
                    .copied()
                    .filter(|s| s != shape)
                    .collect();
                let full = StructuredRecognizers::with_regions(&[region.id]);
                let cut = StructuredRecognizers::with_shapes(&[(region.id, &reduced)]);
                let differs = REGION_RENDERINGS
                    .iter()
                    .filter(|(code, _)| *code == region.code)
                    .any(|(_, number)| {
                        let text = format!("x {number} y");
                        full.detect(&text).len() != cut.detect(&text).len()
                    });
                assert!(
                    differs,
                    "{}'s `{shape:?}` shape is declared but no corpus rendering needs it — \
                     an unneeded shape is pure false-positive surface",
                    region.code
                );
            }
        }
    }

    /// One real rendering per (region, shape) — the evidence the table above rests on.
    const REGION_RENDERINGS: &[(&str, &str)] = &[
        ("de", "030 12345678"),
        ("de", "0171 1234567"),
        ("es", "91 123 45 67"),
        ("es", "612 34 56 78"),
        ("fr", "0123456789"),
        ("fr", "01 23 45 67 89"),
        ("gb", "020 7946 0958"),
        ("gb", "0800 1111"),
        ("it", "06 69821234"),
        // `320 123 4567` is 3-3-4, which the *universal* US arm already matches — so it is
        // not evidence for `Groups`. The toll-free 3-3-3 form is.
        ("it", "800 123 456"),
        ("it", "347 1234567"),
        ("lv", "67 22 33 44"),
        ("nl", "0343 123456"),
        ("pt", "210 123 456"),
        ("cn", "010 12345678"),
        ("cn", "138 0013 8000"),
    ];

    /// The un-anchored family is validated **only** against regions whose numbers really
    /// are written without a trunk `0` — the single most important precision decision in
    /// M10, and the one that is easiest to undo by accident.
    ///
    /// libphonenumber accepts a national number with *or without* its trunk prefix, because
    /// in a trunk-prefix country you can dial a local number that way. So handing
    /// un-anchored digit groups to DE/FR/NL/GB asks "could this be a same-area local dial?"
    /// — true of an enormous slice of ordinary numeric text. Measured: DE alone turned 7 of
    /// 24 digit-shaped non-phones into `Phone` spans.
    #[test]
    fn the_un_anchored_family_never_reaches_a_trunk_prefix_region() {
        for junk in [
            "sizes 512 1024 2048 4096",
            "retry after 30 60 120 seconds",
            "matrix 10 20 30 40",
            "the run took 100 200 300 ms",
            "id 123 456 789 in the list",
            "pairs 12 34 56 78 here",
            "ports 80 443 8080 open",
        ] {
            for region in ["de", "fr", "nl", "gb"] {
                assert!(
                    kinds_with(&[region], junk).is_empty(),
                    "{region} must not accept un-anchored digit groups: {junk:?}"
                );
            }
        }
    }

    #[test]
    fn national_phone_validator_rejects_lookalikes() {
        // COMPACT 0-leading runs of phone-plausible length: the universal arm can't touch
        // them (no separators), so they reach the `phonenumber` validator — which rejects
        // them because they are not assigned numbers. This is the M4-R1 FP concern, defused.
        for junk in ["0000000000", "0123456789", "0999999999"] {
            assert!(
                kinds_with(&["gb"], &format!("ref {junk} end")).is_empty(),
                "look-alike {junk:?} must be rejected by is_valid()"
            );
            assert!(kinds_with(&["de"], &format!("ref {junk} end")).is_empty());
        }
    }

    #[test]
    fn national_phone_does_not_swallow_an_adjacent_number() {
        // Two real GB numbers separated by a SINGLE space, no word between: an open
        // `(?:[ -]?\d)+` would grab them as one over-long span that is_valid() rejects.
        // Here the greedy match at position 0 is *already* the valid full number (the two
        // 3-group numbers don't let a longer arm-1 match form), so single-pass detect()
        // yields both. The *shadowing* sibling shape — where a longer invalid arm-1 match
        // hides the first number in one pass — is M8-R8, pinned at the `mask_all` level below.
        let got = kinds_with(&["gb"], "020 7946 0958 0161 496 0000");
        let phones: Vec<&String> = got
            .iter()
            .filter(|(k, _)| *k == PiiKind::Phone)
            .map(|(_, t)| t)
            .collect();
        assert!(
            phones.iter().any(|t| *t == "020 7946 0958")
                && phones.iter().any(|t| *t == "0161 496 0000"),
            "both adjacent numbers must be detected separately, got {got:?}"
        );
    }

    #[test]
    fn adjacent_national_phones_are_both_masked_by_the_fixpoint() {
        // M8-R8: arm 1's greedy `\d{3,4}` can take the trunk of the *next* number
        // (`0800 1111 0800`), an over-long span is_valid() rejects — which shadows the FIRST
        // number's shorter valid form in a single detect() pass (the overlapping rescan
        // resumes forward of the rejected match). The request path masks with
        // `Vault::mask_all`, whose fixpoint re-detects: masking the second number un-shadows
        // the first, and the next pass masks it. This makes that fixpoint reliance
        // load-bearing and visible — a future `redetect` shortcut that skips later passes (an
        // S4-style latency move) would fail HERE instead of leaking silently.
        use crate::pii::anonymizer::Vault;
        let detector = StructuredRecognizers::with_locales(&["gb"]);
        for input in [
            "0800 1111 0800 1111",     // two identical freephones — the shadowing shape
            "0800 1111 0207 946 0958", // 2-group first, 3-group second
            "call 0800 1111 0800 1111 now", // in prose
        ] {
            let mut vault = Vault::new();
            let masked = vault
                .mask_all(input, &detector, &Budget::per_call())
                .unwrap();
            assert!(
                detector.detect(&masked).is_empty(),
                "a national phone survived mask_all of {input:?}: {masked:?}"
            );
            assert_eq!(
                vault.demask(&masked),
                input,
                "round-trip must be exact for {input:?}"
            );
        }
    }

    /// **PHONE-NAT-09 (M10-R1) — masking two adjacent numbers must leave NO DIGIT of either.**
    ///
    /// The predicate is the finding. Its sibling above asserts `detect(masked).is_empty()`,
    /// and that is *satisfied* by the defect it needs to catch: when an un-anchored candidate
    /// starts mid-value, masking **truncates** the neighbour, and the three orphaned digits
    /// left behind are not detectable — which is exactly why they survived. "Nothing
    /// detectable survives" and "no byte of a real value survives" are different properties,
    /// and only the second one is the privacy guarantee.
    ///
    /// Runs on the **default** region set, not `["gb"]`: the trunk family is the one where
    /// the M8-R8 fixpoint argument still holds, so testing only it tests only the safe case.
    #[test]
    fn adjacent_phones_leave_no_digit_of_either_number() {
        use crate::pii::anonymizer::Vault;
        let detector = StructuredRecognizers::new();
        for input in [
            // The two M10-R1 repros: both numbers real, one space apart, un-anchored.
            "138 0013 8000 139 0013 8001", // CN mobiles
            "912 345 678 913 456 789",     // PT, PHONE-PT-03's own corpus positive twice
            "91 123 45 67 91 123 45 68",   // ES
            "347 1234567 320 123 4567",    // IT mobiles, the two un-anchored families meeting
            // And the trunk shapes M8-R8 already covered, so the stronger predicate is
            // applied to them too rather than only to the new families.
            "0800 1111 0800 1111",
            "020 7946 0958 0161 496 0000",
            "01 23 45 67 89 06 12 34 56 78",
        ] {
            let mut vault = Vault::new();
            let masked = vault
                .mask_all(input, &detector, &Budget::per_call())
                .unwrap();
            let survivors: String = masked
                .chars()
                .zip(masked.char_indices().map(|(i, _)| i))
                .filter(|(c, _)| c.is_ascii_digit())
                .filter(|(_, i)| {
                    // Placeholder indices (`[PHONE_1]`) are digits we put there ourselves.
                    let before = masked[..*i].rfind('[');
                    let closes = masked[*i..].find(']');
                    !matches!((before, closes), (Some(b), Some(_)) if !masked[b..*i].contains(']'))
                })
                .map(|(c, _)| c)
                .collect();
            assert!(
                survivors.is_empty(),
                "digits of a real number survived masking {input:?}: {masked:?} left \
                 {survivors:?} in clear"
            );
            assert_eq!(
                vault.demask(&masked),
                input,
                "round-trip must stay exact for {input:?}"
            );
        }
    }

    /// **PHONE-NAT-10 (M10-R13) — differential recall: nothing may sit between a candidate
    /// and its validator.**
    ///
    /// The property, stated so it cannot be satisfied by choosing convenient inputs: take a
    /// candidate **generated from a shape family's own grammar**; if `is_valid` accepts it for
    /// a region that **declares that family**, the detector must produce a `Phone` span
    /// covering the whole thing. Anything less is a **miss**, which for a privacy tool is the
    /// one direction that is not a trade-off.
    ///
    /// **Its predecessor asserted the same idea over 30 hand-written literals and could not
    /// fail.** They were all domestic renderings, so all ≤ 13 digits — and the defect lived at
    /// 14–15, where `phonenumber` strips an international prefix or a bare country code that
    /// no domestic rendering carries. *An assertion made only where it cannot fail is not an
    /// assertion.* Generating the inputs is what takes the author's expectations out of them.
    ///
    /// **The premise is per family on purpose.** Asking "valid for *any* region" would fold in
    /// the per-region shape restriction ([`PhoneRegion::shapes`]) and report it as a miss: a
    /// 14-digit un-anchored run that only Germany's plan accepts is *deliberately* not offered
    /// to Germany. That is a precision decision with its own measurement, not a filter bug,
    /// and conflating the two would make this guard un-actionable.
    #[test]
    fn a_valid_number_is_never_lost_between_the_regex_and_the_validator() {
        let detector = StructuredRecognizers::new();

        /// Calling codes of the regions that declare the `Groups` family (ES · IT · LV · PT ·
        /// CN), as a leading token the family's regex accepts (1–3 digits, non-zero first).
        const GROUPS_OWNER_CALLING_CODES: [&str; 5] = ["34", "39", "86", "351", "371"];

        // Deterministic pseudo-random digits — a fixed sequence, so a failure reproduces.
        let mut seed = 0x5eed_1234_u64;
        let mut tick = 0u64;
        let mut seed_tick = move || {
            tick = tick.wrapping_add(1);
            tick
        };
        let mut digit = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            b'0' + ((seed >> 33) % 10) as u8
        };
        let mut group = |n: usize, first_nonzero: bool| -> String {
            (0..n)
                .map(|i| {
                    let d = digit();
                    if i == 0 && first_nonzero && d == b'0' {
                        '7'
                    } else {
                        d as char
                    }
                })
                .collect()
        };

        // Accepted count **per generated shape**, not in aggregate (M10-R21). An aggregate
        // floor is dominated by whichever shape happens to be permissive — `Trunk` alone
        // contributed 705 of 1,286 — so it can be met with **zero** samples in the band that
        // actually matters. The band R13 lived in (14–15 digits, `Groups`) yielded 0–2 across
        // seeds, and 0 in seven of twenty: the floor was 12.9× slack and blind at the same time.
        let mut per_shape = [0usize; 6];
        let mut checked = 0usize;
        // Sized for the **debug** profile `cargo test` builds, where `phonenumber::parse` is
        // ~50× slower: 3,000 rounds took 93 s, which is a guard nobody would keep running.
        for _ in 0..400 {
            // Per family, in `PHONE_SHAPES` order, built to that family's own grammar —
            // including the group counts that push a candidate past any *domestic* length,
            // which is exactly the band the deleted length gate refused.
            let generated = [
                (
                    Trunk,
                    format!(
                        "0{} {} {}",
                        group(2, false),
                        group(4, false),
                        group(4, false)
                    ),
                ),
                (Trunk, format!("0{} {}", group(3, false), group(8, false))),
                (
                    TrunkPairs,
                    format!(
                        "0{} {} {} {} {}",
                        group(1, false),
                        group(2, false),
                        group(2, false),
                        group(2, false),
                        group(2, false)
                    ),
                ),
                // **The 14–15-digit band, aimed at rather than stumbled into (M10-R21).** This
                // is where M10-R13 lived: a candidate written with its **country calling
                // code**, which `parse` strips before validating. Under uniform random digits
                // the band is accepted ~2 times in 2,400 — statistically absent, and the
                // earlier aggregate floor could not tell. Leading with a real calling code of a
                // region that owns this family is what makes the sample land there.
                (
                    Groups,
                    format!(
                        "{} {} {} {}",
                        // Indexed by the array's own length, not a restated literal — a `% 4`
                        // here is what silently kept PT out of the aim while the doc comment
                        // above claimed it (M10-R34).
                        GROUPS_OWNER_CALLING_CODES
                            [(seed_tick() as usize) % GROUPS_OWNER_CALLING_CODES.len()],
                        group(4, false),
                        group(4, false),
                        group(4, false)
                    ),
                ),
                (LongBlock, format!("{} {}", group(3, true), group(8, false))),
                (
                    Groups,
                    format!("{} {} {}", group(3, true), group(3, false), group(3, false)),
                ),
            ];

            for (slot, (shape, candidate)) in generated.into_iter().enumerate() {
                checked += 1;
                // **The generator has to speak the family's grammar, and checking that is not
                // a formality — the first version emitted a four-pair French form where the
                // family requires five, so it produced renderings we never claimed to detect
                // and reported them as misses.** A rendering no family matches is a documented
                // recall gap (we do not claim every rendering on earth), not a defect on the
                // path this test is about.
                let pattern = PHONE_SHAPES
                    .iter()
                    .find(|(s, _)| *s == shape)
                    .map(|(_, p)| Regex::new(p).unwrap())
                    .expect("every generated shape must exist in PHONE_SHAPES");
                assert!(
                    pattern
                        .find(&candidate)
                        .is_some_and(|m| m.as_str() == candidate),
                    "the generator produced {candidate:?}, which the {shape:?} family does not \
                     match whole — fix the generator, not the detector"
                );

                // Only the regions that declare this family will ever see this candidate.
                let owners: Vec<Id> = PHONE_REGIONS
                    .iter()
                    .filter(|r| r.shapes.contains(&shape))
                    .map(|r| r.id)
                    .collect();
                if !owners.iter().any(|r| national_phone_valid(*r, &candidate)) {
                    continue;
                }
                per_shape[slot] += 1;

                let text = format!("x {candidate} y");
                let spans: Vec<String> = detector
                    .detect(&text)
                    .into_iter()
                    .filter(|e| e.kind == PiiKind::Phone)
                    .map(|e| e.text)
                    .collect();
                assert!(
                    spans.iter().any(|s| s == &candidate),
                    "{candidate:?} ({} digits) is valid for a region that declares the \
                     {shape:?} family, but the detector produced {spans:?} — that is a MISS, \
                     not an over-mask",
                    candidate.bytes().filter(u8::is_ascii_digit).count()
                );
            }
        }

        // Non-vacuity, **per generated shape**: each must have reached the assertion on its
        // own, or this guard is silently blind wherever it matters most (M10-R21).
        //
        // **Slot 3's floor is lower, and the number is measured rather than rounded.** That is
        // the country-code band, and it is *intrinsically* sparse: a random 12-digit national
        // number is rarely an assigned one, so even aiming a real calling code at it yields
        // ~15 acceptances per 400 rounds (uniform digits yielded 2). Raising the floor would
        // mean raising the round count, and this test already costs ~17 s in the debug profile.
        // The honest thing is a floor the aiming can actually clear, with the reason next to it
        // — not a uniform number that makes the table look tidy.
        //
        // **The margin here is 5, and it got smaller on purpose.** Fixing M10-R34 put PT into
        // the aim it had always claimed, which re-drew the whole deterministic sequence: this
        // slot measured 17 with four calling codes and measures 15 with five. Measured counts
        // as of `main`: `[355, 356, 330, 15, 28, 234]` of 2400.
        const FLOORS: [usize; 6] = [20, 20, 20, 10, 20, 20];
        for (slot, count) in per_shape.iter().enumerate() {
            assert!(
                *count >= FLOORS[slot],
                "generated shape #{slot} produced only {count} candidates any owning region \
                 accepts (floor {}), out of {checked} total — this guard is not exercising that \
                 shape, and an aggregate count would have hidden it",
                FLOORS[slot]
            );
        }
    }

    #[test]
    fn national_phone_validators_accept_reals_reject_junk() {
        // Direct validator unit tests (region-specific).
        assert!(national_phone_valid(Id::GB, "020 7946 0958"));
        assert!(national_phone_valid(Id::GB, "07911 123456"));
        assert!(!national_phone_valid(Id::GB, "0000000000"));
        assert!(!national_phone_valid(Id::GB, "0123456789"));
        assert!(national_phone_valid(Id::DE, "030 12345678"));
        assert!(national_phone_valid(Id::DE, "0171 1234567"));
        assert!(!national_phone_valid(Id::DE, "0000000000"));
        // NOTE: the validator is not a locale *discriminator* — national numbering plans
        // overlap, so a number valid in one region can be valid in another (e.g.
        // `07911 123456` validates as DE too). That is privacy-safe: over-masking a real
        // phone is never a leak. But some region-specific numbers still fail elsewhere —
        // a London geographic number is not a valid DE number:
        assert!(!national_phone_valid(Id::DE, "020 7946 0958"));
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
            let masked = vault.mask_all(&input, &detector, &Budget::per_call()).unwrap();

            let leftovers = detector.detect(&masked);
            proptest::prop_assert!(
                leftovers.is_empty(),
                "structured PII survived masking of {:?} → {:?}: {:?}",
                input, masked, leftovers
            );
        }
    }

    // -----------------------------------------------------------------------
    // VAT / tax identifiers (M11 Track A)
    //
    // The corpus model is PHONE-NAT's, unchanged: a positive set of REAL published
    // renderings and an adversarial negative set of things that merely look like one.
    // A category ships when it is measured.
    // -----------------------------------------------------------------------

    /// **VAT-01 — the Italian Partita IVA check digit, against real published numbers.**
    ///
    /// Six P.IVAs taken from Italian company registrations. They are the anchor for the whole
    /// recognizer: an algorithm that reproduces six independent real check digits is an
    /// implementation of the scheme, where one that reproduces a single hand-picked number is
    /// a plausible transcription of it. (These identify *companies*, are printed on every
    /// invoice those companies issue, and are public registry data.)
    #[test]
    fn italian_piva_accepts_real_published_numbers() {
        for piva in [
            "00905811006", // ENI
            "00159560366", // Ferrari
            "00488410010", // TIM
            "00891030272", // Luxottica
            "00811720580", // Enel
            "07973780013", // Stellantis Italy
        ] {
            assert!(
                it_piva_valid(piva),
                "{piva} is a real P.IVA and must validate"
            );
        }
    }

    /// **VAT-02 — a wrong check digit is rejected, for every shipped country.**
    ///
    /// The negative half of VAT-01: each of these is a real number with its final digit moved
    /// by one. Without this the recognizers would be "accepts everything of the right length",
    /// which is the shape M4-R1 named FP-prone and the reason this tier is checksum-gated.
    #[test]
    fn vat_check_digits_reject_a_moved_digit() {
        assert!(!it_piva_valid("00905811007"));
        assert!(!it_piva_valid("00159560367"));
        assert!(!de_vat_valid("136695975"));
        assert!(!de_vat_valid("115235682"));
        assert!(!gb_vat_valid("220430232"));
        assert!(!gb_vat_valid("123456783"));
        assert!(!pt_nif_valid("524287245"));
        // Wrong length is rejected before any arithmetic runs.
        assert!(!it_piva_valid("0090581100"));
        assert!(!it_piva_valid("009058110066"));
        assert!(!de_vat_valid("13669597"));
        assert!(!gb_vat_valid("2204302311"));
    }

    /// **VAT-03 — every shipped country's VIES form is detected end to end.**
    ///
    /// Real published VAT numbers, run through the whole recognizer set rather than the
    /// validators alone, so the regex, the word boundaries and the overlap resolver are all in
    /// the path. Five countries ship because five are measured; ES, FR and LV are deliberately
    /// absent and VAT-04 pins that they stay absent rather than half-working.
    #[test]
    fn vat_numbers_are_detected_for_every_shipped_country() {
        for (text, want) in [
            ("fattura a IT00905811006 grazie", "IT00905811006"), // IT — ENI
            ("Rechnung an DE136695976 bitte", "DE136695976"),    // DE — documented vector
            ("Rechnung an DE115235681 bitte", "DE115235681"),    // DE — Volkswagen
            ("invoice GB220430231 please", "GB220430231"),       // GB — Tesco
            ("invoice GB123456782 please", "GB123456782"),       // GB — worked example
            ("factura PT524287244 obrigado", "PT524287244"),     // PT
            ("factura PT504499777 obrigado", "PT504499777"),     // PT — Galp
            ("factuur NL111222333B01 dank", "NL111222333B01"),   // NL — 11-proef body
        ] {
            assert_eq!(
                kinds(text),
                vec![(PiiKind::TaxId, want.to_string())],
                "{text} must yield exactly one TaxId span"
            );
        }
    }

    /// **VAT-04 — the countries that did NOT ship are not half-shipped.**
    ///
    /// ES, FR and LV VAT numbers are real and well-formed; this repo does not recognize them
    /// because their checksums are not measured here, and an unmeasured recognizer does not go
    /// in — the rule that produced nine phone regions rather than a table of guesses. Asserting
    /// it keeps the gap **documented and deliberate** instead of something a future reader
    /// discovers as a bug. (An ES/FR/LV number whose digits happen to satisfy a national-ID
    /// checksum may still be masked by that tier; these are chosen not to be.)
    ///
    /// **An absence assertion is only worth its reachability, and the ES arm had none
    /// (M11-R1).** It read `"ES B12345678"` — with a space. The tier documents and enforces
    /// that there is *no* space between prefix and body, so that input was absent for a reason
    /// having nothing to do with ES not shipping: a live, checksum-less ES recognizer — exactly
    /// the half-shipped outcome this guard exists to forbid — left it green. FR and LV were
    /// fine; ES was the one broken arm, and it was the one whose failure mode the recognizer's
    /// own doc describes in detail.
    ///
    /// **So the negatives are now checked for reachability first**, against the tier's actual
    /// grammar: a two-letter uppercase prefix followed by an unbroken run of alphanumerics. A
    /// negative the grammar could never match is red on the spot rather than quietly true — the
    /// same "prove the corpus can express the thing" move PROP-04 made for M4-R17. Writing a
    /// negative with a space in it is now a failure, not a silent pass.
    ///
    /// The reachability decision is a **pure function** ([`vat_grammar_could_match`]) with its
    /// matrix below, kept inside this test rather than promoted to a guard of its own — a guard
    /// on a guard is worth one level, not two. The first version was written inline and was
    /// subtly wrong in a way the `&&` hid: `chars.by_ref().take(2).all(…)` short-circuits on a
    /// failing first character, so the "body" it then examined started one character too early.
    /// The outcome was still correct, which is exactly why it needed extracting rather than
    /// re-reading.
    #[test]
    fn unmeasured_vat_countries_are_absent_rather_than_guessed() {
        // The matrix. The last four rows are the ones that matter: the M11-R1 defect itself, and
        // the degenerate inputs where an inline `take(2)` reads as vacuously true.
        for reachable in [
            "ESB12345678",
            "ES12345678Z",
            "FR40404833048",
            "NL123456789B01",
        ] {
            assert!(
                vat_grammar_could_match(reachable),
                "{reachable} is reachable"
            );
        }
        for unreachable in [
            "ES B12345678", // a space — the M11-R1 defect itself
            "es12345678z",  // lowercase prefix: the patterns are uppercase-only (VAT-05)
            "E12345678Z",   // one prefix letter, not two
            "ESB1234567-8", // punctuation inside the body
            "ES1234",       // body too short for any shipped scheme
            "ES",           // prefix only
            "E",            // shorter than the prefix
            "",             // empty
        ] {
            assert!(
                !vat_grammar_could_match(unreachable),
                "{unreachable:?} must NOT count as reachable — treating it as reachable is how \
                 an absence assertion becomes vacuous (M11-R1)"
            );
        }

        for text in [
            "ESB12345678", // ES — the legal-entity CIF form, the case the comment names
            "ES12345678Z", // ES — the person form (DNI + control letter)
            "ESX1234567L", // ES — the foreigner form (NIE)
            "FR40404833048",
            "LV40003032949",
        ] {
            // Reachability: could the tier's grammar match this token at all? If a negative
            // cannot satisfy that shape, its absence proves nothing about the country and
            // everything about the punctuation (M11-R1).
            assert!(
                vat_grammar_could_match(text),
                "{text} is not a token the VAT grammar could ever match — two uppercase ASCII \
                 letters then an unbroken alphanumeric run, no space. Its absence would prove \
                 nothing about whether the country ships (M11-R1)."
            );

            let got = kinds(text);
            assert!(
                !got.iter().any(|(k, _)| *k == PiiKind::TaxId),
                "{text} must not be recognized as a TaxId — {got:?} — its country's checksum \
                 is not measured here (ROADMAP M11 Track A)"
            );
        }
    }

    /// **VAT-05 — the span never eats the word before the digits.** (Rewritten for M11-R10;
    /// it used to assert the opposite of what ships.)
    ///
    /// **The rationale this guard carried was refuted by the tier's own grammar.** It read: the
    /// prefixes are uppercase-only because lowercased, `it` is one of the commonest words in
    /// English, so `"call it <11 digits>"` would produce a 14-character span swallowing a word.
    /// But the recognizer table forbids **any space between prefix and digits**, so `it 12345678901`
    /// cannot match under *any* case rule — the danger the argument described was already
    /// impossible, and what the rule actually bought was a **leak**: `it00905811006` and
    /// `IT60x0542811101000000123456` matched nothing and went upstream in clear (M11-R10).
    ///
    /// The prefixes now fold case. What survives, and what this guard is really for, is the
    /// **span boundary**: a VAT number written after an English word must still yield the digits
    /// alone. That property is independent of the case rule, and it is the only one the original
    /// test's first assertion ever exercised.
    #[test]
    fn a_vat_span_never_swallows_the_word_before_it() {
        // The case the old rationale was written about — still correct, and now for a reason that
        // holds: no space is allowed between prefix and digits, so the span is the digits.
        assert_eq!(
            kinds("call it 00905811006 back"),
            vec![(PiiKind::TaxId, "00905811006".to_string())],
            "the span must be the digits alone — never `it 00905811006`"
        );
        // Same for an uppercase prefix: a space breaks the prefixed form in both cases.
        assert_eq!(
            kinds("VAT IT 00905811006 please"),
            vec![(PiiKind::TaxId, "00905811006".to_string())],
            "a space after the prefix must not be bridged"
        );
        // Glued to a preceding letter there is no ASCII word boundary at all, so nothing fires —
        // in either case. This is what stops a VAT number being found inside a longer token.
        assert!(kinds("XIT00905811006").is_empty());
        assert!(kinds("xit00905811006").is_empty());
        // And the leak M11-R10 found is closed in both directions.
        assert_eq!(
            kinds("fattura it00905811006"),
            vec![(PiiKind::TaxId, "it00905811006".to_string())],
            "a lowercase VIES prefix must be masked, not forwarded in clear (M11-R10)"
        );
    }

    /// **CASE-01 (M11-R10) — every letter-bearing recognizer has a recorded answer on the case
    /// axis, and the answer is asserted rather than assumed.**
    ///
    /// **This is the chokepoint the leak asked for.** M11-R10 was not one bug: seven of thirteen
    /// renderings of the same values went upstream **in clear** because the VAT prefixes and the
    /// IBAN pattern spelled their letters `[A-Z]`, while the Codice Fiscale, the ES DNI/NIE and
    /// the CN resident id folded case. Nobody had ever decided the axis — `iban_mod97`'s doc
    /// comment has promised to fold letters since M1, for input the regex could not deliver. The
    /// instance-shaped fix (add `it00905811006` to a corpus) would have closed IT and left DE, GB,
    /// PT, NL and IBAN open, which is the move [M4's retrospective] is six rounds of warning
    /// against.
    ///
    /// So the question is asked **once per pattern**: given a known positive, does the lowercase
    /// rendering behave like the uppercase one? A new letter-bearing recognizer cannot ship
    /// without an entry here, and an entry cannot be added without stating the answer — the same
    /// move `shipped_tax_recognizer_count` makes for the count and `pii_kinds!` for the enum.
    ///
    /// **`MUST_MATCH` is not decoration: `Some(kind)` means the lowercase form must be detected
    /// *as that kind*.** A recognizer that silently stopped folding case would go red here even
    /// though its uppercase corpus tests all stay green — which is exactly the state the tier was
    /// in before this guard existed.
    #[test]
    fn every_letter_bearing_recognizer_answers_the_case_axis() {
        // (a known positive, the kind its UPPERCASE form yields, why it is here)
        const MATRIX: &[(&str, PiiKind)] = &[
            // —— the two families M11-R10 found leaking ——
            ("IT00905811006", PiiKind::TaxId),
            ("DE136695976", PiiKind::TaxId),
            ("GB220430231", PiiKind::TaxId),
            ("PT524287244", PiiKind::TaxId),
            ("NL111222333B01", PiiKind::TaxId),
            ("IT60X0542811101000000123456", PiiKind::Iban),
            ("DE89370400440532013000", PiiKind::Iban),
            // —— the families that already folded case, kept so a regression is visible ——
            ("RSSMRA85T10A562S", PiiKind::NationalId), // Codice Fiscale
            ("X1234567L", PiiKind::NationalId),        // ES NIE
        ];

        for (upper, kind) in MATRIX {
            let lower = upper.to_lowercase();
            assert_ne!(
                &lower, upper,
                "{upper} carries no letter — it does not belong in a case matrix"
            );

            assert_eq!(
                kinds(upper),
                vec![(*kind, (*upper).to_string())],
                "{upper} must be detected in its canonical uppercase rendering"
            );
            assert_eq!(
                kinds(&lower),
                vec![(*kind, lower.clone())],
                "{lower} is the same value in lowercase and must be masked the same way. \
                 Before M11-R10 this reached the provider IN CLEAR: the letters are ASCII word \
                 characters, so there is no ASCII word boundary for a shorter recognizer to fall back on \
                 — the span does not shrink, it disappears."
            );

            // The sharpest case, and the one a corpus of lowercase strings would miss entirely:
            // an otherwise canonical value with exactly ONE letter flipped.
            let flipped: String = {
                let mut out = String::with_capacity(upper.len());
                let mut done = false;
                for c in upper.chars() {
                    if !done && c.is_ascii_uppercase() && !out.is_empty() {
                        out.extend(c.to_lowercase());
                        done = true;
                    } else {
                        out.push(c);
                    }
                }
                out
            };
            if flipped != *upper {
                assert_eq!(
                    kinds(&flipped),
                    vec![(*kind, flipped.clone())],
                    "{flipped} differs from {upper} by a single letter's case and must still be \
                     masked (M11-R10: `IT60x0542811101000000123456` was forwarded in clear)"
                );
            }
        }
    }

    /// **CASE-02 (M11-R10) — folding case on IBAN does not open the false-positive door it was
    /// keeping shut.**
    ///
    /// The IBAN half of the axis could not simply be folded. An IBAN has **no hard checksum gate**
    /// — M4 decided on purpose that a structurally valid one is masked even when mod-97 fails —
    /// so widening `[A-Z]` to `[A-Za-z]` would have swept in every hex digest and base64 blob in
    /// reach. Measured over 341.1 MB / 16 380 files of third-party source: **1 match uppercase,
    /// 150 case-folded**. Masking a hex digest inside a `tool_use.input` is the functional harm
    /// M10 spent nine rounds bounding.
    ///
    /// So [`iban_case_gate`] splits the rule by rendering: canonical uppercase keeps M4's
    /// behaviour untouched, while a rendering carrying **any** lowercase letter must be fully
    /// verifiable — mod-97 *and* the ISO 13616 length. Measured residue: **0 of the 149 added**.
    /// This pins both halves of that split, because getting either wrong is silent.
    #[test]
    fn a_lowercase_iban_is_masked_only_when_it_verifies() {
        // Canonical uppercase: M4's rule, unchanged. Structurally valid, mod-97 fails, masked
        // anyway and flagged `Structural`.
        let bad_checksum = "DE89370400440532013001";
        assert!(!iban_mod97(bad_checksum), "this fixture must fail mod-97");
        assert_eq!(
            kinds(bad_checksum),
            vec![(PiiKind::Iban, bad_checksum.to_string())],
            "an uppercase IBAN whose mod-97 fails is still masked — that is M4's decision and              folding case must not disturb it"
        );

        // The same string in lowercase is NOT masked: it is the shape a hex digest also has, and
        // nothing verifies it. This is the false-positive door staying shut.
        assert!(
            kinds(&bad_checksum.to_lowercase()).is_empty(),
            "a lowercase IBAN-shaped string that fails mod-97 must not be masked — 341 MB of              real source yields 149 such strings, and masking them is the M10 harm"
        );

        // But a lowercase IBAN that really verifies IS masked — the leak M11-R10 found.
        let good = "de89370400440532013000";
        assert!(iban_mod97(good) && iban_length_ok(good));
        assert_eq!(
            kinds(good),
            vec![(PiiKind::Iban, good.to_string())],
            "a verifiable lowercase IBAN must be masked, not forwarded in clear (M11-R10)"
        );
    }

    /// **VAT-06 — the tier is always on, regardless of `PII_LOCALES`.**
    ///
    /// The national-ID posture (M4-R1), not the FP-prone phone tier's. A VAT number that
    /// reaches the proxy is masked even when its country is not configured — and setting the
    /// variable to something else, or to a country that is not this one, cannot switch it off.
    /// **There is no configuration variable for this tier**, deliberately (ROADMAP M11 Track A,
    /// decision 2); this is the guard that fails if one is ever introduced by accident.
    #[test]
    fn vat_is_always_on_regardless_of_locales() {
        for locales in [vec!["us"], vec!["zz"], vec![], vec!["cn", "de"]] {
            assert_eq!(
                kinds_with(&locales, "fattura IT00905811006"),
                vec![(PiiKind::TaxId, "IT00905811006".to_string())],
                "locales {locales:?} must not gate the VAT tier"
            );
            // Ferrari's P.IVA — chosen because it is NOT also a valid Latvian personal code
            // or German Steuer-ID, so this asserts the VAT tier rather than the collision
            // (which VAT-10 measures and VAT-14 pins).
            assert_eq!(
                kinds_with(&locales, "P.IVA 00159560366"),
                vec![(PiiKind::TaxId, "00159560366".to_string())],
                "locales {locales:?} must not gate the bare P.IVA form"
            );
        }
    }

    /// **VAT-07 — the recognizers fire in CJK prose (M4-R13).**
    ///
    /// Chinese and Japanese have no inter-word spaces, so a VAT number glued to a Han character
    /// is the *natural* rendering, not an evasion. With Rust `regex`'s default Unicode `\b` a
    /// Han character is a word character, so there would be **no boundary** before the `I` and
    /// the whole tier would be silently inert in CJK text — which is exactly how this repo once
    /// shipped inert card and ID recognizers. `(?-u:\b)` is what keeps it alive.
    #[test]
    fn vat_is_not_inert_in_cjk_prose() {
        assert_eq!(
            kinds("我的增值税号是IT00905811006"),
            vec![(PiiKind::TaxId, "IT00905811006".to_string())]
        );
        assert_eq!(
            kinds("增值税号00905811006是这个"),
            vec![(PiiKind::TaxId, "00905811006".to_string())]
        );
    }

    /// **VAT-08 — a VAT number must not swallow an adjacent number, and must not be swallowed.**
    ///
    /// The word-boundary half of the contract. Two VAT numbers separated by a single space stay
    /// two spans; a VAT number inside a longer ASCII token is not a match at all (the
    /// anti-false-positive guarantee `(?-u:\b)` preserves exactly — a hash, a UUID or a base64
    /// blob cannot contain one).
    #[test]
    fn vat_does_not_swallow_or_get_swallowed_by_an_adjacent_token() {
        assert_eq!(
            kinds("IT00905811006 IT00159560366"),
            vec![
                (PiiKind::TaxId, "IT00905811006".to_string()),
                (PiiKind::TaxId, "IT00159560366".to_string()),
            ]
        );
        // Inside a longer ASCII run: no boundary, no match.
        assert!(kinds("refIT00905811006x").is_empty());
        assert!(kinds("a00905811006b").is_empty());
        // A 12-digit run contains no 11-digit token with boundaries on both sides.
        assert!(kinds("009058110066").is_empty());
    }

    /// **VAT-09 (measured, not asserted-by-intuition) — the bare P.IVA over-mask rate.**
    ///
    /// A mod-10 check accepts about one arbitrary 11-digit number in ten, so the bare Italian
    /// form masks a fraction of ordinary 11-digit tokens. That cost is **accepted on purpose**
    /// (over-mask, never leak — and the vault restores the value on the response path), the
    /// same trade M4-R6 took for the 9- and 11-digit national-ID recognizers. What is *not*
    /// acceptable is quoting a rate nobody measured, so this measures it over deterministic
    /// sweeps and pins it to a band.
    ///
    /// **What the contiguous sweep does and does not pin (M11-R5).** This guard used to run one
    /// sweep, over 100 000 *consecutive* 11-digit numbers, and claim that "a change that moved
    /// the rate would have to edit this number to land". It could not: for **any** scheme whose
    /// eleventh digit is a function of the first ten, exactly one value per block of ten passes,
    /// so the rate is `0.100` *by construction* whatever the arithmetic does. Replacing the
    /// final comparison with `d[10] == 7` still printed `0.100` and still passed. What sweep 1
    /// really pins is the **shape** — eleven digits, the last a check digit, no context required
    /// — which is load-bearing but is not the checksum.
    ///
    /// **Sweep 2 is the negative control that separates them**, and it is why this guard can now
    /// go red. Hold the check digit *constant* and vary the first ten digits: a correct mod-10
    /// accepts ~1 in 10 of that space too, while a stub comparing against a fixed digit accepts
    /// either **all** of it or **none** — 0.0 or 1.0, nowhere near the band. Two sweeps whose
    /// rates agree is evidence about the arithmetic; one sweep is evidence about the format.
    ///
    /// **And the number survives contact with real text.** M11-R1's round measured the shipped
    /// recognizer over 104 uncurated `\d{11}` tokens found in ~304 MB of third-party source:
    /// **9 accepted, 0.0865** — see `TESTING.md` → VAT-09/VAT-15.
    #[test]
    fn vat_over_mask_rate_on_arbitrary_eleven_digit_numbers() {
        let total = 100_000u32;

        // Sweep 1 — contiguous. Pins the SHAPE: one accept per block of ten.
        let hits = (0..total)
            .filter(|i| it_piva_valid(&format!("{:011}", 10_000_000_000u64 + u64::from(*i))))
            .count();
        let rate = f64::from(u32::try_from(hits).unwrap()) / f64::from(total);
        eprintln!("bare P.IVA accepts {hits}/{total} arbitrary 11-digit numbers ({rate:.3})");
        assert!(
            (0.08..=0.12).contains(&rate),
            "mod-10 should accept ~1 in 10; measured {rate:.3} over {total} — if this moved, \
             the shipped over-mask cost moved with it"
        );

        // Sweep 2 — the check digit HELD CONSTANT while the first ten digits vary. This is the
        // one a wrong checksum cannot survive (M11-R5): `d[10] == <const>` accepts 0.0 or 1.0
        // here, and only arithmetic that actually depends on the first ten digits lands in the
        // band. The two rates agreeing is what makes the claim about the *checksum* rather than
        // about the format.
        for fixed in [0u64, 7] {
            let hits = (0..total)
                .filter(|i| {
                    let body = 1_000_000_000u64 + u64::from(*i);
                    it_piva_valid(&format!("{body:010}{fixed}"))
                })
                .count();
            let held = f64::from(u32::try_from(hits).unwrap()) / f64::from(total);
            eprintln!(
                "bare P.IVA accepts {hits}/{total} 11-digit numbers whose check digit is fixed \
                 at {fixed} ({held:.3})"
            );
            assert!(
                (0.08..=0.12).contains(&held),
                "with the check digit held at {fixed}, a correct mod-10 still accepts ~1 in 10 \
                 of the first-ten-digit space; measured {held:.3}. A rate of ~0.0 or ~1.0 here \
                 means the eleventh digit is being compared against a constant rather than \
                 computed — which sweep 1 cannot see (M11-R5)."
            );
        }
    }

    /// **VAT-10 (measured) — how often a valid P.IVA is ALSO a valid 11-digit national ID.**
    ///
    /// This is the input that makes `PiiKind::priority`'s placement observable: the bare P.IVA
    /// pattern is byte-identical to the always-on `\d{11}` national-ID pattern, so a token
    /// satisfying both produces two identical spans and priority decides only which one *names*
    /// the union. Both are masked either way (M4-R10/R11), so the rate is a **labelling**
    /// statistic, not a coverage one — but it is the number a reader needs in order to know
    /// whether the naming rule matters in practice or is a curiosity.
    ///
    /// **This measures the SMALLER of the two collisions, and used to be quoted as if it were
    /// the only one (M11-R2).** Its sweep runs `10_000_000_000 + i`, so every value it looks at
    /// starts with `1` — while the only phone family that can claim a bare digit run needs a
    /// leading `0`. The phone collision is therefore outside this guard's range *by
    /// construction*, and it is much the larger. `VAT-17` measures that one; the two together
    /// are the labelling picture, and neither alone is.
    #[test]
    fn vat_and_natid_collision_rate() {
        let mut piva = 0u32;
        let mut both = 0u32;
        for i in 0..200_000u64 {
            let s = format!("{:011}", 10_000_000_000u64 + i);
            if it_piva_valid(&s) {
                piva += 1;
                if eleven_digit_id_valid(&s) {
                    both += 1;
                }
            }
        }
        let share = f64::from(both) / f64::from(piva);
        eprintln!(
            "{both}/{piva} valid P.IVAs are also a valid DE Steuer-ID or LV code ({share:.4}) \
             — those are named [NATID_n], the rest [TAXID_n]"
        );
        // The point of the guard is that the two sets genuinely differ: most valid P.IVAs are
        // NOT national IDs, which is the coverage this recognizer adds.
        assert!(
            share < 0.5,
            "if most P.IVAs were already national IDs this recognizer would be adding almost \
             nothing; measured {share:.4}"
        );
        assert!(
            piva > 1_000,
            "the sweep must actually find P.IVAs, found {piva}"
        );
    }

    /// **VAT-17 (measured, M11-R2) — how often a bare P.IVA is named `[PHONE_n]` under the
    /// configuration that actually ships.**
    ///
    /// **The collision the milestone fixed the direction of without ever measuring the size of.**
    /// `VAT-14` established that `TaxId` must rank below `Phone` — a numbering-plan lookup
    /// confirming an *assigned* number is better evidence than a mod-10 check one arbitrary
    /// 11-digit number in ten satisfies — and that ordering stands. What nobody had was the
    /// magnitude, so nobody chose it: the two instances `VAT-14` pins are the ones a review round
    /// happened to find, and `VAT-10`, the guard ROADMAP presents as *the* collision number,
    /// cannot see this collision at all (its sweep is `1`-leading; the phone `Trunk` family's
    /// separator-free arm `\b0\d{6,11}\b` needs a `0`).
    ///
    /// **And every other VAT guard sits inside the immune sub-shape, which is why they are all
    /// green.** A leading `00` reads to libphonenumber as the international access code and is
    /// rejected — and all five real published P.IVAs this repo's corpus is built on
    /// (`00905811006`, `00159560366`, `00488410010`, `00891030272`, `00811720580`) are
    /// `00…`-leading. *A corpus has a shape, and that shape is a blind spot* — M4's lesson 2,
    /// landing again in the milestone that quotes it. So this sweeps the **issuable** space
    /// deliberately: a 0-leading serial and a plausible province code, both leading pairs
    /// reported separately, because the split is the whole explanation.
    ///
    /// This is a **labelling** statistic like `VAT-10`, not a coverage one — every byte is masked
    /// under either name. What it costs is decision 1's entire purpose: a token that tells a
    /// consumer *business identifier* rather than *person*. **The maintainer settled the
    /// ordering on 2026-09-03 (M11-R8): it stands** — ROADMAP → M11 Track A, decision 4, with
    /// the accepted cost and both rejected alternatives. The reason the obvious refinement was
    /// rejected is worth carrying here, because this is where somebody would try it: yielding
    /// the separator-free arm to `TaxId` would relabel `02079460958` and `03012345678`, which
    /// **are themselves separator-free** — it undoes `VAT-14` rather than narrowing it. What
    /// this guard now does is hold the accepted number still.
    #[test]
    fn bare_piva_phone_collision_rate_under_the_shipped_default() {
        let detector = StructuredRecognizers::new();
        assert_eq!(
            vetted_phone_regions().len(),
            9,
            "this rate is a property of the shipped region set"
        );

        // The issuable bare form: `{serial:07}{province:03}{check}`, serial < 1_000_000 so the
        // token is 0-leading, province in the assigned 001..=110 band. Striding the serial keeps
        // the sample spread across both leading pairs rather than clustered at the bottom.
        let mut piva = 0u32;
        let (mut phoned, mut zero_zero, mut zero_zero_phoned) = (0u32, 0u32, 0u32);
        for i in 0..1_500u64 {
            let serial = (i * 661) % 1_000_000;
            let province = 1 + (i % 110);
            for check in 0..10u64 {
                let s = format!("{serial:07}{province:03}{check}");
                if !it_piva_valid(&s) {
                    continue;
                }
                piva += 1;
                let is_00 = s.starts_with("00");
                if is_00 {
                    zero_zero += 1;
                }
                if detector.detect(&s).iter().any(|e| e.kind == PiiKind::Phone) {
                    phoned += 1;
                    if is_00 {
                        zero_zero_phoned += 1;
                    }
                }
            }
        }

        let share = f64::from(phoned) / f64::from(piva);
        let other = piva - zero_zero;
        eprintln!(
            "{phoned}/{piva} issuable bare P.IVAs are named [PHONE_n] under the shipped default \
             ({share:.3}) — 00xx: {zero_zero_phoned}/{zero_zero}, 0[1-9]xx: {}/{other}",
            phoned - zero_zero_phoned
        );

        assert!(
            piva > 1_000,
            "the sweep must actually find P.IVAs, found {piva}"
        );
        // A band, not a point: this number is a property of libphonenumber's metadata as much as
        // of this code, and a dependency bump moving it is exactly what should be noticed.
        assert!(
            (0.55..=0.90).contains(&share),
            "most issuable bare P.IVAs are named [PHONE_n], not [TAXID_n]; measured {share:.3}. \
             If this moved a lot, either the priority order changed (that is VAT-14's subject and \
             a product-visible decision) or the phone tier's region set or metadata did — both \
             need explaining before this number is edited."
        );
        // The split is the explanation, and it must stay visible: a leading `00` is read as an
        // international access code and rejected, which is precisely why every other VAT guard —
        // all of them built on `00…`-leading real numbers — is green.
        let zero_zero_share = f64::from(zero_zero_phoned) / f64::from(zero_zero);
        assert!(
            zero_zero > 100 && zero_zero_share < 0.15,
            "the `00…` sub-shape is supposed to be nearly immune ({zero_zero_phoned}/{zero_zero} \
             = {zero_zero_share:.3}) — if it stopped being, the corpus every other VAT guard is \
             built on has moved out of the blind spot and those guards mean something different"
        );
    }

    /// **VAT-11 — the Dutch btw-id splits `Verified` from `Structural`, and both are masked.**
    ///
    /// NL is the one shipped country whose scheme has nothing to check: the 2020 sole-trader
    /// btw-id is randomized by design, while a legal entity's 9-digit body is an RSIN that
    /// satisfies the 11-proef. Rather than claim a verification the scheme cannot support, the
    /// recognizer accepts on **format** and [`confidence_of`] tells the truth about which one
    /// it got — exactly as an IBAN whose mod-97 fails is masked and tagged `Structural` (M4).
    #[test]
    fn nl_vat_confidence_splits_verified_from_format_only() {
        let entities = StructuredRecognizers::new().detect("btw NL111222333B01 en NL123456789B01");
        assert_eq!(entities.len(), 2, "both forms must be masked: {entities:?}");
        assert_eq!(entities[0].kind, PiiKind::TaxId);
        assert_eq!(entities[0].confidence, Confidence::Verified);
        assert_eq!(entities[1].kind, PiiKind::TaxId);
        assert_eq!(entities[1].confidence, Confidence::Structural);
        // The literal `B` is load-bearing: without it the format is not the format.
        assert!(kinds("NL111222333X01").is_empty());
        assert!(kinds("NL111222333B0").is_empty());
    }

    /// **VAT-14 — the bare P.IVA pattern must not relabel a phone number or a national ID.**
    ///
    /// **This is the guard for the defect this track nearly shipped.** A bare Partita IVA is
    /// `\d{11}`, and two always-on tiers already claim that shape: compact domestic phone
    /// numbers (a real London number `02079460958` and a real Berlin one `03012345678` both
    /// satisfy the P.IVA mod-10) and the 11-digit national IDs. With `TaxId` ranked above
    /// them, every compact GB and DE number M10 measured silently became `[TAXID_n]` — no
    /// leak, since the bytes are masked either way, but a **fidelity regression on a shipped,
    /// measured capability**, telling the model a phone number is a tax identifier.
    ///
    /// PHONE-NAT-01 caught it from the phone side. This pins it from the VAT side, where the
    /// change that would reintroduce it actually lives, and states the two principles that
    /// order the tiers: a numbering-plan lookup confirming an **assigned** number beats a
    /// mod-10 check, and a *person*-implying label beats a *business*-implying one.
    #[test]
    fn a_bare_piva_never_outranks_a_phone_or_a_national_id() {
        // Real domestic numbers that are ALSO mod-10-valid — the collision, not a contrivance.
        for (locale, number) in [("gb", "02079460958"), ("de", "03012345678")] {
            assert!(
                it_piva_valid(number),
                "{number} must satisfy the P.IVA checksum, or this guard is vacuous"
            );
            assert_eq!(
                kinds_with(&[locale], &format!("call {number} now")),
                vec![(PiiKind::Phone, number.to_string())],
                "a real assigned {locale} number must stay a Phone, not become a TaxId"
            );
        }
        // Enel's real P.IVA is also a valid Latvian personal code, so the person-implying
        // label wins. Masked either way — priority names the union, it never drops bytes.
        assert!(it_piva_valid("00811720580"));
        assert!(eleven_digit_id_valid("00811720580"));
        assert_eq!(
            kinds("P.IVA 00811720580"),
            vec![(PiiKind::NationalId, "00811720580".to_string())]
        );
    }

    /// **VAT-12 — a VAT number inside an email keeps the email's label, and nothing is dropped.**
    ///
    /// The overlap resolver's naming rule (M4-R10/R11): a union that is *exactly* an `Email`
    /// span is named by the email even though `TaxId` outranks it, because the email is the
    /// span that actually describes those bytes. What matters for privacy is the other half —
    /// every byte of both spans is masked, which the single returned span demonstrates.
    #[test]
    fn a_vat_inside_an_email_keeps_the_email_label() {
        assert_eq!(
            kinds("IT00905811006@example.com"),
            vec![(PiiKind::Email, "IT00905811006@example.com".to_string())]
        );
    }

    /// **VAT-13 — masking a VAT number reaches a fixpoint and the placeholder stays inert.**
    ///
    /// `[TAXID_1]` must not itself look like PII to the next pass, or `mask_all` would not
    /// converge (ARCHITECTURE → mask-to-a-fixpoint). Also pins the round trip: the value comes
    /// back byte-identical, which is what makes an over-mask harmless on the response path.
    #[test]
    fn a_masked_vat_number_is_inert_and_restores_exactly() {
        let detector = StructuredRecognizers::new();
        let mut vault = crate::pii::anonymizer::Vault::new();
        let masked = vault
            .mask_all(
                "fattura IT00905811006 e P.IVA 00159560366",
                &detector,
                &Budget::per_call(),
            )
            .expect("masking must converge");
        assert_eq!(masked, "fattura [TAXID_1] e P.IVA [TAXID_2]");
        assert!(
            detector.detect(&masked).is_empty(),
            "the placeholders must be inert: {masked}"
        );
        assert_eq!(
            vault.demask(&masked),
            "fattura IT00905811006 e P.IVA 00159560366"
        );
    }
}

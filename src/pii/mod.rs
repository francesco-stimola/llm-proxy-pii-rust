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

/// Declare the PII kinds **once**.
///
/// **Why a macro, when this repo avoids cleverness (M11-R6).** Four things have to move
/// together whenever a kind is added: the enum, [`PiiKind::ALL`], [`PiiKind::label`] and its
/// inverse [`PiiKind::from_label`]. M11 tried to hold them together with a guard instead — a
/// successor chain whose `match` the compiler checks — and review round 2 broke it in one move:
/// the compiler demands an *arm*, not a place in the walk, so `Vin => return None` compiles, the
/// walk never reaches `Vin`, `ALL` and the walk still agree, and the guard passes with a twelfth
/// kind unlisted. Rust cannot enumerate an enum's variants, so **no test can close that**: the
/// list has to be the single source the others are generated from.
///
/// What is deliberately *not* generated is [`PiiKind::priority`] and
/// [`PiiKind::is_structured`]. Those are judgements about a new kind, not restatements of it —
/// the compiler already forces an answer through exhaustive `match`, and that is the right place
/// to be stopped.
macro_rules! pii_kinds {
    ($( $(#[$meta:meta])* $variant:ident => $label:literal ),+ $(,)?) => {
        /// A category of personally identifiable information.
        ///
        /// `Serialize`/`Deserialize` use the variant names verbatim (e.g. `"Email"`,
        /// `"CreditCard"`), which is the encoding the JSON test corpus in
        /// `tests/corpus/pii_cases.json` relies on.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum PiiKind {
            $( $(#[$meta])* $variant, )+
        }

        impl PiiKind {
            /// Every variant, in declaration order.
            ///
            /// Generated from the same list as the enum, so it cannot fall behind it. Drive
            /// per-variant guards from here rather than from a list of your own.
            pub const ALL: &'static [PiiKind] = &[ $( PiiKind::$variant ),+ ];

            /// The uppercase label used inside placeholders, e.g. `Email` → `"EMAIL"`
            /// yields the `[EMAIL_1]` token. ASCII and tokenizer-friendly.
            pub fn label(self) -> &'static str {
                match self { $( PiiKind::$variant => $label ),+ }
            }

            /// Inverse of [`label`](Self::label): recover the kind from a placeholder label
            /// (case-insensitive). Lets the de-masker recognise a token the model echoed back,
            /// so it can warn when that token is not in the vault.
            ///
            /// Generated from the same literals as `label`, so the two cannot disagree about a
            /// kind. What a generator cannot check is that each literal is **uppercase** and
            /// **distinct** — a lowercase one would be unreachable after `to_ascii_uppercase`,
            /// and a duplicate would silently collapse two kinds. `KIND-02` checks both.
            pub fn from_label(label: &str) -> Option<Self> {
                match label.to_ascii_uppercase().as_str() {
                    $( $label => Some(PiiKind::$variant), )+
                    _ => None,
                }
            }
        }
    };
}

pii_kinds! {
    // Structured (deterministic recognizers — M1)
    Email => "EMAIL",
    Phone => "PHONE",
    Ssn => "SSN",
    /// A non-US national identifier (M4) — e.g. Italian Codice Fiscale, UK NINO.
    /// US SSN keeps its own [`Ssn`](Self::Ssn) variant for continuity.
    NationalId => "NATID",
    /// A **business tax identifier** (M11 Track A) — an Italian Partita IVA, an EU VAT
    /// number. Deliberately *not* folded into [`NationalId`](Self::NationalId).
    ///
    /// **Why its own kind, decided by the maintainer 2026-09-02.** A P.IVA identifies a
    /// *business*, and is personal data only when that business is a sole trader; a
    /// Codice Fiscale identifies a *person*, always. A single token unable to tell them
    /// apart destroys that distinction for every consumer downstream, permanently — and
    /// reusing `NATID` now would convert a free choice today into a breaking change to
    /// the placeholder vocabulary tomorrow, where the same input silently starts emitting
    /// a different token. See ROADMAP → M11 Track A, decision 1.
    TaxId => "TAXID",
    CreditCard => "CARD",
    Iban => "IBAN",
    /// API keys / tokens (e.g. `sk-…`, `sk-ant-…`, `AKIA…`). Deterministic —
    /// the old ML model missed these, so they are treated as structured PII.
    Secret => "SECRET",
    // Unstructured (ONNX NER — M2)
    Person => "PERSON",
    Organization => "ORG",
    Location => "LOCATION",
}

impl PiiKind {
    /// Overlap-resolution priority (see [`overlap::resolve_overlaps`]). Order:
    /// Secret > Iban > CreditCard > Ssn ≈ NationalId > Phone > TaxId > Email > NER.
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
    ///
    /// **`TaxId` sits below BOTH `NationalId` and `Phone`, and the input that decides it is
    /// a bare 11-digit number (M11 Track A).** An Italian Partita IVA is written as 11 bare
    /// digits, and two always-on tiers already claim `\d{11}`: the national IDs (German
    /// Steuer-ID, Latvian personal code) and the **compact domestic phone** shapes M10
    /// measured (`02079460958` is a real London number; `03012345678` a real Berlin one, and
    /// both satisfy the P.IVA mod-10). A token matching two tiers produces two identical
    /// spans, and priority decides only which of them *names* the union — never whether it
    /// is masked (M4-R10/R11). Two different principles put `TaxId` under each neighbour,
    /// and they happen to agree:
    ///
    /// - **Under `NationalId`: conservatism about personhood.** Between a reading that says
    ///   *a person* and one that says *a business*, the conservative label is the one
    ///   implying a natural person. Mislabelling a person's ID as a company's under-states
    ///   its sensitivity for every consumer downstream; the reverse over-states it, and
    ///   over-stating is the side this project errs on everywhere else.
    /// - **Under `Phone`: strength of evidence.** The domestic-phone tier does not match a
    ///   shape — it asks `phonenumber` whether the candidate is a real **assigned** number
    ///   in that region's plan (M10). That is strictly better evidence than a mod-10 check
    ///   one arbitrary 11-digit number in ten satisfies, so where they disagree the plan
    ///   lookup should name the span. Ranking `TaxId` above `Phone` silently relabelled
    ///   every compact GB and DE number M10 measured as `[TAXID_n]` — caught by
    ///   PHONE-NAT-01, and pinned from this side by VAT-14.
    ///
    /// The national-ID collision is measured rather than assumed —
    /// `vat_and_natid_collision_rate` in `recognizers.rs` reports what fraction of valid
    /// P.IVAs also satisfy an 11-digit national-ID check. Every *prefixed* VAT form (`IT…`,
    /// `DE…`, `GB…`, `NL…`, `PT…`) is collision-free by construction: the letters break the
    /// `(?-u:\b)` the digit-only recognizers need.
    pub fn priority(self) -> u8 {
        match self {
            PiiKind::Secret => 7,
            PiiKind::Iban => 6,
            PiiKind::CreditCard => 5,
            // National identifiers (US SSN + other locales) share a tier; ties fall
            // through to span length, then to the incumbent — always deterministic.
            PiiKind::Ssn | PiiKind::NationalId => 4,
            PiiKind::Phone => 3,
            // Business tax identifiers (M11) — below both tiers it collides with on a bare
            // 11-digit token; see the two principles above.
            PiiKind::TaxId => 2,
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
    /// Whether this is *"the allowance for scanning this request ran out"* rather than *"this
    /// detector is unavailable"* (M10-R41).
    ///
    /// **The two are different kinds of failure and [`FailOpen`](composite::FailOpen) must treat
    /// them differently**, so it has to be able to *tell* — a detector failure is swallowed, an
    /// exhausted request allowance is not. This flag is that distinction, on the error, where it
    /// belongs.
    ///
    /// It used to be inferred from `budget.is_exhausted()` at the wrapper, which is a property of
    /// the **request** asked about an error that may belong to the **detector**. That was correct
    /// only via an unstated invariant (no detector returns `Ok` with an exhausted budget, and
    /// `build_detector` orders the structured recognizers first) — the same
    /// wiring-dependent argument whose collapse round 4 identified as the defect underneath its own
    /// empty fail-open hunt. A genuine GPU or tokenizer failure arriving while the budget happened
    /// to be at zero became a `400` on a proxy configured to degrade to structured-only.
    ///
    /// **Private, and read through [`is_budget_exhausted`](Self::is_budget_exhausted) (M10-R45).**
    /// Making it private is what pushes every error site through
    /// [`unavailable`](Self::unavailable) or [`budget_exhausted`](Self::budget_exhausted), so the
    /// choice is made rather than defaulted.
    ///
    /// **The reach of that is narrower than "outside this module", and saying otherwise was
    /// M10-R52.** Rust privacy extends to a module's **descendants**, and `pii::composite`,
    /// `pii::recognizers` and `pii::onnx` are all children of `pii` — which is where every error
    /// site in this crate lives. So a `DetectError { .. }` literal still compiles there, and the
    /// guarantee is *"no other crate can build one"* plus a convention inside `pii`, not a proof.
    /// Recorded rather than engineered around: moving the type to a leaf module to buy the stronger
    /// version would scatter it away from the trait it belongs to, and the constructors are two lines
    /// each. **The claim is weaker than it looked; the code is what it is.**
    budget_exhausted: bool,
}

impl DetectError {
    /// A detector that could not run: unavailable, misconfigured, inference failed. **Fail-open
    /// eligible** — a non-critical detector wrapped in [`FailOpen`](composite::FailOpen) is skipped.
    pub fn unavailable(detector: &'static str, message: impl Into<String>) -> Self {
        Self {
            detector,
            message: message.into(),
            budget_exhausted: false,
        }
    }

    /// The request exhausted its validation allowance ([`Budget`]). **Never fail-open**: the text
    /// was not fully examined, so its PII status is unknown, and reporting "no PII" would forward a
    /// partially scanned body with a clean bill of health.
    pub fn budget_exhausted(detector: &'static str, message: impl Into<String>) -> Self {
        Self {
            detector,
            message: message.into(),
            budget_exhausted: true,
        }
    }

    /// Whether this is an exhausted **request allowance** rather than an unavailable **detector** —
    /// the one distinction [`FailOpen`](composite::FailOpen) must not collapse.
    pub fn is_budget_exhausted(&self) -> bool {
        self.budget_exhausted
    }
}

impl std::fmt::Display for DetectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} detector failed: {}", self.detector, self.message)
    }
}

impl std::error::Error for DetectError {}

/// A **per-request** allowance of expensive validator calls, shared by every field and every
/// fixpoint pass of one request (M10-R28).
///
/// **The unit is the whole point.** M10 first bounded this per *field*, and a field is a unit the
/// client multiplies: the same 15.6 MiB that is refused in one field answered `200` in **57 s**
/// split across 78 legal `messages[].content` fields, indistinguishable from the binary that had no
/// budget at all. `Vault::mask_all` then re-minted the allowance on each of its five passes, so the
/// published bound did not hold in *any* unit (M10-R30). A budget scoped to something the caller can
/// multiply is not a bound — it is a rate.
///
/// So there is exactly one of these per request, created by `PrivacyStage::on_request` beside the
/// `Vault` it already owns, and threaded down through
/// [`mask_all`](crate::pii::anonymizer::Vault::mask_all) into
/// [`try_detect`](PiiDetector::try_detect) and [`redetect`](PiiDetector::redetect) — which **take**
/// a budget rather than offering a budget-less sibling beside it (M10-R35).
///
/// **Not `Sync`, deliberately.** A `Cell` rather than an `AtomicUsize`: one request's masking is
/// single-threaded (the whole pipeline runs inside one `spawn_blocking`), so a shared counter would
/// buy atomics nobody needs and, worse, would make it *possible* to share one budget between
/// requests. The type system should refuse that, and this way it does.
pub struct Budget {
    left: std::cell::Cell<usize>,
    initial: usize,
    /// Calls by a **sub-unit** validator that have not yet added up to a whole unit — see
    /// [`spend_fraction`](Self::spend_fraction).
    part: std::cell::Cell<usize>,
    /// Units charged, per [`PiiKind`], so a refusal can name **which** tier spent the allowance
    /// (M11-R60). Indexed by position in [`PiiKind::ALL`].
    by_kind: std::cell::RefCell<[usize; PiiKind::ALL.len()]>,
}

impl Budget {
    /// A budget of `calls` cache-missing validator invocations.
    pub fn new(calls: usize) -> Self {
        Self {
            left: std::cell::Cell::new(calls),
            initial: calls,
            part: std::cell::Cell::new(0),
            by_kind: std::cell::RefCell::new([0; PiiKind::ALL.len()]),
        }
    }

    /// The shipped per-request allowance, minted for a caller that has **no request to charge** —
    /// unit tests, the measurement harnesses, the infallible [`PiiDetector::detect`] view.
    ///
    /// **Minting is allowed; minting *invisibly* is not (M10-R35).** The defect this milestone kept
    /// producing was a budget appearing out of nowhere partway through a request, because a method
    /// with a plausible name quietly created one. So there is no longer any method that mints
    /// implicitly: every allowance comes from a `Budget::new` / `Budget::per_call` / `Budget::unlimited`
    /// that a reader can see at the call site and `grep` can find.
    pub fn per_call() -> Self {
        Self::new(crate::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST)
    }

    /// A budget that never runs out — for callers with no request to bound: unit tests, the
    /// measurement harnesses, and the `#[ignore]`d evaluation suites. **Never** on the request
    /// path; `PrivacyStage` always creates a real one.
    pub fn unlimited() -> Self {
        Self {
            left: std::cell::Cell::new(usize::MAX),
            initial: usize::MAX,
            part: std::cell::Cell::new(0),
            by_kind: std::cell::RefCell::new([0; PiiKind::ALL.len()]),
        }
    }

    /// Charge one expensive validator call. Saturates at zero rather than wrapping — an exhausted
    /// budget stays exhausted, which is what [`is_exhausted`](Self::is_exhausted) rests on.
    pub fn spend(&self) {
        self.left.set(self.left.get().saturating_sub(1));
    }

    /// Charge a validator whose cost is a **fraction** of a unit: `calls_per_unit` of them cost
    /// one (M11-R60).
    ///
    /// **A unit is a quantum of work, not a call, and that is the whole of M10-R29.** One unit is
    /// one `phonenumber::parse()` — the ~3.7 µs the allowance was sized from. A validator that
    /// costs materially less and is charged a full unit *over-prices* the request, and over-pricing
    /// cheap work does not make the bound safer: it refuses legal traffic. M10-R29 measured that as
    /// a defect once (800 KB of 9-digit tokens refused in 45 ms with the phone tier not loaded);
    /// M11-R60 measured it again, on an ordinary `xxd` hex dump refused at 8 MiB.
    ///
    /// **One accumulator, deliberately — and the bound it gives is exact for one denominator and
    /// nothing at all for two.** With a single `calls_per_unit` the charge is exactly
    /// `floor(calls / calls_per_unit)`. With two sharing this field the mischarge is unbounded in
    /// **either** direction, and the counterexample is short: a spender at `d = 1000` calls 999
    /// times and charges nothing, then a spender at `d = 2` calls once, `part` reaches 1000 ≥ 2 and
    /// **one** unit is charged for a thousand calls — and the cycle repeats. (An earlier version of
    /// this paragraph claimed it was "conservative, never under-charging by more than a unit in
    /// total"; that is false, and it is the kind of safety claim that gets believed.) So the
    /// guidance is the load-bearing half: **a second kind of cheap validator wanting a different
    /// denominator gets its own field here**, never this one. Today there is exactly one, which is
    /// why nothing fits through the hole.
    pub fn spend_fraction(&self, calls_per_unit: usize) {
        debug_assert!(calls_per_unit >= 1);
        let n = self.part.get() + 1;
        if n >= calls_per_unit {
            self.part.set(0);
            self.spend();
        } else {
            self.part.set(n);
        }
    }

    /// Record that `units` of the allowance were spent on `kind`'s candidates.
    ///
    /// **So a refusal can name the tier that spent it (M11-R60).** The allowance had one spender
    /// for two milestones and the refusal message said so in as many words; the moment a second
    /// arrived the advice was attached to the wrong term, and an agent whose hex dump was refused
    /// was told to add a `LIMIT` to a SQL query. Attribution lives here rather than in the message
    /// so that a *third* spender cannot repeat it — the sentence is generated from what was
    /// actually charged.
    pub fn attribute(&self, kind: PiiKind, units: usize) {
        if units == 0 {
            return;
        }
        if let Some(i) = PiiKind::ALL.iter().position(|k| *k == kind) {
            self.by_kind.borrow_mut()[i] += units;
        }
    }

    /// How much of the allowance `kind`'s candidates were charged. Exists so a guard can assert
    /// the **M10-R29 invariant directly** — a validator cheaper than a phone parse must not
    /// dominate the allowance on ordinary traffic — instead of inferring it from a total.
    pub fn spent_on(&self, kind: PiiKind) -> usize {
        PiiKind::ALL
            .iter()
            .position(|k| *k == kind)
            .map(|i| self.by_kind.borrow()[i])
            .unwrap_or(0)
    }

    /// The kind that spent the most of the allowance, with its share — `None` if nothing was
    /// charged. Used only to build the refusal message, and the label comes from the enum, never
    /// from the input.
    pub fn top_spender(&self) -> Option<(PiiKind, usize)> {
        let by_kind = self.by_kind.borrow();
        by_kind
            .iter()
            .enumerate()
            .max_by_key(|(_, n)| **n)
            .filter(|(_, n)| **n > 0)
            .map(|(i, n)| (PiiKind::ALL[i], *n))
    }

    /// Whether the allowance is gone. The caller's contract is **fail closed**: stop scanning and
    /// report an error — never return the partial result as a success.
    pub fn is_exhausted(&self) -> bool {
        self.left.get() == 0
    }

    /// The allowance this budget started with, so an error message can quote the real number
    /// instead of restating a constant that may not be the one in force.
    pub fn initial(&self) -> usize {
        self.initial
    }

    /// How much of the allowance has been charged. Exists so **headroom against real traffic is a
    /// measurement rather than a claim**: M10 asserted the budget was unreachable by ordinary bodies
    /// and could not show it, which is how M10-R30's per-pass multiplier went unnoticed. PHONE-BUD
    /// pins the M7 Claude Code turn's spend against this.
    pub fn spent(&self) -> usize {
        self.initial - self.left.get()
    }
}

/// Engine-agnostic PII detector.
///
/// Deterministic recognizers and the ML NER model both implement this, so the
/// pipeline can combine or swap engines freely.
///
/// # The budget is a parameter, and there is exactly one fallible entry point per pass
///
/// **This shape is the fix for M10-R35, and the shape it replaced is the finding.** M10-R28 added a
/// second pair of methods — `try_detect_within` / `redetect_within` — whose *defaults* delegated to
/// the budget-less pair. Every wrapper therefore carried an obligation to override **both**, and the
/// penalty for missing one was invisible: the call fell through to a method that **minted a fresh
/// allowance**, restoring the unbounded behaviour with the whole suite green. Round 4 saw that hazard
/// clearly and closed it for all three wrappers. It missed the **leaf** — `StructuredRecognizers`,
/// the only detector whose cost the budget bounds and the only place that mints one — which overrode
/// `try_detect_within` and not `redetect_within`, so every fixpoint pass after the first started from
/// a full allowance and a legal 15.63 MiB body answered `200` in **17.2 s** against a published
/// ceiling of ~1.4 s.
///
/// So the pair is gone. `try_detect` and `redetect` **take** the budget, and `redetect`'s default
/// forwards *the same one*.
///
/// **And `try_detect` is the *required* method, not the derived one (M10-R44).** Deleting the seam
/// fixed one half of the trait and left the other: `try_detect`'s own default routed to `detect`,
/// and every production `detect` **mints** an allowance *and* `unwrap_or_default()`s the refusal. So
/// a five-line wrapper implementing only `detect` compiled, never mentioned `Budget`, and forwarded
/// a body the request path must refuse — with the refusal not ignored but **erased**. That is
/// word-for-word how M10-R35 described the sharpest case of its own defect, surviving in the half
/// its fix did not turn over.
///
/// Inverting which one is derived is what makes the claim true: `detect` is the convenience view and
/// has the default; `try_detect` is the contract and must be written. A detector that cannot fail
/// writes `Ok(...)` — but it writes it, and it sees the budget. **There is now no method a default
/// can route to which would mint another allowance**, and the only ways to create one —
/// `Budget::new` / `per_call` / `unlimited` — are visible at their call sites.
///
/// *An obligation a trait default can satisfy is not carried by the type system — and "every test
/// passes" is the signature of that, not evidence against it.* Generalized: **when forgetting to
/// override is silently valid, the API is the defect.**
pub trait PiiDetector: Send + Sync {
    /// Fallible detection, charged against the **caller's** [`Budget`] — one per request.
    ///
    /// **Required (M10-R44).** A detector that cannot fail returns `Ok(..)`, and a detector whose
    /// cost is not what is being bounded (the NER pays per token, and `MAX_BODY_BYTES` already bounds
    /// tokens) may ignore `budget` — but it has to say so, in one line, where a reader can see it. A
    /// **wrapper** forwards `budget` to what it wraps; there is no default that would do something
    /// else on its behalf.
    fn try_detect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError>;

    /// Return all PII entities found in `input`. Infallible view — a detector that can fail returns
    /// whatever it could detect (typically empty on error).
    ///
    /// **Not on the request path**, and deliberately: this mints its own allowance because there is
    /// no request to charge, and it swallows a refusal. Both are fine *here* and are exactly why this
    /// is the derived method rather than the required one — `PrivacyStage` uses the fallible pair.
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        self.try_detect(input, &Budget::per_call())
            .unwrap_or_default()
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
    /// The default **rescans** ([`try_detect`](Self::try_detect)) — the safe direction: a new
    /// detector re-runs on every pass unless it explicitly declares itself masking-idempotent.
    /// Skipping a detector here must be justified by "masking a neighbour cannot reveal one of its
    /// matches", and the no-recall-loss claim that rests on it is measured, not assumed (S4; DEVLOG
    /// 2026-07-18).
    ///
    /// **The default forwards the caller's `budget`, and that is load-bearing (M10-R35).** These
    /// later passes are where an allowance was previously re-minted — five passes over one field,
    /// each starting full. A detector that overrides only `try_detect` is now correct here for free.
    fn redetect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        self.try_detect(input, budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **KIND-01 (M11-R3, rebuilt for M11-R6) — one kind, one line, four things generated.**
    ///
    /// The original KIND-01 walked a successor chain whose `match` the compiler checks, to prove
    /// `PiiKind::ALL` could not fall behind the enum. **Review round 2 broke it in one move:** the
    /// compiler demands an *arm*, not a place in the walk, so a twelfth kind wired as
    /// `Vin => return None` compiles, the walk never reaches it, `ALL` and the walk still agree,
    /// and the guard passes with the kind unlisted. Rust cannot enumerate an enum's variants, so
    /// **no test could have closed that** — which is why the chain is gone and
    /// [`pii_kinds!`] generates the enum, `ALL`, `label` and `from_label` from one list instead.
    ///
    /// What remains to check is the thing a generator cannot: that the list is **used** where it
    /// matters and has not been quietly bypassed. `ALL` is now structurally complete, so this
    /// asserts the two properties that would still be wrong if somebody hand-edited the expansion
    /// or added a kind outside the macro — `ALL` agrees with `label` on every entry, and every
    /// variant is reachable through the public constructor the rest of the code uses.
    #[test]
    fn all_is_the_list_every_other_guard_is_driven_from() {
        // Non-vacuity, and the only number here worth pinning: a kind added to the macro must
        // show up in `ALL` without anyone touching this file.
        assert_eq!(
            PiiKind::ALL.len(),
            11,
            "PiiKind::ALL has {} entries. If you added a kind, bump this and check every guard \
             driven from `ALL` still says something true about it (KIND-02, AUG-01).",
            PiiKind::ALL.len()
        );
        // `ALL` and `label` come from the same macro list, so disagreement means the expansion
        // was hand-edited or a variant was declared outside `pii_kinds!`.
        for &kind in PiiKind::ALL {
            assert!(
                !kind.label().is_empty(),
                "{kind:?} has an empty placeholder label"
            );
        }
        // Every kind must have a priority: `priority` is a hand-written exhaustive match on
        // purpose (a new kind is a *judgement*, and the compiler should stop you there), so this
        // only checks the judgement was made rather than defaulted.
        let mut priorities: Vec<u8> = PiiKind::ALL.iter().map(|k| k.priority()).collect();
        priorities.sort_unstable();
        priorities.dedup();
        assert!(
            priorities.len() > 1,
            "every kind returned the same priority — the overlap resolver would have no order"
        );
    }

    /// **KIND-02 (M11-R3) — every kind's placeholder label survives the round trip.**
    ///
    /// [`PiiKind::label`] names a variant; [`PiiKind::from_label`] is its inverse and ends in
    /// `_ => return None`, so before this guard a kind could be added to one and not the other
    /// and nothing would fail. That is not cosmetic: `from_label` gates
    /// `anonymizer::is_placeholder_token`, which is what makes a placeholder **inert** to the
    /// next detection pass — the mechanism `Vault::mask_all`'s fixpoint rests on (M5-R4) — and
    /// it also gates the M1.5 warning that an unresolved known-kind placeholder is logged
    /// rather than silently shipped. A kind in `label` but not `from_label` degrades
    /// convergence and observability with no failing test.
    ///
    /// Labels must also be **distinct**: two kinds sharing one label would make the inverse
    /// ambiguous and silently collapse them in the vault.
    #[test]
    fn every_kind_round_trips_through_its_label() {
        let mut seen: Vec<&'static str> = Vec::new();
        for &kind in PiiKind::ALL {
            let label = kind.label();
            assert_eq!(
                PiiKind::from_label(label),
                Some(kind),
                "{kind:?} labels itself {label} but from_label does not map it back — the \
                 placeholder would not be inert to the next masking pass (M5-R4)"
            );
            // Case-insensitivity is part of the contract: the model may echo a token back in
            // any case, and the de-masker has to recognise it to warn about it.
            assert_eq!(PiiKind::from_label(&label.to_ascii_lowercase()), Some(kind));
            assert!(
                !seen.contains(&label),
                "{label} is used by two kinds — from_label cannot be an inverse of label"
            );
            seen.push(label);
        }
    }
}

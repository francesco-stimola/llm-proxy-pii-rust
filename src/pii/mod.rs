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

/// A category of personally identifiable information.
///
/// `Serialize`/`Deserialize` use the variant names verbatim (e.g. `"Email"`,
/// `"CreditCard"`), which is the encoding the JSON test corpus in
/// `tests/corpus/pii_cases.json` relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PiiKind {
    // Structured (deterministic recognizers — M1)
    Email,
    Phone,
    Ssn,
    /// A non-US national identifier (M4) — e.g. Italian Codice Fiscale, UK NINO.
    /// US SSN keeps its own [`Ssn`](Self::Ssn) variant for continuity.
    NationalId,
    CreditCard,
    Iban,
    /// API keys / tokens (e.g. `sk-…`, `sk-ant-…`, `AKIA…`). Deterministic —
    /// the old ML model missed these, so they are treated as structured PII.
    Secret,
    // Unstructured (ONNX NER — M2)
    Person,
    Organization,
    Location,
}

impl PiiKind {
    /// The uppercase label used inside placeholders, e.g. `Email` → `"EMAIL"`
    /// yields the `[EMAIL_1]` token. ASCII and tokenizer-friendly.
    pub fn label(self) -> &'static str {
        match self {
            PiiKind::Email => "EMAIL",
            PiiKind::Phone => "PHONE",
            PiiKind::Ssn => "SSN",
            PiiKind::NationalId => "NATID",
            PiiKind::CreditCard => "CARD",
            PiiKind::Iban => "IBAN",
            PiiKind::Secret => "SECRET",
            PiiKind::Person => "PERSON",
            PiiKind::Organization => "ORG",
            PiiKind::Location => "LOCATION",
        }
    }

    /// Overlap-resolution priority (see [`overlap::resolve_overlaps`]). Order:
    /// Secret > Iban > CreditCard > Ssn ≈ NationalId > Phone > Email > NER.
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
    pub fn priority(self) -> u8 {
        match self {
            PiiKind::Secret => 6,
            PiiKind::Iban => 5,
            PiiKind::CreditCard => 4,
            // National identifiers (US SSN + other locales) share a tier; ties fall
            // through to span length, then to the incumbent — always deterministic.
            PiiKind::Ssn | PiiKind::NationalId => 3,
            PiiKind::Phone => 2,
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

    /// Inverse of [`label`](Self::label): recover the kind from a placeholder
    /// label (case-insensitive). Lets the de-masker recognise a token that the
    /// model echoed back so it can warn if that token isn't in the vault.
    pub fn from_label(label: &str) -> Option<Self> {
        let kind = match label.to_ascii_uppercase().as_str() {
            "EMAIL" => PiiKind::Email,
            "PHONE" => PiiKind::Phone,
            "SSN" => PiiKind::Ssn,
            "NATID" => PiiKind::NationalId,
            "CARD" => PiiKind::CreditCard,
            "IBAN" => PiiKind::Iban,
            "SECRET" => PiiKind::Secret,
            "PERSON" => PiiKind::Person,
            "ORG" => PiiKind::Organization,
            "LOCATION" => PiiKind::Location,
            _ => return None,
        };
        Some(kind)
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
    pub budget_exhausted: bool,
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
}

impl Budget {
    /// A budget of `calls` cache-missing validator invocations.
    pub fn new(calls: usize) -> Self {
        Self {
            left: std::cell::Cell::new(calls),
            initial: calls,
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
        }
    }

    /// Charge one expensive validator call. Saturates at zero rather than wrapping — an exhausted
    /// budget stays exhausted, which is what [`is_exhausted`](Self::is_exhausted) rests on.
    pub fn spend(&self) {
        self.left.set(self.left.get().saturating_sub(1));
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
/// forwards *the same one*. There is no method left that a default can route to which would mint
/// another: the only way to create an allowance is `Budget::new`, which is one grep and is not
/// something a forgotten override can do by accident.
///
/// *An obligation a trait default can satisfy is not carried by the type system — and "every test
/// passes" is the signature of that, not evidence against it.*
pub trait PiiDetector: Send + Sync {
    /// Return all PII entities found in `input`. Infallible view — a detector
    /// that can fail returns whatever it could detect (typically empty on error).
    ///
    /// **Not on the request path**, and deliberately: a detector that can exhaust an allowance mints
    /// its own here, because there is no request to charge. `PrivacyStage` uses the fallible pair.
    fn detect(&self, input: &str) -> Vec<PiiEntity>;

    /// Fallible detection, charged against the **caller's** [`Budget`] — one per request.
    ///
    /// The default is infallible ([`detect`](Self::detect)) and ignores the budget, which is right
    /// for a detector whose cost is not what is being bounded (the NER pays per token, and
    /// `MAX_BODY_BYTES` already bounds tokens). A detector that can genuinely fail (ML inference, bad
    /// config) overrides this so a *required* detector can fail the request **closed**; a **wrapper**
    /// overrides it to forward `budget` to what it wraps.
    fn try_detect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        let _ = budget;
        Ok(self.detect(input))
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

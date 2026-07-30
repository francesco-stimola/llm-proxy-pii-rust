//! Combine several detectors behind one [`PiiDetector`].
//!
//! The hybrid design (see `docs/ARCHITECTURE.md`) runs the deterministic
//! structured recognizers **and** the ML NER (M2) over the same text and merges
//! their spans. Keeping the combination behind the trait means the pipeline
//! never knows how many engines are underneath — it just gets one detector.

use super::overlap::resolve_overlaps;
use super::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST;
use super::{Budget, DetectError, PiiDetector, PiiEntity};

/// Runs every wrapped detector over the input and reconciles their spans with
/// the shared [`resolve_overlaps`] — so a deterministic match (email, IBAN) wins
/// over an ML `Person`/`Org`/`Location` guess on the same characters.
///
/// [`try_detect`](PiiDetector::try_detect) **propagates** a sub-detector error,
/// so a required detector (one not wrapped in [`FailOpen`]) fails the request
/// closed. Wrap a non-critical detector in [`FailOpen`] to opt it out.
pub struct CompositeDetector {
    detectors: Vec<Box<dyn PiiDetector>>,
}

impl CompositeDetector {
    /// Build a composite from an ordered list of detectors. A single-detector
    /// composite is fine (and behaves exactly like that detector).
    pub fn new(detectors: Vec<Box<dyn PiiDetector>>) -> Self {
        Self { detectors }
    }
}

impl PiiDetector for CompositeDetector {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        // Infallible view: never propagate — used where an error shouldn't block. Not the request
        // path, so it mints an allowance of its own.
        self.try_detect(input, &Budget::new(MAX_PHONE_VALIDATIONS_PER_REQUEST))
            .unwrap_or_default()
    }

    /// Forwards the caller's [`Budget`] to every sub-detector, so **one** allowance covers the whole
    /// composite rather than one per engine (M10-R28).
    fn try_detect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        let mut all = Vec::new();
        for detector in &self.detectors {
            all.extend(detector.try_detect(input, budget)?);
        }
        Ok(resolve_overlaps(input, all))
    }

    /// Re-detect on a later fixpoint pass by asking each sub-detector for **its** `redetect` — so
    /// the NER (idempotent after pass 0) contributes nothing while the structured recognizers
    /// rescan for PII the last mask may have exposed. Overlaps are resolved exactly as in
    /// [`try_detect`](Self::try_detect); the two differ only in which detectors speak.
    ///
    /// Overriding this is a **dispatch** decision — which children speak — not a budget one. Since
    /// M10-R35 the budget is a parameter of both methods and there is no budget-less sibling for a
    /// forgotten override to fall through to.
    fn redetect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        let mut all = Vec::new();
        for detector in &self.detectors {
            all.extend(detector.redetect(input, budget)?);
        }
        Ok(resolve_overlaps(input, all))
    }
}

/// Wraps a detector so its errors are **swallowed** (logged, treated as "no
/// entities") instead of failing the request. This is how a non-critical
/// detector — e.g. the NER when `NER_REQUIRED` is off — opts out of fail-closed.
pub struct FailOpen(pub Box<dyn PiiDetector>);

impl PiiDetector for FailOpen {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        self.0.detect(input)
    }

    /// Forwards the caller's [`Budget`] — and **stops being fail-open when the budget is what
    /// failed** (M10-R28, made structural by M10-R41).
    ///
    /// The two are different kinds of failure and conflating them is a leak. *"This detector is
    /// unavailable"* is a property of the **detector**, and continuing without a non-critical engine
    /// is exactly what this wrapper is for. *"The allowance for scanning this request ran out"* is a
    /// property of the **request**: the text was not fully examined, so its PII status is unknown,
    /// and answering `Ok(vec![])` there means forwarding a partially scanned body with a clean bill
    /// of health. Round 4 was launched to ask whether an exhausted budget could ever fail *open*; the
    /// answer was no only because nothing wraps the structured recognizers in `FailOpen` today. That
    /// is a property of the wiring, not of the code, and wiring changes.
    ///
    /// **It asks the error, not the budget (M10-R41).** The first version matched on
    /// `budget.is_exhausted()` — a property of the *request*, asked about an error that may belong to
    /// the *detector*. It gave the right answer only through an unstated invariant (no detector
    /// returns `Ok` with an exhausted budget, and `build_detector` orders the structured recognizers
    /// first), and it turned a genuine GPU or tokenizer failure that happened to arrive at a spent
    /// budget into a `400` on a proxy configured to degrade to structured-only. `DetectError` now
    /// carries the distinction, so this reads what it means.
    fn try_detect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        Self::fail_open(self.0.try_detect(input, budget))
    }

    /// Same fail-open contract as [`try_detect`](Self::try_detect), delegating to the inner
    /// detector's [`redetect`](PiiDetector::redetect) — so a fail-open-wrapped NER stays
    /// idempotent after pass 0 (it returns nothing) rather than re-running via the default.
    fn redetect(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        Self::fail_open(self.0.redetect(input, budget))
    }
}

impl FailOpen {
    /// Swallow a **detector** failure, propagate an exhausted **request** allowance.
    ///
    /// One place for the decision, so the two entry points cannot drift apart on it — which is the
    /// shape of M10-R35 one level down.
    fn fail_open(
        outcome: Result<Vec<PiiEntity>, DetectError>,
    ) -> Result<Vec<PiiEntity>, DetectError> {
        match outcome {
            Ok(found) => Ok(found),
            Err(err) if err.is_budget_exhausted() => Err(err),
            Err(err) => {
                // Log the detector label only — never the input.
                tracing::warn!(
                    detector = err.detector,
                    "detector failed; continuing without it"
                );
                Ok(Vec::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::recognizers::StructuredRecognizers;
    use crate::pii::{Confidence, PiiEntity, PiiKind};

    /// A stand-in NER that tags a fixed substring — used to prove the merge
    /// without needing a real ONNX model.
    struct FakeNer {
        needle: &'static str,
        kind: PiiKind,
    }

    impl PiiDetector for FakeNer {
        // A stand-in NER cannot fail and does not charge the budget — but since M10-R44 it says so
        // here, in the required method, rather than inheriting a default that would decide for it.
        fn try_detect(&self, input: &str, _budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
            Ok(input
                .match_indices(self.needle)
                .map(|(start, m)| PiiEntity {
                    kind: self.kind,
                    span: start..start + m.len(),
                    text: m.to_string(),
                    confidence: Confidence::Structural,
                })
                .collect())
        }
    }

    #[test]
    fn merges_structured_and_ner_entities() {
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(FakeNer {
                needle: "Mario Rossi",
                kind: PiiKind::Person,
            }),
        ]);
        let got: Vec<_> = composite
            .detect("Mario Rossi, mail mario@rossi.it")
            .into_iter()
            .map(|e| (e.kind, e.text))
            .collect();
        assert_eq!(
            got,
            vec![
                (PiiKind::Person, "Mario Rossi".to_string()),
                (PiiKind::Email, "mario@rossi.it".to_string()),
            ]
        );
    }

    #[test]
    fn structured_wins_when_ner_overlaps_it() {
        // NER greedily tags the whole email as an Organization; the deterministic
        // Email must win the overlapping span.
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(FakeNer {
                needle: "bob@acme.com",
                kind: PiiKind::Organization,
            }),
        ]);
        let got = composite.detect("write bob@acme.com");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Email);
    }

    /// A detector that always fails, to exercise the error channel.
    struct FailingDetector;
    impl PiiDetector for FailingDetector {
        fn detect(&self, _input: &str) -> Vec<PiiEntity> {
            Vec::new()
        }
        fn try_detect(
            &self,
            _input: &str,
            _budget: &Budget,
        ) -> Result<Vec<PiiEntity>, super::DetectError> {
            Err(super::DetectError::unavailable("failing", "boom"))
        }
    }

    #[test]
    fn required_detector_error_propagates() {
        // M2-R2: a required (unwrapped) failing detector fails the composite closed.
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(FailingDetector),
        ]);
        assert!(composite
            .try_detect("mail bob@test.com", &Budget::per_call())
            .is_err());
    }

    #[test]
    fn fail_open_wrapped_detector_is_swallowed() {
        // M2-R2: FailOpen turns a detector error into "no entities", so the
        // composite still succeeds (structured PII survives).
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(super::FailOpen(Box::new(FailingDetector))),
        ]);
        let got = composite
            .try_detect("mail bob@test.com", &Budget::per_call())
            .expect("must not error");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Email);
    }
}

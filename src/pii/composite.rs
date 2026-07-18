//! Combine several detectors behind one [`PiiDetector`].
//!
//! The hybrid design (see `docs/ARCHITECTURE.md`) runs the deterministic
//! structured recognizers **and** the ML NER (M2) over the same text and merges
//! their spans. Keeping the combination behind the trait means the pipeline
//! never knows how many engines are underneath — it just gets one detector.

use super::overlap::resolve_overlaps;
use super::{DetectError, PiiDetector, PiiEntity};

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
        // Infallible view: never propagate — used where an error shouldn't block.
        self.try_detect(input).unwrap_or_default()
    }

    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        let mut all = Vec::new();
        for detector in &self.detectors {
            all.extend(detector.try_detect(input)?);
        }
        Ok(resolve_overlaps(input, all))
    }

    /// Re-detect on a later fixpoint pass by asking each sub-detector for **its** `redetect` — so
    /// the NER (idempotent after pass 0) contributes nothing while the structured recognizers
    /// rescan for PII the last mask may have exposed. Overlaps are resolved exactly as in
    /// [`try_detect`](Self::try_detect); the two differ only in which detectors speak.
    fn redetect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        let mut all = Vec::new();
        for detector in &self.detectors {
            all.extend(detector.redetect(input)?);
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

    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        Ok(self.0.try_detect(input).unwrap_or_else(|err| {
            // Log the detector label only — never the input.
            tracing::warn!(
                detector = err.detector,
                "detector failed; continuing without it"
            );
            Vec::new()
        }))
    }

    /// Same fail-open contract as [`try_detect`](Self::try_detect), delegating to the inner
    /// detector's [`redetect`](PiiDetector::redetect) — so a fail-open-wrapped NER stays
    /// idempotent after pass 0 (it returns nothing) rather than re-running via the default.
    fn redetect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        Ok(self.0.redetect(input).unwrap_or_else(|err| {
            tracing::warn!(
                detector = err.detector,
                "detector failed; continuing without it"
            );
            Vec::new()
        }))
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
        fn detect(&self, input: &str) -> Vec<PiiEntity> {
            input
                .match_indices(self.needle)
                .map(|(start, m)| PiiEntity {
                    kind: self.kind,
                    span: start..start + m.len(),
                    text: m.to_string(),
                    confidence: Confidence::Structural,
                })
                .collect()
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
        fn try_detect(&self, _input: &str) -> Result<Vec<PiiEntity>, super::DetectError> {
            Err(super::DetectError {
                detector: "failing",
                message: "boom".to_string(),
            })
        }
    }

    #[test]
    fn required_detector_error_propagates() {
        // M2-R2: a required (unwrapped) failing detector fails the composite closed.
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(FailingDetector),
        ]);
        assert!(composite.try_detect("mail bob@test.com").is_err());
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
            .try_detect("mail bob@test.com")
            .expect("must not error");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Email);
    }
}

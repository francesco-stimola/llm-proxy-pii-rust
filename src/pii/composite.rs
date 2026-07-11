//! Combine several detectors behind one [`PiiDetector`].
//!
//! The hybrid design (see `docs/ARCHITECTURE.md`) runs the deterministic
//! structured recognizers **and** the ML NER (M2) over the same text and merges
//! their spans. Keeping the combination behind the trait means the pipeline
//! never knows how many engines are underneath — it just gets one detector.

use super::overlap::resolve_overlaps;
use super::{PiiDetector, PiiEntity};

/// Runs every wrapped detector over the input and reconciles their spans with
/// the shared [`resolve_overlaps`] — so a deterministic match (email, IBAN) wins
/// over an ML `Person`/`Org`/`Location` guess on the same characters.
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
        let mut all = Vec::new();
        for detector in &self.detectors {
            all.extend(detector.detect(input));
        }
        resolve_overlaps(all)
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
}

//! The privacy stage: detect PII in the outgoing request, mask it, and restore
//! it in the incoming response. The only stage wired in the current milestone.

use crate::pii::PiiDetector;
use crate::pipeline::Stage;
use crate::proxy::{ProxyRequest, ProxyResponse};

/// Masks PII on the way out and restores it on the way back, using an
/// engine-agnostic [`PiiDetector`].
pub struct PrivacyStage {
    #[allow(dead_code)] // used from M1 onward
    detector: Box<dyn PiiDetector>,
    // TODO(M1): per-request vaults (placeholder → original), keyed by request id.
}

impl PrivacyStage {
    /// Build the stage around a concrete detector (deterministic, ML, or both).
    pub fn new(detector: Box<dyn PiiDetector>) -> Self {
        Self { detector }
    }
}

impl Stage for PrivacyStage {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn on_request(&self, _req: &mut ProxyRequest) {
        // TODO(M1): extract text from the messages, detect PII with `detector`,
        // and mask each span via the request's Vault.
        todo!()
    }

    fn on_response(&self, _resp: &mut ProxyResponse) {
        // TODO(M1): restore placeholders using the request's Vault.
        todo!()
    }
}

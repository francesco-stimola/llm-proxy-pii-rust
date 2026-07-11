//! Modular request/response pipeline.
//!
//! Each [`Stage`] can transform a request before it goes upstream and the
//! response before it returns to the client. Only [`privacy::PrivacyStage`] is
//! wired for now, but the trait keeps the proxy open to more stages (auth,
//! rate-limiting, logging, …) later without touching the core.

pub mod privacy;

use crate::pii::anonymizer::Vault;
use crate::proxy::{ProxyRequest, ProxyResponse};

/// Per-request state carried from the request hooks to the response hooks.
///
/// The proxy creates one of these per incoming request, threads it through every
/// stage's [`Stage::on_request`], and then through every stage's
/// [`Stage::on_response`]. Today it only holds the privacy [`Vault`] (built while
/// masking, consumed while restoring); other per-request state (auth claims,
/// timing, …) can hang off here as stages are added.
#[derive(Debug, Default)]
pub struct RequestContext {
    /// Placeholder ↔ original mapping for the privacy stage.
    pub vault: Vault,
}

impl RequestContext {
    /// Create an empty per-request context.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A single transformation stage in the proxy pipeline.
///
/// Stages run in order on the way out ([`on_request`](Stage::on_request)) and in
/// reverse order on the way back ([`on_response`](Stage::on_response)), sharing a
/// per-request [`RequestContext`] so a stage can carry state from request to
/// response.
pub trait Stage: Send + Sync {
    /// Stable, human-readable stage name (for logging/telemetry).
    fn name(&self) -> &'static str;

    /// Transform the outgoing request (client → provider).
    fn on_request(&self, req: &mut ProxyRequest, ctx: &mut RequestContext);

    /// Transform the incoming response (provider → client).
    fn on_response(&self, resp: &mut ProxyResponse, ctx: &mut RequestContext);
}

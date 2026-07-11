//! Modular request/response pipeline.
//!
//! Each [`Stage`] can transform a request before it goes upstream and the
//! response before it returns to the client. Only [`privacy::PrivacyStage`] is
//! wired for now, but the trait keeps the proxy open to more stages (auth,
//! rate-limiting, logging, …) later without touching the core.

pub mod privacy;

use crate::proxy::{ProxyRequest, ProxyResponse};

/// A single transformation stage in the proxy pipeline.
///
/// Design note (M1): stages that need per-request state carried from request to
/// response — e.g. the privacy [`Vault`](crate::pii::anonymizer::Vault) — will
/// receive a per-request context. The exact signature is finalized in M1.
pub trait Stage: Send + Sync {
    /// Stable, human-readable stage name (for logging/telemetry).
    fn name(&self) -> &'static str;

    /// Transform the outgoing request (client → provider).
    fn on_request(&self, req: &mut ProxyRequest);

    /// Transform the incoming response (provider → client).
    fn on_response(&self, resp: &mut ProxyResponse);
}

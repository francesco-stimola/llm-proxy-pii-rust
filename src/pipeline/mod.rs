//! Modular request/response pipeline.
//!
//! Each [`Stage`] can transform a request before it goes upstream and the
//! response before it returns to the client. Only [`privacy::PrivacyStage`] is
//! wired for now, but the trait keeps the proxy open to more stages (auth,
//! rate-limiting, logging, …) later without touching the core.

pub mod privacy;

use crate::pii::anonymizer::Vault;
use crate::proxy::{ProxyRequest, ProxyResponse};

/// Which wire protocol an inbound request speaks. It selects the schema-specific
/// masking / demasking walk in [`privacy::PrivacyStage`] (M6) — the request body
/// is a different shape per schema, so a single walk cannot cover both without a
/// missed field, and a missed field is a leak.
///
/// `OpenAi` is the Chat Completions schema (`/v1/chat/completions`), the default
/// and the only inbound schema before M6. `Anthropic` is the native Messages
/// schema (`/v1/messages`) — the Claude Code passthrough. The handler sets the
/// tag on the [`RequestContext`] before running the stages; the OpenAI path is
/// therefore entirely undisturbed (it gets the `Default`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WireSchema {
    /// OpenAI Chat Completions (`/v1/chat/completions`).
    #[default]
    OpenAi,
    /// Anthropic native Messages (`/v1/messages`).
    Anthropic,
}

/// Per-request state carried from the request hooks to the response hooks.
///
/// The proxy creates one of these per incoming request, threads it through every
/// stage's [`Stage::on_request`], and then through every stage's
/// [`Stage::on_response`]. Today it holds the privacy [`Vault`] (built while
/// masking, consumed while restoring) and the inbound [`WireSchema`]; other
/// per-request state (auth claims, timing, …) can hang off here as stages are added.
#[derive(Debug, Default)]
pub struct RequestContext {
    /// Placeholder ↔ original mapping for the privacy stage.
    pub vault: Vault,
    /// Which wire schema the request body speaks — selects the privacy stage's
    /// mask/demask walk (M6). Set by the handler before the stages run.
    pub schema: WireSchema,
    /// Set by a stage to **fail closed**: the request must be rejected, not
    /// forwarded, because it could otherwise leak PII (e.g. an unrecognized
    /// payload shape a masker can't safely cover). The `String` is a
    /// client-facing reason. The proxy stops the pipeline and returns 400.
    pub block: Option<String>,
}

impl RequestContext {
    /// Create an empty per-request context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail closed: record why the request must be blocked (first reason wins).
    pub fn block(&mut self, reason: impl Into<String>) {
        if self.block.is_none() {
            self.block = Some(reason.into());
        }
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

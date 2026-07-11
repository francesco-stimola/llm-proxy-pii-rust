//! Proxy core: applies the pipeline and forwards to the upstream provider.
//!
//! Streaming (SSE) is part of the design from day one; incremental
//! de-anonymization over the token stream is milestone M3.

/// Minimal view of an outgoing request we need to inspect and rewrite before
/// forwarding it upstream.
pub struct ProxyRequest {
    /// The parsed JSON body (OpenAI-shaped).
    pub body: serde_json::Value,
}

/// Minimal view of an incoming (non-streaming) response to restore before
/// returning it to the client.
pub struct ProxyResponse {
    /// The parsed JSON body (OpenAI-shaped).
    pub body: serde_json::Value,
}

// TODO(M1): upstream client (reqwest), request forwarding, and error mapping.
// TODO(M3): streaming passthrough with incremental de-anonymization.

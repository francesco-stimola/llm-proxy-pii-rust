//! Proxy core: the request/response value objects the pipeline rewrites, plus
//! the upstream HTTP client that forwards to the provider.
//!
//! Streaming (SSE) is part of the design from day one; incremental
//! de-anonymization over the token stream is milestone M3, so today only
//! non-streaming JSON responses are forwarded.

use anyhow::Context;
use serde_json::Value;

/// Minimal view of an outgoing request we need to inspect and rewrite before
/// forwarding it upstream.
pub struct ProxyRequest {
    /// The parsed JSON body (OpenAI-shaped).
    pub body: Value,
}

/// Minimal view of an incoming (non-streaming) response to restore before
/// returning it to the client.
pub struct ProxyResponse {
    /// The parsed JSON body (OpenAI-shaped).
    pub body: Value,
}

/// HTTP client to the upstream OpenAI-compatible provider.
pub struct Upstream {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl Upstream {
    /// Build an upstream client. `api_key`, if set, is used only when the client
    /// request carries no `Authorization` header of its own.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key,
        }
    }

    /// Forward an (already anonymized) chat-completions body upstream and return
    /// the `(status, headers, json)` response. The caller restores the JSON via
    /// the pipeline and forwards a safe subset of the headers to the client.
    pub async fn forward_chat_completions(
        &self,
        body: &Value,
        client_auth: Option<&str>,
    ) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Value)> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let mut request = self.client.post(&url).json(body);
        if let Some(auth) = client_auth {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        } else if let Some(key) = &self.api_key {
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"));
        }

        let response = request
            .send()
            .await
            .context("forwarding request to upstream failed")?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let json = response
            .json::<Value>()
            .await
            .context("upstream response was not valid JSON")?;
        Ok((status, headers, json))
    }
}

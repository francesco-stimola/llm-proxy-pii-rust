//! Proxy core: the request/response value objects the pipeline rewrites, plus
//! the upstream HTTP client that forwards to the provider.
//!
//! Streaming (SSE) round-trips are handled in [`crate::server`] via
//! [`Upstream::send`] (which exposes the raw response for `bytes_stream`);
//! non-streaming JSON goes through [`Upstream::forward_chat_completions`].

use anyhow::Context;
use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
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
///
/// The path and any provider-required static headers are configurable (M3,
/// Option A), so one binary can front OpenAI, Copilot, or Anthropic's
/// OpenAI-compat endpoint by config alone.
pub struct Upstream {
    client: reqwest::Client,
    base_url: String,
    chat_path: String,
    api_key: Option<String>,
    /// Pre-parsed static headers added to every upstream request.
    extra_headers: Vec<(HeaderName, HeaderValue)>,
}

impl Upstream {
    /// Build an upstream client. `api_key`, if set, is used only when the client
    /// request carries no `Authorization` header of its own. `chat_path` is the
    /// provider's chat-completions path; `extra_headers` are provider-required
    /// static headers (invalid ones are skipped with a warning).
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        chat_path: impl Into<String>,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        let extra_headers = extra_headers
            .into_iter()
            .filter_map(|(name, value)| {
                match (
                    HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(&value),
                ) {
                    (Ok(n), Ok(v)) => Some((n, v)),
                    _ => {
                        tracing::warn!(header = %name, "ignoring invalid upstream extra header");
                        None
                    }
                }
            })
            .collect();
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            chat_path: chat_path.into(),
            api_key,
            extra_headers,
        }
    }

    /// Build the upstream request: URL + body + auth + static + passthrough headers.
    fn build(
        &self,
        body: &Value,
        client_auth: Option<&str>,
        passthrough: &[(HeaderName, HeaderValue)],
    ) -> reqwest::RequestBuilder {
        let url = format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            self.chat_path
        );
        let mut request = self.client.post(&url).json(body);
        // Prefer the client's own credential; else inject the configured key.
        if let Some(auth) = client_auth {
            request = request.header(AUTHORIZATION, auth);
        } else if let Some(key) = &self.api_key {
            request = request.header(AUTHORIZATION, format!("Bearer {key}"));
        }
        for (name, value) in &self.extra_headers {
            request = request.header(name, value);
        }
        for (name, value) in passthrough {
            request = request.header(name, value);
        }
        request
    }

    /// Send an (already anonymized) body upstream and return the **raw** response,
    /// so the caller can either parse JSON or consume the SSE `bytes_stream`.
    pub async fn send(
        &self,
        body: &Value,
        client_auth: Option<&str>,
        passthrough: &[(HeaderName, HeaderValue)],
    ) -> anyhow::Result<reqwest::Response> {
        self.build(body, client_auth, passthrough)
            .send()
            .await
            .context("forwarding request to upstream failed")
    }

    /// Forward a non-streaming chat-completions body upstream and return the
    /// `(status, headers, json)` response. The caller restores the JSON via the
    /// pipeline and forwards a safe subset of the headers to the client.
    pub async fn forward_chat_completions(
        &self,
        body: &Value,
        client_auth: Option<&str>,
        passthrough: &[(HeaderName, HeaderValue)],
    ) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Value)> {
        let response = self.send(body, client_auth, passthrough).await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let json = response
            .json::<Value>()
            .await
            .context("upstream response was not valid JSON")?;
        Ok((status, headers, json))
    }
}

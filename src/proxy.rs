//! Proxy core: the request/response value objects the pipeline rewrites, plus
//! the upstream HTTP client that forwards to the provider.
//!
//! Streaming (SSE) round-trips are handled in [`crate::server`] via
//! [`Upstream::send`] (which exposes the raw response for `bytes_stream`);
//! non-streaming JSON goes through [`Upstream::forward_chat_completions`].

use anyhow::Context;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};
use serde_json::Value;

/// Default `anthropic-version` injected on the native Messages route (M6) when the
/// client didn't send one. Claude Code always sends it; this is the robustness
/// fallback so a bare Anthropic SDK request still reaches a versioned endpoint.
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

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
    /// Native Anthropic Messages path (M6) — used only by the `/v1/messages` route.
    messages_path: String,
    api_key: Option<String>,
    /// Pre-parsed static headers added to every upstream request.
    extra_headers: Vec<(HeaderName, HeaderValue)>,
}

impl Upstream {
    /// Build an upstream client. `api_key`, if set, is used only when the client
    /// request carries no credential of its own. `chat_path` is the provider's
    /// chat-completions path; `messages_path` the native Anthropic Messages path
    /// (M6); `extra_headers` are provider-required static headers (invalid ones
    /// are skipped with a warning).
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        chat_path: impl Into<String>,
        messages_path: impl Into<String>,
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
            messages_path: messages_path.into(),
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
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), self.chat_path);
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

    // ── Native Anthropic Messages (M6) ──────────────────────────────────────

    /// Resolve the single credential header for a native `/v1/messages` request.
    ///
    /// Order (M6): the client's own `Authorization` (Claude Code's OAuth
    /// `Bearer sk-ant-oat01-…`) wins, then a client `x-api-key`, then the proxy's
    /// configured key as `x-api-key`. **An OAuth token is never placed in
    /// `x-api-key`** — Anthropic rejects that with 401; it only ever rides in
    /// `Authorization`, forwarded verbatim. `None` means no usable credential at
    /// all (the handler answers 401 without forwarding). This mirrors the chat
    /// path's "client credential wins, configured key is the fallback" posture,
    /// so the proxy can front Claude Code without ever holding a key.
    pub fn messages_auth(
        &self,
        client_auth: Option<&str>,
        client_api_key: Option<&str>,
    ) -> Option<(HeaderName, HeaderValue)> {
        if let Some(auth) = client_auth {
            if let Ok(value) = HeaderValue::from_str(auth) {
                return Some((AUTHORIZATION, value));
            }
        }
        let x_api_key = HeaderName::from_static("x-api-key");
        if let Some(key) = client_api_key {
            if let Ok(value) = HeaderValue::from_str(key) {
                return Some((x_api_key, value));
            }
        }
        if let Some(key) = &self.api_key {
            if let Ok(value) = HeaderValue::from_str(key) {
                return Some((x_api_key, value));
            }
        }
        None
    }

    /// Build a native Messages request: URL + body + the resolved credential +
    /// static + passthrough headers, with a default `anthropic-version` when the
    /// client sent none (Claude Code always sends it; a bare SDK request may not).
    fn build_messages(
        &self,
        body: &Value,
        auth: &(HeaderName, HeaderValue),
        passthrough: &[(HeaderName, HeaderValue)],
    ) -> reqwest::RequestBuilder {
        let url = format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            self.messages_path
        );
        let mut request = self.client.post(&url).json(body);
        request = request.header(auth.0.clone(), auth.1.clone());
        for (name, value) in &self.extra_headers {
            request = request.header(name, value);
        }
        // `anthropic-version` / `anthropic-beta` reach here via the allowlist
        // passthrough; inject the default version only if the client omitted it.
        let mut has_version = false;
        for (name, value) in passthrough {
            if name.as_str().eq_ignore_ascii_case("anthropic-version") {
                has_version = true;
            }
            request = request.header(name, value);
        }
        if !has_version {
            request = request.header(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION),
            );
        }
        request
    }

    /// Send an (already anonymized) native Messages body upstream and return the
    /// **raw** response — for either JSON parsing or SSE `bytes_stream`.
    pub async fn send_messages(
        &self,
        body: &Value,
        auth: &(HeaderName, HeaderValue),
        passthrough: &[(HeaderName, HeaderValue)],
    ) -> anyhow::Result<reqwest::Response> {
        self.build_messages(body, auth, passthrough)
            .send()
            .await
            .context("forwarding request to upstream (messages) failed")
    }

    /// Forward a non-streaming native Messages body upstream and return the
    /// `(status, headers, json)` response, mirroring
    /// [`forward_chat_completions`](Self::forward_chat_completions).
    pub async fn forward_messages(
        &self,
        body: &Value,
        auth: &(HeaderName, HeaderValue),
        passthrough: &[(HeaderName, HeaderValue)],
    ) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Value)> {
        let response = self.send_messages(body, auth, passthrough).await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let json = response
            .json::<Value>()
            .await
            .context("upstream response was not valid JSON")?;
        Ok((status, headers, json))
    }
}

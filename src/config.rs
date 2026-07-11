//! Runtime configuration.

use std::fmt;
use std::net::SocketAddr;

use anyhow::Context;

/// Where the proxy listens and which upstream it forwards to.
#[derive(Clone)]
pub struct Config {
    /// Address the proxy listens on.
    pub listen: SocketAddr,
    /// Base URL of the upstream OpenAI-compatible provider (no trailing
    /// `/v1/...` — the proxy appends the path).
    pub upstream_base_url: String,
    /// Optional API key injected as `Authorization: Bearer …` when the client
    /// did not send its own `Authorization` header.
    pub upstream_api_key: Option<String>,
}

impl Config {
    /// Load configuration from environment variables, with sensible defaults:
    /// `LISTEN_ADDR` (default `127.0.0.1:8080`), `UPSTREAM_BASE_URL`
    /// (default `https://api.openai.com`), and optional `UPSTREAM_API_KEY`.
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_raw =
            std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        let listen = listen_raw
            .parse()
            .with_context(|| format!("invalid LISTEN_ADDR: {listen_raw:?}"))?;

        let upstream_base_url = std::env::var("UPSTREAM_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com".to_string());

        // An empty value is treated as "unset" so an exported-but-blank var
        // doesn't send `Authorization: Bearer `.
        let upstream_api_key = std::env::var("UPSTREAM_API_KEY")
            .ok()
            .filter(|k| !k.is_empty());

        Ok(Self {
            listen,
            upstream_base_url,
            upstream_api_key,
        })
    }
}

/// Manual `Debug` so the API key is never written to logs — `main` logs the
/// whole config at startup, and this is a privacy tool.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("listen", &self.listen)
            .field("upstream_base_url", &self.upstream_base_url)
            .field(
                "upstream_api_key",
                &self.upstream_api_key.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

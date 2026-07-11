//! Runtime configuration.

use std::net::SocketAddr;

/// Proxy configuration, loaded at startup.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the proxy listens on.
    pub listen: SocketAddr,
    /// Base URL of the upstream OpenAI-compatible provider.
    pub upstream_base_url: String,
}

impl Config {
    /// Load configuration from environment variables, with sensible defaults.
    pub fn from_env() -> anyhow::Result<Self> {
        // TODO(M1): read LISTEN_ADDR and UPSTREAM_BASE_URL from the environment,
        // falling back to defaults (e.g. 127.0.0.1:8080 and the OpenAI API base).
        todo!()
    }
}

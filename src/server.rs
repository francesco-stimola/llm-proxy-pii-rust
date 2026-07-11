//! HTTP server: axum router and request handlers.

use crate::config::Config;

/// Build the router, bind the listener, and serve until shutdown.
///
/// The router forwards `/v1/*` requests through the pipeline and on to the
/// upstream provider (see [`crate::proxy`]).
pub async fn run(_config: Config) -> anyhow::Result<()> {
    // TODO(M1): build an axum Router, bind a TcpListener on `config.listen`,
    // and route `/v1/chat/completions` through the proxy handler.
    todo!()
}

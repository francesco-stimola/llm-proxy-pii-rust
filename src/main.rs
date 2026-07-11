//! Binary entry point: initialize tracing, load config, run the server.

use llm_proxy_pii_rust::{config::Config, server};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env()?;
    tracing::info!(?config, "starting llm-proxy-pii-rust");

    server::run(config).await
}

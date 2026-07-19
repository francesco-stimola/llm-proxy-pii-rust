//! Binary entry point: initialize tracing, load config, run the server.
//!
//! Configuration is entirely environment-driven (see `config::Config`), so there is no
//! argument parser here and no CLI dependency. The single flag below is a **diagnostic
//! mode**, not configuration: it measures and exits instead of serving.

use llm_proxy_pii_rust::{config::Config, server};

/// Measure the execution providers this binary can use, on this machine, and exit (M9).
const BENCH_PROVIDERS_FLAG: &str = "--bench-providers";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Diagnostic mode: "which backend is fastest *here*?" is a hardware question the project
    // cannot answer for the operator, so ship the measurement rather than a number. Handled
    // before `Config::from_env` because the benchmark needs only the NER model config, not a
    // valid upstream/server config — you should be able to benchmark without a provider key.
    if std::env::args().skip(1).any(|a| a == BENCH_PROVIDERS_FLAG) {
        return server::run_provider_benchmark().await;
    }

    let config = Config::from_env()?;
    tracing::info!(?config, "starting llm-proxy-pii-rust");

    server::run(config).await
}

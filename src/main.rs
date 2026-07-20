//! Binary entry point: initialize tracing, load config, run the server.
//!
//! Configuration is entirely environment-driven (see `config::Config`), so there is no
//! argument parser here and no CLI dependency. The single flag below is a **diagnostic
//! mode**, not configuration: it measures and exits instead of serving.

use llm_proxy_pii_rust::{config::Config, server};

/// Measure the execution providers this binary can use, on this machine, and exit (M9).
const BENCH_PROVIDERS_FLAG: &str = "--bench-providers";

const USAGE: &str = "\
llm-proxy-pii-rust — a local PII-masking proxy in front of an OpenAI-compatible LLM.

USAGE:
    llm-proxy-pii-rust                    Run the proxy (configuration is via environment
                                          variables — see README.md / docs/SETUP.md).
    llm-proxy-pii-rust --bench-providers  Measure which ONNX execution provider is fastest on
                                          this machine, print a report, and exit.
    llm-proxy-pii-rust --help             Print this message.
";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // **Unknown arguments are refused, never ignored (M9-R4).** Before M9 the binary took no
    // arguments, so discarding them was harmless; now a near-miss (`--bench-provider`,
    // `--bench-providers=1`, `--help`) would fall through and start a LIVE PROXY pointed at the
    // configured upstream, while the operator believed they had run a diagnostic. Every other
    // unexpected-input path in this codebase blocks or refuses — including this milestone's own
    // `NER_EXECUTION_PROVIDER` typo check — so this one must not fail open.
    let mut bench_providers = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            BENCH_PROVIDERS_FLAG => bench_providers = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => {
                anyhow::bail!(
                    "unrecognized argument {other:?}\n\n{USAGE}\nRefusing to start: an \
                     unrecognized argument is a mistake, and starting a live proxy in response \
                     to one would look like a diagnostic that never ran."
                );
            }
        }
    }

    // Diagnostic mode: "which backend is fastest *here*?" is a hardware question the project
    // cannot answer for the operator, so ship the measurement rather than a number. Handled
    // before `Config::from_env` because the benchmark needs only the NER model config, not a
    // valid upstream/server config — you should be able to benchmark without a provider key.
    if bench_providers {
        return server::run_provider_benchmark().await;
    }

    let config = Config::from_env()?;
    tracing::info!(?config, "starting llm-proxy-pii-rust");

    server::run(config).await
}

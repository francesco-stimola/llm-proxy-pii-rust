//! Binary entry point: initialize tracing, load config, run the server.
//!
//! Configuration is entirely environment-driven (see `config::Config`), so there is no
//! argument parser here and no CLI dependency. The two flags below are **diagnostic
//! modes**, not configuration: each answers a question and exits instead of serving.
//!
//! ## Why this is `fn main`, not `#[tokio::main]` (M10)
//!
//! Log timestamps are local, and the local UTC offset can only be read while the process
//! is still **single-threaded** — `#[tokio::main]` builds the runtime, worker threads and
//! all, *before* the body of `main` runs. So the offset is captured first and the runtime
//! is built by hand. See [`llm_proxy_pii_rust::logging`] for the full reasoning.

use llm_proxy_pii_rust::{config::Config, logging, server};

/// Measure the execution providers this binary can use, on this machine, and exit (M9).
const BENCH_PROVIDERS_FLAG: &str = "--bench-providers";

/// The environment reference `--help` prints (M10).
///
/// **A downloaded executable must be able to say what it accepts.** Configuration here is
/// *entirely* environment variables with no config file, so a help text that lists two
/// flags and defers to `README.md` means you need the repo to run the program.
///
/// Kept honest by CLI-05 (`tests/binary_smoke.rs`), which extracts every env key read by
/// `config.rs` / `server.rs` and fails if one is missing here — so a variable cannot be
/// added without appearing in `--help`.
const ENV_REFERENCE: &str = "\
CONFIGURATION (environment variables — there is no config file):

  Server
    LISTEN_ADDR                 [127.0.0.1:8080] Address the proxy listens on.
    MAX_BODY_BYTES              [16777216]       Request body limit, in bytes.
    RUST_LOG                    [info]           Log filter. `info` shows startup and
                                                 per-request lines; `llm_proxy_pii_rust=trace`
                                                 also dumps the MASKED upstream body.

  Upstream
    UPSTREAM_BASE_URL           [https://api.openai.com] Provider base URL. Must be an
                                                 http/https URL with a host — the proxy
                                                 refuses to start otherwise.
    UPSTREAM_API_KEY            [unset]          Sent as `Authorization: Bearer …` only when
                                                 the client sends no credential of its own.
    UPSTREAM_PROVIDER           [openai]         openai | copilot | anthropic. Picks the chat
                                                 path and forwarded headers; `anthropic` also
                                                 enables the native /v1/messages route.
    UPSTREAM_CHAT_PATH          [per provider]   Override the chat-completions path.
    UPSTREAM_MESSAGES_PATH      [/v1/messages]   Override the native Anthropic Messages path.
    UPSTREAM_FORWARD_HEADERS    [per provider]   Comma-separated client headers to pass through.
    UPSTREAM_EXTRA_HEADERS      [none]           `Key=Value;Key2=Value2` added to every
                                                 upstream request.

  Detection
    PII_LOCALES  [de,es,fr,gb,it,lv,nl,pt,cn]    Regions for the domestic-phone recognizers
                                                 (numbers written with no +CC). All of them are
                                                 on by default; setting this REPLACES that set,
                                                 and a code outside the list contributes nothing.
                                                 Set it EMPTY (PII_LOCALES=) to turn the tier
                                                 off entirely. National IDs are always on
                                                 regardless.
    PII_CACHE_ENTRIES           [16]             Content-keyed detection cache size; 0 disables.

  NER (needs a build with `--features onnx`)
    NER_MODEL_REPO              [unset]          HuggingFace `owner/name` to auto-download.
    NER_MODEL_REVISION          [478a2a3]        Pinned revision for that download.
    NER_MODEL_FILE              [onnx/model_quantized.onnx] File within the repo.
    NER_TOKENIZER_FILE          [tokenizer.json] Tokenizer file within the repo.
    NER_CONFIG_FILE             [config.json]    Config file within the repo (for the labels).
    NER_MODEL_PATH              [unset]          Explicit local model file. Wins over the repo,
                                                 and makes zero outbound calls.
    NER_TOKENIZER_PATH          [unset]          Explicit local tokenizer, with NER_MODEL_PATH.
    NER_LABELS                  [from config]    Comma-separated BIO label list override.
    NER_REQUIRED                [off]            Fail CLOSED for names: at least one ML detector
                                                 must load, and a detection failure blocks the
                                                 request instead of degrading silently.
    NER_POOL_SIZE               [1]              Concurrent ONNX sessions.
    NER_INTRA_THREADS           [derived]        Threads per session (multiplies with the pool).
                                                 max(1, base / NER_POOL_SIZE), where the base is
                                                 min(physical cores, available parallelism) --
                                                 PHYSICAL cores, so on an SMT box this is half the
                                                 logical count. An explicit value wins. The startup
                                                 log prints the base and which count decided it.
    NER_TOKEN_TYPE_IDS          [off]            Feed `token_type_ids` to models that need them.
    NER_EXECUTION_PROVIDER      [auto]           Force one execution provider by name.
    NER_BENCH_MODELS            [unset]          Extra model files for --bench-providers.
    HF_HOME / HF_HUB_CACHE      [~/.cache/huggingface] HuggingFace's own cache location, honored
                                                 as-is when either is set; otherwise the standard
                                                 hub cache is pinned explicitly.

  GLiNER (needs a build with `--features onnx`)
    GLINER_MODEL_PATH           [unset]          Explicit local model file. Unset = GLiNER off.
    GLINER_TOKENIZER_PATH       [unset]          Explicit local tokenizer.
    GLINER_CONFIG_PATH          [unset]          The model's `gliner_config.json`.
    GLINER_LABELS               [person,organization,location,phone number,address]
    GLINER_THRESHOLD            [0.15]           Per-span probability threshold.
    GLINER_POOL_SIZE            [1]              Concurrent ONNX sessions.
    GLINER_INTRA_THREADS        [derived]        Threads per session. Same derivation, same base,
                                                 same function as the NER's knob.

  Debug (off by default; neither weakens fail-closed)
    PII_DEBUG_SKIP_DEMASK       [off]            Skip the response de-mask so the client sees the
                                                 placeholders the provider saw. Never in production.

Full descriptions, the detection matrix and the privacy model: README.md, docs/SETUP.md.
";

const USAGE_HEADER: &str = "\
llm-proxy-pii-rust — a local PII-masking proxy in front of an OpenAI-compatible LLM.

USAGE:
    llm-proxy-pii-rust                    Run the proxy (configuration is via environment
                                          variables — see below).
    llm-proxy-pii-rust --bench-providers  Measure which ONNX execution provider is fastest on
                                          this machine, print a report, and exit.
    llm-proxy-pii-rust --version          Print version, target and build features, and exit.
    llm-proxy-pii-rust --help             Print this message.

";

/// What a `--version` run reports.
///
/// A release asset is a bare executable: saved next to an older copy, **nothing identifies
/// it**. Version alone isn't enough either — "which build is this?" is really three
/// questions (which release, for which machine, with the ML layer or not), and the third
/// decides whether names are masked at all.
fn version_report() -> String {
    format!(
        "{} {}\ntarget: {}\nml (onnx feature): {}\n",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_TARGET"),
        if cfg!(feature = "onnx") {
            "compiled in — NER/GLiNER available when configured"
        } else {
            "NOT compiled in — structured recognizers only, names are never masked"
        },
    )
}

fn usage() -> String {
    format!("{USAGE_HEADER}{ENV_REFERENCE}")
}

/// What the argument scan decided to do.
enum Mode {
    Serve,
    BenchProviders,
    Version,
    Help,
}

/// Read every argument **before acting on any of them** (M9-R4, M10-R11).
///
/// The order matters and it is the whole point: an earlier version dispatched as it scanned,
/// so `--version --bogus` printed the version and exited **0** while `--bogus --version` was
/// correctly refused. M9-R4's sharp risk — starting a live proxy in response to a typo, while
/// the operator believed they had run a diagnostic — was not reopened by that (neither order
/// binds a listener), but "an unrecognized argument is a mistake" should not depend on where
/// the mistake sits in the line.
///
/// A later flag wins over an earlier one, which only matters for the nonsense case of passing
/// two; every argument is still checked.
fn parse_args<I: IntoIterator<Item = String>>(args: I) -> anyhow::Result<Mode> {
    let mut mode = Mode::Serve;
    for arg in args {
        mode = match arg.as_str() {
            BENCH_PROVIDERS_FLAG => Mode::BenchProviders,
            "--version" | "-V" => Mode::Version,
            "--help" | "-h" => Mode::Help,
            other => {
                anyhow::bail!(
                    "unrecognized argument {other:?}\n\n{}\nRefusing to start: an \
                     unrecognized argument is a mistake, and starting a live proxy in \
                     response to one would look like a diagnostic that never ran.",
                    usage()
                );
            }
        };
    }
    Ok(mode)
}

fn main() -> anyhow::Result<()> {
    // FIRST, before anything spawns a thread: the local UTC offset. `time` refuses to
    // answer once the process is multi-threaded (CVE-2020-26235), and the refusal is
    // platform-split — Windows usually answers, Linux and macOS do not — so doing this
    // late ships as "it looked right on my box". See `logging`.
    let offset = logging::local_offset();

    tracing_subscriber::fmt()
        .with_env_filter(logging::log_filter(
            std::env::var("RUST_LOG").ok().as_deref(),
        ))
        .with_timer(logging::timer(offset))
        .init();

    if offset.is_none() {
        // Honest UTC beats a silently wrong local time — that is the kind of thing that
        // costs an hour during an incident. Say it once, loudly, and never guess.
        tracing::warn!(
            "could not determine this machine's UTC offset — log timestamps are UTC \
             (+00:00), not local time"
        );
    }

    // **Unknown arguments are refused, never ignored (M9-R4).** Before M9 the binary took
    // no arguments, so discarding them was harmless; now a near-miss (`--bench-provider`,
    // `--bench-providers=1`, `--help`) would fall through and start a LIVE PROXY pointed at
    // the configured upstream, while the operator believed they had run a diagnostic. Every
    // other unexpected-input path in this codebase blocks or refuses — including M9's
    // `NER_EXECUTION_PROVIDER` typo check — so this one must not fail open.
    let mode = parse_args(std::env::args().skip(1))?;

    // `--version` and `--help` answer and exit *here*, before `Config::from_env` and before
    // the runtime exists: you must be able to ask what a binary is without holding a valid
    // upstream configuration.
    match mode {
        Mode::Version => {
            print!("{}", version_report());
            return Ok(());
        }
        Mode::Help => {
            print!("{}", usage());
            return Ok(());
        }
        Mode::Serve | Mode::BenchProviders => {}
    }

    // Only now is it safe to create threads.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        // Diagnostic mode: "which backend is fastest *here*?" is a hardware question the
        // project cannot answer for the operator, so ship the measurement rather than a
        // number. Handled before `Config::from_env` because the benchmark needs only the
        // NER model config, not a valid upstream/server config — you should be able to
        // benchmark without a provider key.
        if matches!(mode, Mode::BenchProviders) {
            return server::run_provider_benchmark().await;
        }

        let config = Config::from_env()?;
        tracing::info!(?config, "starting llm-proxy-pii-rust");

        server::run(config).await
    })
}

//! HTTP server: axum router and request handlers.
//!
//! Scope (fail-closed by default): `POST /v1/chat/completions` is always proxied
//! (OpenAI schema); `POST /v1/messages` (the native Anthropic schema, M6) is
//! proxied **only when `UPSTREAM_PROVIDER=anthropic`**; `GET /healthz` is served.
//! Every other path/method returns 404 and is **never forwarded** — an un-modelled
//! endpoint could leak PII, so we don't proxy what we don't understand.

use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, HeaderName, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tower_http::trace::TraceLayer;

use crate::config::{env_flag, Config};
use crate::pii::composite::CompositeDetector;
use crate::pii::recognizers::StructuredRecognizers;
use crate::pii::PiiDetector;
use crate::pipeline::privacy::PrivacyStage;
use crate::pipeline::{RequestContext, Stage, WireSchema};
use crate::proxy::{ProxyRequest, ProxyResponse, Upstream};
use crate::stream::SseDemasker;

/// Shared, cheaply-cloneable server state (axum clones it per request).
#[derive(Clone)]
pub struct AppState {
    upstream: Arc<Upstream>,
    stages: Arc<Vec<Box<dyn Stage>>>,
    max_body_bytes: usize,
    /// Allowlist of client request headers to pass through upstream (M3).
    forward_request_headers: Arc<Vec<String>>,
    /// Debug only (M2.6): skip the response de-mask so the client sees placeholders.
    debug_skip_demask: bool,
    /// Register the native Anthropic `/v1/messages` route (M6) — only when the
    /// upstream actually speaks it (`provider == "anthropic"`), so other providers
    /// never mis-route a native body they'd 404 anyway.
    serve_anthropic_messages: bool,
}

impl AppState {
    /// Build the state from config: the upstream client and the pipeline.
    ///
    /// Fallible because a **required** NER (`NER_REQUIRED`) that can't be loaded
    /// must be fatal at startup (fail closed), not silently downgraded. Async
    /// because an opt-in `hf-hub` model fetch (M2.5) is network I/O at startup.
    pub async fn new(config: &Config) -> anyhow::Result<Self> {
        let upstream = Upstream::new(
            config.upstream_base_url.clone(),
            config.upstream_api_key.clone(),
            config.upstream_chat_path.clone(),
            config.upstream_messages_path.clone(),
            config.upstream_extra_headers.clone(),
        );
        let ner_required = env_flag("NER_REQUIRED");
        let mut detector = build_detector(ner_required, &config.pii_locales).await?;
        // Content-keyed detection cache (S3, M7.1): the byte-identical system prompt Claude Code
        // re-sends every turn is detected once and reused, saving the dominant NER scan. Sound
        // because a hit is keyed on the exact bytes of a deterministic scan (see `pii::cache`);
        // `PII_CACHE_ENTRIES=0` opts out. Wrapping the whole composite means the cache sits above
        // both engines, so a hit skips the recognizers *and* the NER.
        if config.pii_cache_entries > 0 {
            detector = Box::new(crate::pii::cache::CachingDetector::new(
                detector,
                config.pii_cache_entries,
            ));
        }
        let stages: Vec<Box<dyn Stage>> = vec![Box::new(PrivacyStage::with_validation_budget(
            detector,
            config.pii_max_phone_validations,
        ))];
        if config.debug_skip_demask {
            // Loud, so it can't quietly linger in a real deployment (M2.6).
            tracing::warn!(
                "PII_DEBUG_SKIP_DEMASK is ON — responses are NOT de-masked; the client \
                 receives placeholders. DEBUG ONLY, never enable in production."
            );
        }
        Ok(Self {
            upstream: Arc::new(upstream),
            stages: Arc::new(stages),
            max_body_bytes: config.max_body_bytes,
            forward_request_headers: Arc::new(config.forward_request_headers.clone()),
            debug_skip_demask: config.debug_skip_demask,
            serve_anthropic_messages: config.provider == "anthropic",
        })
    }
}

/// Build the hybrid detector: the deterministic structured recognizers, plus the
/// ONNX NER (M2) when the `onnx` feature is on **and** the model env vars are set.
///
/// `ner_required` fails closed: a configured-but-unloadable NER (or, without the
/// `onnx` feature, requiring one at all) is a fatal error rather than a silent
/// structured-only downgrade (M2-R1).
///
/// Async so the `onnx` branch can `await` an opt-in `hf-hub` model fetch (M2.5).
/// `locales` selects the structured recognizers' national-ID coverage (M4).
async fn build_detector(
    ner_required: bool,
    locales: &[String],
) -> anyhow::Result<Box<dyn PiiDetector>> {
    let structured: Box<dyn PiiDetector> = Box::new(StructuredRecognizers::with_locales(locales));

    #[cfg(feature = "onnx")]
    {
        // The ML layer is one or both of the token-classification NER (M2, XLM-R) and the
        // GLiNER span detector (M8) — each opt-in via its own env. `NER_REQUIRED` means
        // "the ML layer must be present and must not silently degrade": at least one must
        // load, and every loaded one runs **unwrapped** so its errors fail the request
        // closed. Without it, each is `FailOpen`-wrapped (a per-request inference error is
        // swallowed to structured-only).
        let mut detectors = vec![structured];
        let mut ml_loaded = 0usize;

        // (M9) The execution backend both ML detectors run on — parsed ONCE, here, so the two
        // loads cannot disagree. Default is CPU; an unknown value is a startup config error
        // (fatal regardless of `NER_REQUIRED`), never a silent CPU run behind an operator who
        // believes a GPU is engaged. A *known* provider that fails to initialize falls back to
        // CPU later, in `build_session_reporting` — a different failure (see onnx::ExecutionProvider).
        let provider = crate::pii::onnx::ExecutionProvider::parse(
            &std::env::var("NER_EXECUTION_PROVIDER").unwrap_or_default(),
        )
        .map_err(|m| anyhow::anyhow!(m))?;

        // (a) Token-classification NER (M2).
        match load_onnx_ner(provider).await {
            Ok(Some(ner)) => {
                detectors.push(wrap_ml(ner, ner_required));
                ml_loaded += 1;
            }
            Ok(None) => {}
            Err(err) => {
                if ner_required {
                    return Err(err.context("NER_REQUIRED is set and the NER model failed to load"));
                }
                tracing::error!(error = %err, "NER configured but failed to load; continuing without it");
            }
        }
        // (b) GLiNER contextual / open-label span detector (M8) — opt-in.
        match load_gliner(provider).await {
            Ok(Some(gliner)) => {
                detectors.push(wrap_ml(gliner, ner_required));
                ml_loaded += 1;
            }
            Ok(None) => {}
            Err(err) => {
                if ner_required {
                    return Err(
                        err.context("NER_REQUIRED is set and the GLiNER model failed to load")
                    );
                }
                tracing::error!(error = %err, "GLiNER configured but failed to load; continuing without it");
            }
        }

        anyhow::ensure!(
            !ner_required || ml_loaded > 0,
            "NER_REQUIRED is set but no ML detector is configured (set NER_MODEL_PATH / NER_MODEL_REPO or GLINER_MODEL_PATH)"
        );
        Ok(Box::new(CompositeDetector::new(detectors)))
    }
    #[cfg(not(feature = "onnx"))]
    {
        anyhow::ensure!(
            !ner_required,
            "NER_REQUIRED is set but this binary was built without the `onnx` feature"
        );
        Ok(Box::new(CompositeDetector::new(vec![structured])))
    }
}

/// Resolve the NER **model source** from env — **the one home of that priority order**, shared
/// by the detector loader and the provider benchmark:
///
/// 1. **Explicit local files** — `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` +
///    `NER_LABELS` (comma-separated, class-id order). Zero outbound calls; this
///    is the airtight-privacy path and always wins.
/// 2. **Opt-in auto-download (M2.5)** — when `NER_MODEL_PATH` is unset but
///    `NER_MODEL_REPO` (`owner/name`) is set: fetch a revision-pinned model into
///    the standard HF cache via `hf-hub`. Tunable via `NER_MODEL_REVISION`
///    (default `478a2a3`, the picked XLM-R int8), `NER_MODEL_FILE`
///    (default `onnx/model_quantized.onnx`), `NER_TOKENIZER_FILE`
///    (default `tokenizer.json`), `NER_CONFIG_FILE` (default `config.json`).
///    `NER_LABELS` is optional here — derived from the model's `config.json`
///    `id2label` unless set explicitly.
///
/// The benchmark must measure *the model the proxy would actually run*; a second copy of this
/// resolution would let the two drift, and a benchmark that silently measured a different
/// model than the server loads is worse than no benchmark (it answers confidently and wrong —
/// the M7-R1 failure, in a new place). **Only the model *source* lives here** — the knobs that
/// shape how it is *run* (`NER_POOL_SIZE` / `NER_INTRA_THREADS` via `resolve_pool_and_intra`,
/// and `NER_TOKEN_TYPE_IDS`) are read by each caller, which must therefore read *all* of them:
/// carrying the path across but not the input contract is what made the benchmark unable to run
/// a BERT-family model (M9-R8).
///
/// `Ok(None)` = unconfigured (including a *partial* explicit config, where the companions are
/// missing); `Err` = configured but the fetch failed. Async so the auto-download can run on the
/// `tokio` runtime.
#[cfg(feature = "onnx")]
async fn resolve_ner_model() -> anyhow::Result<Option<(String, String, Vec<String>)>> {
    let labels_override = std::env::var("NER_LABELS").ok();

    if let Ok(model) = std::env::var("NER_MODEL_PATH") {
        // (1) Explicit local files — tokenizer + labels are required alongside.
        let (Some(tokenizer), Some(labels)) =
            (std::env::var("NER_TOKENIZER_PATH").ok(), labels_override)
        else {
            return Ok(None); // partial config → unconfigured
        };
        let id2label = labels.split(',').map(|s| s.trim().to_string()).collect();
        Ok(Some((model, tokenizer, id2label)))
    } else if let Ok(repo) = std::env::var("NER_MODEL_REPO") {
        // (2) Opt-in, revision-pinned fetch into the standard HF cache (M2.5).
        let spec = crate::pii::hf::HfModelSpec {
            repo,
            revision: env_or("NER_MODEL_REVISION", "478a2a3"),
            model_file: env_or("NER_MODEL_FILE", "onnx/model_quantized.onnx"),
            tokenizer_file: env_or("NER_TOKENIZER_FILE", "tokenizer.json"),
            config_file: env_or("NER_CONFIG_FILE", "config.json"),
        };
        let resolved = spec.resolve().await?;
        // An explicit NER_LABELS overrides the config-derived labels.
        let id2label = match labels_override {
            Some(labels) => labels.split(',').map(|s| s.trim().to_string()).collect(),
            None => resolved.id2label,
        };
        Ok(Some((
            resolved.model_path.to_string_lossy().into_owned(),
            resolved.tokenizer_path.to_string_lossy().into_owned(),
            id2label,
        )))
    } else {
        Ok(None) // not configured
    }
}

/// `--bench-providers`: measure every compiled-in execution provider against the configured
/// NER model **on this machine**, print the report, and exit. Never serves.
///
/// This exists because the right backend is a **hardware** question the project cannot answer
/// for the operator — M9 measured a box where the GPU *loses* to the CPU, and a different box
/// would flip that. See [`crate::pii::bench`] for the reasoning and the quantization trap the
/// report warns about.
#[cfg(feature = "onnx")]
pub async fn run_provider_benchmark() -> anyhow::Result<()> {
    let Some((model, _tokenizer, _id2label)) = resolve_ner_model().await? else {
        anyhow::bail!(
            "no NER model configured — set NER_MODEL_PATH (with NER_TOKENIZER_PATH and \
             NER_LABELS) or NER_MODEL_REPO. The benchmark deliberately measures the model the \
             proxy would actually run, so it needs the same configuration the server does."
        );
    };

    // The configured model, plus any extra variants to compare against it. The choice of
    // backend and the choice of quantization are **coupled** (CPU wants int8, a GPU wants
    // fp16), so answering "is the GPU worth it?" needs a matrix, not one model across
    // providers — `NER_BENCH_MODELS` is how the operator supplies the other corner.
    let mut models = vec![model];
    if let Ok(extra) = std::env::var("NER_BENCH_MODELS") {
        for candidate in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let candidate = candidate.to_string();
            if !models.contains(&candidate) {
                models.push(candidate);
            }
        }
    }

    // Resolve the thread shape through the SAME function the server uses (M9-R3). Measuring the
    // CPU with `available_cores()` when the operator set `NER_POOL_SIZE`/`NER_INTRA_THREADS`
    // benchmarks a configuration nobody runs — M7-R1's failure verbatim, and the reason
    // `resolve_pool_and_intra` has one home.
    let (pool_size, intra_threads) = crate::pii::onnx::resolve_pool_and_intra(
        std::env::var("NER_POOL_SIZE").ok().as_deref(),
        std::env::var("NER_INTRA_THREADS").ok().as_deref(),
        crate::pii::onnx::available_cores(),
    );
    // The model's input contract has to travel with the model path (M9-R8): a BERT-family model
    // needs a third graph input, and omitting it makes every row fail with a raw ORT error.
    let needs_token_type_ids = env_flag("NER_TOKEN_TYPE_IDS");

    let results = crate::pii::bench::benchmark_matrix(&models, intra_threads, needs_token_type_ids);
    println!(
        "{}",
        crate::pii::bench::format_report(&models, &results, pool_size, intra_threads)
    );
    Ok(())
}

/// Without the `onnx` feature there is no inference to accelerate. The command still
/// **runs and explains** rather than failing: `--bench-providers` must behave the same way in
/// every build, so an operator can always ask the question and always get a true, actionable
/// answer — here, "nothing to accelerate, and this is how you get a build that has something".
#[cfg(not(feature = "onnx"))]
pub async fn run_provider_benchmark() -> anyhow::Result<()> {
    // The advice must not name an `ep-*` feature (M9-R16, M9-R14's defect in this sibling
    // branch): each platform's accelerator is wired per-target in Cargo.toml, so `--features
    // onnx` already carries it — and `ep-directml` in particular is Windows-only, so naming it
    // to a macOS or Linux operator points at a backend no hardware of theirs can provide.
    println!(
        "Execution-provider benchmark (M9)\n\n\
         This binary was built WITHOUT the `onnx` feature, so it has no ML inference layer to\n\
         accelerate. Detection here is the deterministic recognizers only (email, phone, SSN,\n\
         credit card, IBAN, national ID) — plain CPU regex, unaffected by any execution\n\
         provider. There is nothing to benchmark.\n\n\
         To get the ML layer (the XLM-R NER) and compare backends:\n\
         \x20 cargo build --features onnx\n\n\
         That is all it takes — your platform's accelerator is included in that build (DirectML\n\
         on Windows, CoreML on macOS, CUDA on x86_64 Linux), so there is no second feature to\n\
         enable. Then re-run with --bench-providers."
    );
    Ok(())
}

/// Load the token-classification NER detector (M2, XLM-R) from env.
///
/// Resolves the model through [`resolve_ner_model`] (the shared priority order), the thread
/// shape through `onnx::resolve_pool_and_intra`, and runs it on `provider` — an accelerator that
/// cannot initialize falls back to CPU rather than failing startup, so what is *logged* is the
/// provider the detector reports back, never the one requested (M9-R1).
///
/// `Ok(None)` = unconfigured; `Err` = configured but failed to load/fetch (the caller decides if
/// that is fatal, per `NER_REQUIRED`). Async so the opt-in auto-download can run on the `tokio`
/// runtime.
#[cfg(feature = "onnx")]
async fn load_onnx_ner(
    provider: crate::pii::onnx::ExecutionProvider,
) -> anyhow::Result<Option<Box<dyn PiiDetector>>> {
    use crate::pii::onnx::OnnxNerDetector;

    // M7. Both knobs resolve in ONE place (`onnx::resolve_pool_and_intra`), which the latency
    // harness calls too — when each read its own default they silently disagreed (2 vs 1) and M7's
    // bar measured a config nobody ships (M7-R1). Explicit wins; otherwise `intra` is derived,
    // because the two knobs multiply and a fixed number is wrong on a 2-core VM and a 64-core
    // server alike. `0` is unset for both, never ONNX Runtime's "pick for me" (M7-R5).
    let (pool_size, intra_threads) = crate::pii::onnx::resolve_pool_and_intra(
        std::env::var("NER_POOL_SIZE").ok().as_deref(),
        std::env::var("NER_INTRA_THREADS").ok().as_deref(),
        crate::pii::onnx::available_cores(),
    );
    let needs_token_type_ids = env_flag("NER_TOKEN_TYPE_IDS");

    let Some((model, tokenizer, id2label)) = resolve_ner_model().await? else {
        return Ok(None); // not configured (NER_REQUIRED makes that fatal upstream)
    };

    let detector = OnnxNerDetector::load(
        &model,
        &tokenizer,
        id2label,
        pool_size,
        intra_threads,
        needs_token_type_ids,
        provider,
    )?;
    // EVERY field on this line is the **effective** value — that is the contract (M7-R5), and it
    // is why `provider` is read back off the detector rather than echoed from the request: a
    // requested accelerator that failed to initialize falls back to CPU, and a startup line
    // naming a GPU the process is not using is precisely the defect this rule exists to prevent
    // (M9-R1). The pool is homogeneous by construction, so one provider describes all of it.
    log_ml_detector_loaded(
        "ONNX NER",
        &model,
        pool_size,
        intra_threads,
        provider,
        detector.provider(),
    );
    Ok(Some(Box::new(detector)))
}

/// Log a loaded ML detector's **effective** configuration, naming the requested provider only
/// when it differs from what was obtained — so a fallback is legible on the one line that claims
/// to describe the running config, instead of only in a `warn!` 300 lines earlier that is
/// invisible at `RUST_LOG=error` (M9-R1).
#[cfg(feature = "onnx")]
fn log_ml_detector_loaded(
    what: &str,
    model: &str,
    pool_size: usize,
    intra_threads: usize,
    requested: crate::pii::onnx::ExecutionProvider,
    effective: crate::pii::onnx::ExecutionProvider,
) {
    if requested == effective {
        tracing::info!(
            model,
            pool_size,
            intra_threads,
            provider = effective.as_str(),
            "{what} detector loaded"
        );
    } else {
        tracing::warn!(
            model,
            pool_size,
            intra_threads,
            provider = effective.as_str(),
            requested = requested.as_str(),
            "{what} detector loaded on a FALLBACK backend — the requested accelerator is not in use"
        );
    }
}

/// Wrap an ML detector for the composite: **unwrapped** under `NER_REQUIRED` (its errors
/// fail the request closed), else `FailOpen` (a per-request inference error is swallowed
/// to structured-only). Shared by the NER and GLiNER so the posture rule has one home.
#[cfg(feature = "onnx")]
fn wrap_ml(detector: Box<dyn PiiDetector>, ner_required: bool) -> Box<dyn PiiDetector> {
    if ner_required {
        detector
    } else {
        Box::new(crate::pii::composite::FailOpen(detector))
    }
}

/// Load the GLiNER span detector (M8) from env — **opt-in, off by default**. GLiNER is
/// *not* a drop-in successor to the XLM-R NER: on the shipped **int8** model its Person
/// recall (~0.58) is below XLM-R's (~0.83), so enabling it does not replace the NER — it
/// **adds** contextual, open-label detection (a bare national phone, a free-form address)
/// that the deterministic layer can't anchor and XLM-R doesn't cover (DEVLOG M8, the eval).
///
/// **Explicit local files only** — `GLINER_MODEL_PATH` + `GLINER_TOKENIZER_PATH` +
/// `GLINER_CONFIG_PATH` (the model's `gliner_config.json`, for the shape params). This is
/// the airtight-privacy path (zero outbound calls); an `hf-hub` auto-download parity with
/// the NER is a documented future addition. Tunables: `GLINER_LABELS` (comma-separated
/// natural-language types; default person/organization/location/phone number/address),
/// `GLINER_THRESHOLD`, `GLINER_POOL_SIZE`, `GLINER_INTRA_THREADS`. `Ok(None)` only when
/// `GLINER_MODEL_PATH` is unset (opt-out); once it is set, a missing companion var or a broken
/// `GLINER_THRESHOLD` is an **`Err`** (M8-R5) — never a silent disable — and the caller decides
/// if that's fatal (`NER_REQUIRED`) or logged-and-skipped.
#[cfg(feature = "onnx")]
async fn load_gliner(
    provider: crate::pii::onnx::ExecutionProvider,
) -> anyhow::Result<Option<Box<dyn PiiDetector>>> {
    use crate::pii::gliner::{GLiNerDetector, GlinerParams, DEFAULT_THRESHOLD};
    use crate::pii::gliner_decode::{default_gliner_labels, parse_gliner_labels};

    let Ok(model) = std::env::var("GLINER_MODEL_PATH") else {
        return Ok(None); // not configured — GLiNER is opt-in
    };
    // `GLINER_MODEL_PATH` is set → GLiNER is *intended*. A missing companion is a config error,
    // **not a silent opt-out** (M8-R5): a typo'd `GLINER_TOKENIZER_PATH` must surface, not quietly
    // disable an opt-in security feature. `Err` flows to the caller's posture (fatal under
    // `NER_REQUIRED`, else logged and skipped) rather than vanishing as `Ok(None)`.
    let (Some(tokenizer), Some(config)) = (
        std::env::var("GLINER_TOKENIZER_PATH").ok(),
        std::env::var("GLINER_CONFIG_PATH").ok(),
    ) else {
        return Err(anyhow::anyhow!(
            "GLINER_MODEL_PATH is set but GLINER_TOKENIZER_PATH and/or GLINER_CONFIG_PATH is missing"
        ));
    };

    let config_json = std::fs::read_to_string(&config)
        .map_err(|e| anyhow::anyhow!("read GLiNER config {config}: {e}"))?;
    let params = GlinerParams::from_config_json(&config_json)?;
    let labels = match std::env::var("GLINER_LABELS") {
        Ok(spec) => parse_gliner_labels(&spec).map_err(|e| anyhow::anyhow!(e))?,
        Err(_) => default_gliner_labels(),
    };
    // A *set* threshold must parse and be a probability. Silently defaulting on a parse error, or
    // accepting `2.0` (→ nothing ever masks), would quietly disable the detector (M8-R5). Unset =
    // the measured default.
    let threshold = match std::env::var("GLINER_THRESHOLD") {
        Ok(s) => {
            let t: f32 = s
                .parse()
                .map_err(|_| anyhow::anyhow!("GLINER_THRESHOLD is not a number: {s:?}"))?;
            anyhow::ensure!(
                (0.0..=1.0).contains(&t),
                "GLINER_THRESHOLD must be a probability in [0.0, 1.0], got {t}"
            );
            t
        }
        Err(_) => DEFAULT_THRESHOLD,
    };
    // Same pool/intra derivation as the NER (its own env), so an operator running both
    // models controls each independently (the two knobs multiply — M7).
    let (pool_size, intra_threads) = crate::pii::onnx::resolve_pool_and_intra(
        std::env::var("GLINER_POOL_SIZE").ok().as_deref(),
        std::env::var("GLINER_INTRA_THREADS").ok().as_deref(),
        crate::pii::onnx::available_cores(),
    );

    let detector = GLiNerDetector::load(
        &model,
        &tokenizer,
        labels,
        params,
        threshold,
        pool_size,
        intra_threads,
        provider,
    )?;
    // Effective, not requested — same contract as the NER's line (M9-R1).
    tracing::info!(threshold, "GLiNER threshold");
    log_ml_detector_loaded(
        "GLiNER",
        &model,
        pool_size,
        intra_threads,
        provider,
        detector.provider(),
    );
    Ok(Some(Box::new(detector)))
}

/// Read an env var, falling back to `default` when unset.
#[cfg(feature = "onnx")]
fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Build the router. Exposed so integration tests can serve it on an ephemeral
/// port without going through [`run`].
pub fn build_router(state: AppState) -> Router {
    let max_body_bytes = state.max_body_bytes;
    let mut router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/chat/completions", post(chat_completions));
    // Native Anthropic Messages (M6) — registered only when the upstream speaks
    // it; otherwise `/v1/messages` stays a 404 like any un-modelled endpoint.
    if state.serve_anthropic_messages {
        router = router.route("/v1/messages", post(messages));
    }
    router
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .layer(TraceLayer::new_for_http())
        .fallback(not_proxied)
        .with_state(state)
}

/// Bind the listener and serve until shutdown.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let app = build_router(AppState::new(&config).await?);
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Any route we don't explicitly proxy → 404, never forwarded (fail closed).
async fn not_proxied() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": { "message": "endpoint not proxied", "type": "not_found" }
        })),
    )
        .into_response()
}

/// `POST /v1/chat/completions`: mask → forward → restore (OpenAI schema). Handles
/// both buffered JSON and streaming (SSE) responses; request-side masking is
/// identical for both.
async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);

    let (request, mut ctx) = match run_privacy_stages(&state, body, WireSchema::OpenAi).await {
        Ok(pair) => pair,
        Err(response) => return response,
    };

    let client_auth = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let passthrough = collect_passthrough(&headers, &state.forward_request_headers);

    if streaming {
        let sent = state
            .upstream
            .send(&request.body, client_auth, &passthrough)
            .await;
        return finish_streaming(&state, sent, &mut ctx, WireSchema::OpenAi).await;
    }

    // ── Non-streaming (buffered JSON) ────────────────────────────────────────
    match state
        .upstream
        .forward_chat_completions(&request.body, client_auth, &passthrough)
        .await
    {
        Ok((status, upstream_headers, upstream_body)) => {
            finish_buffered(&state, status, &upstream_headers, upstream_body, &mut ctx)
        }
        Err(err) => {
            tracing::error!(error = %err, "upstream forwarding failed");
            error_response(StatusCode::BAD_GATEWAY, &err.to_string(), "proxy_error")
        }
    }
}

/// `POST /v1/messages`: mask → forward → restore for the **native Anthropic**
/// schema (M6, Claude Code passthrough). Registered only when the upstream speaks
/// it. Same mask + fail-closed + streaming-detect flow as `chat_completions`,
/// differing only in the schema tag, the native forward, and the SSE demasker.
async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);

    let (request, mut ctx) = match run_privacy_stages(&state, body, WireSchema::Anthropic).await {
        Ok(pair) => pair,
        Err(response) => return response,
    };

    // Native auth (M6): client `Authorization` (OAuth Bearer) wins, then a client
    // `x-api-key`, then the proxy's configured key — never OAuth in `x-api-key`.
    // No usable credential → 401 (masking already ran; nothing is forwarded).
    let client_auth = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let client_api_key = headers.get("x-api-key").and_then(|v| v.to_str().ok());
    let auth = match state.upstream.messages_auth(client_auth, client_api_key) {
        Some(auth) => auth,
        None => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "no Anthropic credential (send Authorization: Bearer <token> or x-api-key)",
                "unauthorized",
            )
        }
    };
    let passthrough = collect_passthrough(&headers, &state.forward_request_headers);

    if streaming {
        let sent = state
            .upstream
            .send_messages(&request.body, &auth, &passthrough)
            .await;
        return finish_streaming(&state, sent, &mut ctx, WireSchema::Anthropic).await;
    }

    // ── Non-streaming (buffered JSON) ────────────────────────────────────────
    match state
        .upstream
        .forward_messages(&request.body, &auth, &passthrough)
        .await
    {
        Ok((status, upstream_headers, upstream_body)) => {
            finish_buffered(&state, status, &upstream_headers, upstream_body, &mut ctx)
        }
        Err(err) => {
            tracing::error!(error = %err, "upstream forwarding failed (messages)");
            error_response(StatusCode::BAD_GATEWAY, &err.to_string(), "proxy_error")
        }
    }
}

/// Run the request through every stage's masking hook on the blocking pool, then
/// apply the fail-closed gates. Shared by both schema handlers (M6); only the
/// `schema` tag differs, which selects the stage's mask/demask walk.
///
/// Masking is **CPU-bound** — regex scans over every text field, plus NER
/// inference when it's on — so it runs on the blocking pool, never inline on a
/// tokio worker (M4-R19). Inline, a few concurrent large bodies starve the
/// executor and the whole proxy stops serving, on an unauthenticated path
/// (detection precedes any upstream auth).
///
/// `Err(Response)` is a ready-to-return failure: a masking-task panic → 500, or a
/// stage that refused to mask → 400. In both cases nothing is forwarded.
async fn run_privacy_stages(
    state: &AppState,
    body: Value,
    schema: WireSchema,
) -> Result<(ProxyRequest, RequestContext), Response> {
    let stages = state.stages.clone();
    let masked = tokio::task::spawn_blocking(move || {
        let mut ctx = RequestContext::new();
        ctx.schema = schema;
        let mut request = ProxyRequest { body };
        for stage in stages.iter() {
            stage.on_request(&mut request, &mut ctx);
            if ctx.block.is_some() {
                break;
            }
        }
        (request, ctx)
    })
    .await;

    // Fail closed: if the masking task itself died (a panic in a stage), we hold a
    // request whose PII status is unknown — reject it, never forward it.
    let (request, ctx) = match masked {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!(error = %err, "masking task failed; blocking the request (fail-closed)");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "masking failed",
                "blocked",
            ));
        }
    };

    // Fail closed: a stage refused to mask this request → reject, don't forward.
    if let Some(reason) = &ctx.block {
        tracing::warn!(reason = %reason, "request blocked (fail-closed)");
        return Err(error_response(StatusCode::BAD_REQUEST, reason, "blocked"));
    }

    // M2.6: the outgoing body is masked here (placeholders only), so it is safe to
    // dump at `trace!` for debugging — opt in with `RUST_LOG=…=trace`. The concise
    // kind-only audit stays at `debug!`. NEVER log the de-masked response.
    tracing::trace!(masked_request = %request.body, "forwarding masked request body upstream");

    Ok((request, ctx))
}

/// Restore a buffered (non-streaming) upstream JSON reply and turn it into the
/// client response. Schema-agnostic: `on_response` dispatches on `ctx.schema`.
fn finish_buffered(
    state: &AppState,
    status: u16,
    upstream_headers: &reqwest::header::HeaderMap,
    upstream_body: Value,
    ctx: &mut RequestContext,
) -> Response {
    let mut response = ProxyResponse {
        body: upstream_body,
    };
    // M2.6 debug: when skipping de-mask, the client deliberately receives the
    // placeholders the provider saw. Request-side masking already ran, so this
    // never leaks raw PII upstream — it only changes what the (local) client sees.
    if state.debug_skip_demask {
        tracing::debug!("skipping response de-mask (PII_DEBUG_SKIP_DEMASK)");
    } else {
        for stage in state.stages.iter().rev() {
            stage.on_response(&mut response, ctx);
        }
    }

    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let forwarded = forwardable_headers(upstream_headers);
    (code, forwarded, Json(response.body)).into_response()
}

/// Finish a streaming round-trip: if the upstream really answered with SSE,
/// de-anonymize the token stream incrementally on the way back (M3), else fall
/// back to the buffered path (M3-R1). Shared by both schemas (M6) — `schema`
/// selects the per-wire-format SSE rewriter. Request-side masking has already run,
/// so nothing raw ever reaches the provider regardless.
async fn finish_streaming(
    state: &AppState,
    sent: anyhow::Result<reqwest::Response>,
    ctx: &mut RequestContext,
    schema: WireSchema,
) -> Response {
    let response = match sent {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(error = %err, "upstream streaming request failed");
            return error_response(StatusCode::BAD_GATEWAY, &err.to_string(), "proxy_error");
        }
    };

    let code = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    // M3-R1: only stream if the upstream actually answered with SSE. A JSON error
    // (401/429/…) or a provider that ignored `stream` is served through the buffered
    // path instead — the client gets the real content-type + status, with de-masking.
    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false);
    if !is_sse {
        return buffered_fallback(state, response, code, ctx).await;
    }

    let mut out_headers = forwardable_headers(response.headers());
    out_headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));

    let upstream = response.bytes_stream();
    // Clean request (nothing masked) or debug-skip → stream through untouched;
    // otherwise de-anonymize each delta with the hold-back buffer.
    let body = if state.debug_skip_demask || ctx.vault.is_empty() {
        if state.debug_skip_demask {
            tracing::debug!("skipping response de-mask (PII_DEBUG_SKIP_DEMASK)");
        }
        Body::from_stream(upstream)
    } else {
        demasking_sse_body(upstream, std::mem::take(&mut ctx.vault), schema)
    };

    (code, out_headers, body).into_response()
}

/// A `stream:true` request the upstream answered without SSE (a JSON error, or a
/// provider that ignored `stream`): buffer it, de-mask, and forward like a normal
/// non-streaming reply so the client sees the real status + content-type (M3-R1).
async fn buffered_fallback(
    state: &AppState,
    response: reqwest::Response,
    code: StatusCode,
    ctx: &mut RequestContext,
) -> Response {
    let upstream_headers = response.headers().clone();
    match response.json::<Value>().await {
        Ok(json) => {
            let mut resp = ProxyResponse { body: json };
            if state.debug_skip_demask {
                tracing::debug!("skipping response de-mask (PII_DEBUG_SKIP_DEMASK)");
            } else {
                for stage in state.stages.iter().rev() {
                    stage.on_response(&mut resp, ctx);
                }
            }
            let forwarded = forwardable_headers(&upstream_headers);
            (code, forwarded, Json(resp.body)).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "streaming request: upstream returned neither SSE nor JSON");
            error_response(
                code,
                "upstream returned a non-SSE, non-JSON response",
                "proxy_error",
            )
        }
    }
}

/// Wrap an upstream SSE byte stream in an incremental de-anonymizer (M3).
///
/// Generic over the stream error so it is unit-testable without a real HTTP round
/// trip. A mid-stream upstream error is turned into a **terminal SSE `event: error`**
/// (after flushing any buffered content) and the stream ends cleanly, rather than
/// aborting the client connection.
fn demasking_sse_body<S, E>(
    upstream: S,
    vault: crate::pii::anonymizer::Vault,
    schema: WireSchema,
) -> Body
where
    S: futures_util::Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    struct StreamState<S> {
        inner: std::pin::Pin<Box<S>>,
        demasker: SseDemasker,
        ended: bool,
    }

    let state = StreamState {
        inner: Box::pin(upstream),
        demasker: SseDemasker::new(vault, schema),
        ended: false,
    };

    let stream = futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if st.ended {
                return None;
            }
            match st.inner.next().await {
                Some(Ok(chunk)) => {
                    let out = st.demasker.push(chunk.as_ref());
                    if out.is_empty() {
                        continue; // no complete line yet — pull more
                    }
                    return Some((Ok::<Bytes, std::convert::Infallible>(Bytes::from(out)), st));
                }
                Some(Err(err)) => {
                    st.ended = true;
                    // Flush buffered content, then a terminal error event; end clean.
                    let mut out = st.demasker.flush();
                    out.extend_from_slice(sse_error_event(&err.to_string()).as_bytes());
                    return Some((Ok(Bytes::from(out)), st));
                }
                None => {
                    st.ended = true;
                    let tail = st.demasker.flush();
                    if tail.is_empty() {
                        return None;
                    }
                    return Some((Ok(Bytes::from(tail)), st));
                }
            }
        }
    });

    Body::from_stream(stream)
}

/// A terminal SSE error event. The message is an upstream transport/decoding error
/// (no input text / PII).
fn sse_error_event(message: &str) -> String {
    let payload = json!({
        "error": { "message": format!("upstream stream error: {message}"), "type": "proxy_error" }
    });
    format!("event: error\ndata: {payload}\n\n")
}

/// Collect the allowlisted client request headers to pass through upstream (M3).
fn collect_passthrough(
    client: &HeaderMap,
    allow: &[String],
) -> Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)> {
    let mut out = Vec::new();
    for name in allow {
        if let Some(value) = client.get(name.as_str()) {
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
            ) {
                out.push((n, v));
            }
        }
    }
    out
}

/// A small JSON error body with a status code.
fn error_response(status: StatusCode, message: &str, kind: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "message": message, "type": kind } })),
    )
        .into_response()
}

/// Copy the safe subset of upstream response headers to forward to the client.
///
/// Allowlist only: informational headers (rate-limit, request id, `retry-after`,
/// provider-specific `openai-*` / `anthropic-*`). We deliberately drop
/// `content-length`/`content-type`/`transfer-encoding`/`connection` etc. — the
/// body is re-serialized here, so those would be wrong.
fn forwardable_headers(upstream: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in upstream {
        if !is_forwardable(name.as_str()) {
            continue;
        }
        // Rebuild from bytes so this doesn't depend on reqwest and axum sharing
        // the exact same `http` crate version.
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            out.insert(name, value);
        }
    }
    out
}

/// Whether an upstream response header is safe to forward to the client.
fn is_forwardable(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "retry-after"
        || name == "x-request-id"
        || name.starts_with("x-ratelimit")
        || name.starts_with("openai-")
        || name.starts_with("anthropic-")
}

#[cfg(test)]
mod tests {
    use super::{build_detector, demasking_sse_body, Bytes};
    use crate::pii::anonymizer::Vault;

    #[tokio::test]
    async fn required_ner_is_fatal_when_absent() {
        // M2-R1: requiring a NER that can't be present (no `onnx` feature, or —
        // with the feature — no model configured) is fatal.
        //
        // `build_detector` reads the model-source env vars, so "no model configured" must be made
        // true for this process rather than assumed (M8-R6): a developer who set `GLINER_MODEL_PATH`
        // / `NER_MODEL_PATH` to run the `#[ignore]`d gated tests would otherwise see this fail,
        // because GLiNER/NER would actually load. `remove_var` is safe here not because nothing else
        // reads env concurrently (other lib tests read `HOME`/`HF_*`), but because every reader in
        // this binary goes through `std::env`'s lock, and no **FFI/C** `getenv` reader (which would
        // race off-lock) runs in the lib-test process — the ONNX session that could is only in the
        // separate gated-test integration binary. (edition 2021 → `remove_var` is not `unsafe`.)
        for var in ["NER_MODEL_PATH", "NER_MODEL_REPO", "GLINER_MODEL_PATH"] {
            std::env::remove_var(var);
        }
        assert!(build_detector(true, &[]).await.is_err());
        // Not requiring it always yields a structured-only detector.
        assert!(build_detector(false, &[]).await.is_ok());
    }

    #[tokio::test]
    async fn mid_stream_upstream_error_becomes_terminal_sse_event() {
        // M3 follow-up: an error partway through the stream is turned into a
        // terminal `event: error` (after flushing buffered content), not a broken
        // connection. Injected via a synthetic stream — no HTTP round-trip.
        let chunk = Bytes::from(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n".to_string(),
        );
        let upstream = futures_util::stream::iter(vec![
            Ok::<Bytes, std::io::Error>(chunk),
            Err(std::io::Error::other("boom")),
        ]);
        let body = demasking_sse_body(upstream, Vault::new(), crate::pipeline::WireSchema::OpenAi);
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(
            text.contains("\"content\":\"hi\""),
            "content received before the error must survive: {text}"
        );
        assert!(
            text.contains("event: error"),
            "a terminal SSE error event must be emitted: {text}"
        );
        assert!(
            text.contains("proxy_error"),
            "error payload present: {text}"
        );
    }
}

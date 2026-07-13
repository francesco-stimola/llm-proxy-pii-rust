//! HTTP server: axum router and request handlers.
//!
//! Scope (fail-closed by default): only `POST /v1/chat/completions` is proxied
//! and `GET /healthz` is served. Every other path/method returns 404 and is
//! **never forwarded** — an un-modelled endpoint could leak PII, so we don't
//! proxy what we don't understand.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tower_http::trace::TraceLayer;

use crate::config::{Config, env_flag};
use crate::pii::PiiDetector;
use crate::pii::composite::CompositeDetector;
use crate::pii::recognizers::StructuredRecognizers;
use crate::pipeline::privacy::PrivacyStage;
use crate::pipeline::{RequestContext, Stage};
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
            config.upstream_extra_headers.clone(),
        );
        let ner_required = env_flag("NER_REQUIRED");
        let stages: Vec<Box<dyn Stage>> = vec![Box::new(PrivacyStage::new(
            build_detector(ner_required, &config.pii_locales).await?,
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
        let mut detectors = vec![structured];
        match load_onnx_ner().await {
            Ok(Some(ner)) => {
                // Required → propagate its errors (fail closed); otherwise wrap
                // so a per-request inference error is swallowed (fail open).
                detectors.push(if ner_required {
                    ner
                } else {
                    Box::new(crate::pii::composite::FailOpen(ner))
                });
            }
            Ok(None) => anyhow::ensure!(
                !ner_required,
                "NER_REQUIRED is set but the NER is not configured (set NER_MODEL_PATH / NER_TOKENIZER_PATH / NER_LABELS)"
            ),
            Err(err) => {
                if ner_required {
                    return Err(err.context("NER_REQUIRED is set and the NER model failed to load"));
                }
                tracing::error!(error = %err, "NER configured but failed to load; running structured-only");
            }
        }
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

/// Load the ONNX NER detector from env. The model source is resolved in priority
/// order (fail-closed defaults, no surprise outbound calls):
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
/// Common to both: optional `NER_POOL_SIZE`, `NER_TOKEN_TYPE_IDS`. `Ok(None)` =
/// unconfigured; `Err` = configured but failed to load/fetch (the caller decides
/// if that's fatal). Async so the auto-download can run on the `tokio` runtime.
#[cfg(feature = "onnx")]
async fn load_onnx_ner() -> anyhow::Result<Option<Box<dyn PiiDetector>>> {
    use crate::pii::onnx::OnnxNerDetector;

    let pool_size = std::env::var("NER_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let needs_token_type_ids = env_flag("NER_TOKEN_TYPE_IDS");
    let labels_override = std::env::var("NER_LABELS").ok();

    // Resolve (model path, tokenizer path, id2label) from one of the two sources.
    let (model, tokenizer, id2label) = if let Ok(model) = std::env::var("NER_MODEL_PATH") {
        // (1) Explicit local files — tokenizer + labels are required alongside.
        let (Some(tokenizer), Some(labels)) =
            (std::env::var("NER_TOKENIZER_PATH").ok(), labels_override)
        else {
            return Ok(None); // partial config → unconfigured (NER_REQUIRED makes it fatal)
        };
        let id2label = labels.split(',').map(|s| s.trim().to_string()).collect();
        (model, tokenizer, id2label)
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
        (
            resolved.model_path.to_string_lossy().into_owned(),
            resolved.tokenizer_path.to_string_lossy().into_owned(),
            id2label,
        )
    } else {
        return Ok(None); // not configured
    };

    let detector =
        OnnxNerDetector::load(&model, &tokenizer, id2label, pool_size, needs_token_type_ids)?;
    tracing::info!(model, pool_size, "ONNX NER detector loaded");
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
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/chat/completions", post(chat_completions))
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

/// `POST /v1/chat/completions`: mask → forward → restore. Handles both buffered
/// JSON and streaming (SSE) responses; request-side masking is identical for both.
async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);

    // Masking is **CPU-bound** — regex scans over every text field, plus NER inference when
    // it's on — so it runs on the blocking pool, never inline on a tokio worker (M4-R19).
    // Inline, a few concurrent large bodies starve the executor and the whole proxy stops
    // serving, on an unauthenticated path (detection precedes any upstream auth).
    let stages = state.stages.clone();
    let masked = tokio::task::spawn_blocking(move || {
        let mut ctx = RequestContext::new();
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

    // Fail closed: if the masking task itself died (a panic in a stage), we hold a request
    // whose PII status is unknown — reject it, never forward it.
    let (request, ctx) = match masked {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!(error = %err, "masking task failed; blocking the request (fail-closed)");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "masking failed",
                "blocked",
            );
        }
    };
    let mut ctx = ctx;

    // Fail closed: a stage refused to mask this request → reject, don't forward.
    if let Some(reason) = ctx.block {
        tracing::warn!(reason = %reason, "request blocked (fail-closed)");
        return error_response(StatusCode::BAD_REQUEST, &reason, "blocked");
    }

    // M2.6: the outgoing body is masked here (placeholders only), so it is safe to
    // dump at `trace!` for debugging — opt in with `RUST_LOG=…=trace`. The concise
    // kind-only audit stays at `debug!`. NEVER log the de-masked response below.
    tracing::trace!(masked_request = %request.body, "forwarding masked request body upstream");

    let client_auth = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let passthrough = collect_passthrough(&headers, &state.forward_request_headers);

    if streaming {
        return stream_chat_completions(&state, &request.body, client_auth, &passthrough, &mut ctx)
            .await;
    }

    // ── Non-streaming (buffered JSON) ────────────────────────────────────────
    let (status, upstream_headers, upstream_body) = match state
        .upstream
        .forward_chat_completions(&request.body, client_auth, &passthrough)
        .await
    {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!(error = %err, "upstream forwarding failed");
            return error_response(StatusCode::BAD_GATEWAY, &err.to_string(), "proxy_error");
        }
    };

    let mut response = ProxyResponse { body: upstream_body };
    // M2.6 debug: when skipping de-mask, the client deliberately receives the
    // placeholders the provider saw. Request-side masking already ran, so this
    // never leaks raw PII upstream — it only changes what the (local) client sees.
    if state.debug_skip_demask {
        tracing::debug!("skipping response de-mask (PII_DEBUG_SKIP_DEMASK)");
    } else {
        for stage in state.stages.iter().rev() {
            stage.on_response(&mut response, &mut ctx);
        }
    }

    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let forwarded = forwardable_headers(&upstream_headers);
    (code, forwarded, Json(response.body)).into_response()
}

/// Streaming (SSE) round-trip: forward the masked body, then de-anonymize the
/// token stream incrementally on the way back (M3). Request-side masking has
/// already run, so nothing raw ever reaches the provider regardless.
async fn stream_chat_completions(
    state: &AppState,
    body: &Value,
    client_auth: Option<&str>,
    passthrough: &[(reqwest::header::HeaderName, reqwest::header::HeaderValue)],
    ctx: &mut RequestContext,
) -> Response {
    let response = match state.upstream.send(body, client_auth, passthrough).await {
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
        demasking_sse_body(upstream, std::mem::take(&mut ctx.vault))
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
            error_response(code, "upstream returned a non-SSE, non-JSON response", "proxy_error")
        }
    }
}

/// Wrap an upstream SSE byte stream in an incremental de-anonymizer (M3).
///
/// Generic over the stream error so it is unit-testable without a real HTTP round
/// trip. A mid-stream upstream error is turned into a **terminal SSE `event: error`**
/// (after flushing any buffered content) and the stream ends cleanly, rather than
/// aborting the client connection.
fn demasking_sse_body<S, E>(upstream: S, vault: crate::pii::anonymizer::Vault) -> Body
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
        demasker: SseDemasker::new(vault),
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
    use super::{Bytes, build_detector, demasking_sse_body};
    use crate::pii::anonymizer::Vault;

    #[tokio::test]
    async fn required_ner_is_fatal_when_absent() {
        // M2-R1: requiring a NER that can't be present (no `onnx` feature, or —
        // with the feature — no model configured in the test env) is fatal.
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
        let body = demasking_sse_body(upstream, Vault::new());
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(
            text.contains("\"content\":\"hi\""),
            "content received before the error must survive: {text}"
        );
        assert!(text.contains("event: error"), "a terminal SSE error event must be emitted: {text}");
        assert!(text.contains("proxy_error"), "error payload present: {text}");
    }
}

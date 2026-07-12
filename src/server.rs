//! HTTP server: axum router and request handlers.
//!
//! Scope (fail-closed by default): only `POST /v1/chat/completions` is proxied
//! and `GET /healthz` is served. Every other path/method returns 404 and is
//! **never forwarded** — an un-modelled endpoint could leak PII, so we don't
//! proxy what we don't understand.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::pii::PiiDetector;
use crate::pii::composite::CompositeDetector;
use crate::pii::recognizers::StructuredRecognizers;
use crate::pipeline::privacy::PrivacyStage;
use crate::pipeline::{RequestContext, Stage};
use crate::proxy::{ProxyRequest, ProxyResponse, Upstream};

/// Shared, cheaply-cloneable server state (axum clones it per request).
#[derive(Clone)]
pub struct AppState {
    upstream: Arc<Upstream>,
    stages: Arc<Vec<Box<dyn Stage>>>,
    max_body_bytes: usize,
}

impl AppState {
    /// Build the state from config: the upstream client and the pipeline.
    ///
    /// Fallible because a **required** NER (`NER_REQUIRED`) that can't be loaded
    /// must be fatal at startup (fail closed), not silently downgraded.
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let upstream = Upstream::new(
            config.upstream_base_url.clone(),
            config.upstream_api_key.clone(),
        );
        let ner_required = env_flag("NER_REQUIRED");
        let stages: Vec<Box<dyn Stage>> =
            vec![Box::new(PrivacyStage::new(build_detector(ner_required)?))];
        Ok(Self {
            upstream: Arc::new(upstream),
            stages: Arc::new(stages),
            max_body_bytes: config.max_body_bytes,
        })
    }
}

/// Read a boolean-ish env flag (`1` / `true` / `yes` / `on`, case-insensitive).
fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Build the hybrid detector: the deterministic structured recognizers, plus the
/// ONNX NER (M2) when the `onnx` feature is on **and** the model env vars are set.
///
/// `ner_required` fails closed: a configured-but-unloadable NER (or, without the
/// `onnx` feature, requiring one at all) is a fatal error rather than a silent
/// structured-only downgrade (M2-R1).
fn build_detector(ner_required: bool) -> anyhow::Result<Box<dyn PiiDetector>> {
    let structured: Box<dyn PiiDetector> = Box::new(StructuredRecognizers::new());

    #[cfg(feature = "onnx")]
    {
        let mut detectors = vec![structured];
        match load_onnx_ner() {
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

/// Load the ONNX NER detector from env (`NER_MODEL_PATH`, `NER_TOKENIZER_PATH`,
/// `NER_LABELS` = comma-separated labels in class-id order, optional
/// `NER_POOL_SIZE`, `NER_TOKEN_TYPE_IDS`). `Ok(None)` = unconfigured; `Err` =
/// configured but failed to load (the caller decides if that's fatal).
#[cfg(feature = "onnx")]
fn load_onnx_ner() -> anyhow::Result<Option<Box<dyn PiiDetector>>> {
    use crate::pii::onnx::OnnxNerDetector;

    let (model, tokenizer, labels) = match (
        std::env::var("NER_MODEL_PATH").ok(),
        std::env::var("NER_TOKENIZER_PATH").ok(),
        std::env::var("NER_LABELS").ok(),
    ) {
        (Some(m), Some(t), Some(l)) => (m, t, l),
        _ => return Ok(None), // not configured
    };

    let id2label: Vec<String> = labels.split(',').map(|s| s.trim().to_string()).collect();
    let pool_size = std::env::var("NER_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let needs_token_type_ids = env_flag("NER_TOKEN_TYPE_IDS");

    let detector =
        OnnxNerDetector::load(&model, &tokenizer, id2label, pool_size, needs_token_type_ids)?;
    tracing::info!(model, pool_size, "ONNX NER detector loaded");
    Ok(Some(Box::new(detector)))
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
    let app = build_router(AppState::new(&config)?);
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

/// `POST /v1/chat/completions`: mask → forward → restore.
async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // Streaming (SSE) round-trip is milestone M3.
    if body.get("stream").and_then(Value::as_bool) == Some(true) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "streaming responses are not supported yet (milestone M3)",
            "unsupported",
        );
    }

    let mut ctx = RequestContext::new();

    let mut request = ProxyRequest { body };
    for stage in state.stages.iter() {
        stage.on_request(&mut request, &mut ctx);
        if ctx.block.is_some() {
            break;
        }
    }

    // Fail closed: a stage refused to mask this request → reject, don't forward.
    if let Some(reason) = ctx.block {
        tracing::warn!(reason = %reason, "request blocked (fail-closed)");
        return error_response(StatusCode::BAD_REQUEST, &reason, "blocked");
    }

    let client_auth = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let (status, upstream_headers, upstream_body) = match state
        .upstream
        .forward_chat_completions(&request.body, client_auth)
        .await
    {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!(error = %err, "upstream forwarding failed");
            return error_response(StatusCode::BAD_GATEWAY, &err.to_string(), "proxy_error");
        }
    };

    let mut response = ProxyResponse { body: upstream_body };
    for stage in state.stages.iter().rev() {
        stage.on_response(&mut response, &mut ctx);
    }

    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let forwarded = forwardable_headers(&upstream_headers);
    (code, forwarded, Json(response.body)).into_response()
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
    use super::build_detector;

    #[test]
    fn required_ner_is_fatal_when_absent() {
        // M2-R1: requiring a NER that can't be present (no `onnx` feature, or —
        // with the feature — no model configured in the test env) is fatal.
        assert!(build_detector(true).is_err());
        // Not requiring it always yields a structured-only detector.
        assert!(build_detector(false).is_ok());
    }
}

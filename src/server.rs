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
    /// Build the state from config: the upstream client and the default
    /// pipeline (currently just the privacy stage over the structured
    /// recognizers).
    pub fn new(config: &Config) -> Self {
        let upstream = Upstream::new(
            config.upstream_base_url.clone(),
            config.upstream_api_key.clone(),
        );
        let stages: Vec<Box<dyn Stage>> = vec![Box::new(PrivacyStage::new(Box::new(
            StructuredRecognizers::new(),
        )))];
        Self {
            upstream: Arc::new(upstream),
            stages: Arc::new(stages),
            max_body_bytes: config.max_body_bytes,
        }
    }
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
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    let app = build_router(AppState::new(&config));
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

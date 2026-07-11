//! HTTP server: axum router and request handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
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
        }
    }
}

/// Build the router. Exposed so integration tests can serve it on an ephemeral
/// port without going through [`run`].
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(TraceLayer::new_for_http())
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

/// `POST /v1/chat/completions`: mask → forward → restore.
async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // Streaming (SSE) round-trip is milestone M3.
    if body.get("stream").and_then(Value::as_bool) == Some(true) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "streaming responses are not supported yet (milestone M3)",
                    "type": "unsupported"
                }
            })),
        )
            .into_response();
    }

    let mut ctx = RequestContext::new();

    let mut request = ProxyRequest { body };
    for stage in state.stages.iter() {
        stage.on_request(&mut request, &mut ctx);
    }

    let client_auth = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
    let (status, upstream_body) = match state
        .upstream
        .forward_chat_completions(&request.body, client_auth)
        .await
    {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!(error = %err, "upstream forwarding failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": { "message": err.to_string(), "type": "proxy_error" }
                })),
            )
                .into_response();
        }
    };

    let mut response = ProxyResponse { body: upstream_body };
    for stage in state.stages.iter().rev() {
        stage.on_response(&mut response, &mut ctx);
    }

    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(response.body)).into_response()
}

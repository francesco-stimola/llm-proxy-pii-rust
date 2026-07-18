//! Log-safety regression test (M2.6, DBG-01 hardening).
//!
//! A privacy proxy must never write raw PII to a log line. Two guarantees are
//! design rules that were previously only inspection-verified:
//!   1. the `trace!` masked-upstream-body log shows **placeholders**, not values;
//!   2. the final **de-masked** client output (real values) is **never** logged.
//!
//! This drives a real PII request through the proxy while capturing *this crate's*
//! logs at `trace`, and asserts the placeholder is present and the raw value is
//! absent — so a future refactor can't silently turn logging into a leak.
//!
//! Own test file (single test) so it can own the process-global tracing subscriber
//! and capture events emitted from the spawned server task.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

use llm_proxy_pii_rust::config::{Config, DEFAULT_MAX_BODY_BYTES};
use llm_proxy_pii_rust::server::{build_router, AppState};

type Buf = Arc<Mutex<Vec<u8>>>;

/// A `MakeWriter` that appends every formatted log line into a shared buffer.
#[derive(Clone)]
struct BufMaker(Buf);
struct BufSink(Buf);

impl std::io::Write for BufSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> MakeWriter<'a> for BufMaker {
    type Writer = BufSink;
    fn make_writer(&'a self) -> Self::Writer {
        BufSink(self.0.clone())
    }
}

/// Mock upstream that echoes the (masked) last user message back — so the proxy
/// de-masks it and the client gets the raw value (which must NOT be logged).
async fn spawn_mock_upstream() -> SocketAddr {
    async fn handler(Json(body): Json<Value>) -> Json<Value> {
        let last = body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .rfind(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string();
        Json(json!({
            "choices": [{ "message": { "role": "assistant", "content": format!("You said: {last}") } }]
        }))
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn spawn_proxy(upstream: SocketAddr) -> SocketAddr {
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: None,
        max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        provider: "openai".to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_messages_path: "/v1/messages".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: Vec::new(),
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask: false,
        pii_cache_entries: 0,
    };
    let app = build_router(AppState::new(&config).await.expect("app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test]
async fn crate_logs_carry_placeholders_never_raw_pii() {
    let buf: Buf = Arc::new(Mutex::new(Vec::new()));
    // Capture only THIS crate's events at trace (so the masked-body `trace!` fires);
    // the test client's own hyper/reqwest logs are excluded, not the subject here.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("llm_proxy_pii_rust=trace"))
        .with_writer(BufMaker(buf.clone()))
        .with_ansi(false)
        .init();

    let proxy = spawn_proxy(spawn_mock_upstream().await).await;

    let raw_email = "alice@secret.example";
    let reply = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({
            "model": "gpt-x",
            "messages": [{ "role": "user", "content": format!("write to {raw_email} please") }]
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();

    // Sanity: the client really did get the de-masked value back (so if it were
    // ever logged, the assertion below would catch it).
    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(
        content.contains(raw_email),
        "expected de-masked value in the reply: {content}"
    );

    let logs = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("[EMAIL_1]"),
        "the trace! masked-body log should show the placeholder; captured logs:\n{logs}"
    );
    assert!(
        !logs.contains(raw_email),
        "raw PII must NEVER appear in a crate log line (neither the request value nor the \
         de-masked response); captured logs:\n{logs}"
    );
}

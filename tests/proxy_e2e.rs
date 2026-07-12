//! End-to-end tests: a real client → the proxy → a mock upstream provider
//! (E2E-01 / E2E-03 in `docs/TESTING.md`).
//!
//! The mock upstream echoes back, under a non-standard `upstream_received` key,
//! exactly the (masked) body it was sent — that field is outside the proxy's
//! restore path, so the test can inspect what the provider actually saw. The
//! mock also replies by quoting the masked last user message, which the proxy
//! *does* de-mask, so the test can confirm the client gets the real values back.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{Json, Router, http::HeaderMap, response::IntoResponse, routing::post};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use llm_proxy_pii_rust::config::Config;
use llm_proxy_pii_rust::server::{AppState, build_router};

async fn mock_upstream_handler(Json(body): Json<Value>) -> Json<Value> {
    let last_user = body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["role"] == "user")
        .next_back()
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();

    Json(json!({
        "choices": [{
            "message": { "role": "assistant", "content": format!("You said: {last_user}") }
        }],
        "upstream_received": body
    }))
}

/// Spawn the mock upstream on an ephemeral port; return its address.
async fn spawn_mock_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_upstream_handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Spawn the proxy pointing at `upstream`; return its address.
async fn spawn_proxy(upstream: SocketAddr) -> SocketAddr {
    spawn_proxy_cfg(upstream, false).await
}

/// Spawn the proxy with an explicit `debug_skip_demask` (M2.6). The flag is set on
/// `Config` (not process env) so it stays isolated to this proxy instance.
async fn spawn_proxy_cfg(upstream: SocketAddr, debug_skip_demask: bool) -> SocketAddr {
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: None,
        max_body_bytes: llm_proxy_pii_rust::config::DEFAULT_MAX_BODY_BYTES,
        provider: "openai".to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: Vec::new(),
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask,
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// POST a chat-completions request through the proxy and return the JSON reply.
async fn chat(proxy: SocketAddr, request: Value) -> Value {
    reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn e2e01_multi_pii_roundtrip() {
    let proxy = spawn_proxy(spawn_mock_upstream().await).await;

    let reply = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [{
                "role": "user",
                "content": "Mail bob@test.com, phone 555-111-2222, IBAN IT60X0542811101000000123456"
            }]
        }),
    )
    .await;

    // The upstream must have seen masked values and the augmentation system msg.
    let seen = &reply["upstream_received"];
    let seen_text = seen.to_string();
    assert!(!seen_text.contains("bob@test.com"), "leaked email upstream");
    assert!(!seen_text.contains("555-111-2222"), "leaked phone upstream");
    assert!(
        !seen_text.contains("IT60X0542811101000000123456"),
        "leaked IBAN upstream"
    );
    assert_eq!(seen["messages"][0]["role"], "system");
    assert!(seen_text.contains("[EMAIL_1]"));
    assert!(seen_text.contains("[PHONE_1]"));
    assert!(seen_text.contains("[IBAN_1]"));

    // The client must get the real values back.
    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("bob@test.com"), "got: {content}");
    assert!(content.contains("555-111-2222"), "got: {content}");
    assert!(
        content.contains("IT60X0542811101000000123456"),
        "got: {content}"
    );
}

#[tokio::test]
async fn e2e_debug_skip_demask_returns_placeholders_to_client() {
    // M2.6: with PII_DEBUG_SKIP_DEMASK on, the client sees placeholders instead of
    // the real values — while the upstream still received only masked data (the
    // request-side masking always runs, so the fail-closed posture is intact).
    let proxy = spawn_proxy_cfg(spawn_mock_upstream().await, true).await;

    let reply = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [{ "role": "user", "content": "Mail bob@test.com please" }]
        }),
    )
    .await;

    // Upstream saw the masked value, never the raw email.
    let seen_text = reply["upstream_received"].to_string();
    assert!(!seen_text.contains("bob@test.com"), "leaked email upstream");
    assert!(seen_text.contains("[EMAIL_1]"));

    // The client gets the placeholder back, NOT the real value (de-mask skipped).
    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("[EMAIL_1]"), "expected placeholder, got: {content}");
    assert!(!content.contains("bob@test.com"), "value leaked to client: {content}");
}

#[tokio::test]
async fn e2e03_secret_and_email_masked_before_upstream() {
    let proxy = spawn_proxy(spawn_mock_upstream().await).await;

    let secret = "sk-ant-api01-test0000000000000000000000000000000000000000000000000";
    let reply = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [{
                "role": "user",
                "content": format!("Deploy with APP_SECRET={secret} and notify ops@corp.com")
            }]
        }),
    )
    .await;

    let seen_text = reply["upstream_received"].to_string();
    assert!(!seen_text.contains(secret), "secret leaked upstream");
    assert!(!seen_text.contains("ops@corp.com"), "email leaked upstream");
    assert!(seen_text.contains("[SECRET_1]"));
    assert!(seen_text.contains("[EMAIL_1]"));

    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains(secret), "secret not restored: {content}");
    assert!(content.contains("ops@corp.com"), "email not restored: {content}");
}

/// Mock upstream that sets one allowlisted header and one that must be dropped.
async fn mock_upstream_with_headers(Json(_body): Json<Value>) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert("x-ratelimit-remaining", "42".parse().unwrap());
    headers.insert("x-internal-secret", "shhh".parse().unwrap());
    (
        headers,
        Json(json!({
            "choices": [{ "message": { "role": "assistant", "content": "ok" } }]
        })),
    )
}

async fn spawn_mock_upstream_with_headers() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_upstream_with_headers));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test]
async fn e2e_forwards_safe_response_headers_only() {
    let proxy = spawn_proxy(spawn_mock_upstream_with_headers().await).await;

    let resp = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({ "messages": [{ "role": "user", "content": "hi" }] }))
        .send()
        .await
        .unwrap();

    // Allowlisted informational header is forwarded…
    assert_eq!(
        resp.headers().get("x-ratelimit-remaining").map(|v| v.to_str().unwrap()),
        Some("42")
    );
    // …but an arbitrary upstream header is not.
    assert!(resp.headers().get("x-internal-secret").is_none());
}

#[tokio::test]
async fn e2e_unproxied_endpoint_returns_404() {
    let proxy = spawn_proxy(spawn_mock_upstream().await).await;
    let resp = reqwest::Client::new()
        .get(format!("http://{proxy}/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn e2e_fail_closed_request_returns_400_and_is_not_forwarded() {
    let proxy = spawn_proxy(spawn_mock_upstream().await).await;
    let resp = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({ "messages": [{ "role": "user", "content": { "weird": "x" } }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "blocked");
}

/// Mock upstream that streams the (masked) last user message back as SSE, split
/// into small fragments so a placeholder like `[EMAIL_1]` lands across events —
/// the proxy's hold-back buffer must still restore it. Captures the received body
/// so the test can assert the upstream saw only masked values.
async fn spawn_sse_mock() -> (SocketAddr, Arc<Mutex<String>>) {
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_for_handler = seen.clone();
    let handler = move |Json(body): Json<Value>| {
        let seen = seen_for_handler.clone();
        async move {
            *seen.lock().unwrap() = body.to_string();
            let last = body["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| m["role"] == "user")
                .next_back()
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_string();

            // Emit `last` in 4-char fragments, so `[EMAIL_1]` splits across events.
            let chars: Vec<char> = last.chars().collect();
            let mut sse = String::new();
            for chunk in chars.chunks(4) {
                let frag: String = chunk.iter().collect();
                let ev = json!({ "choices": [ { "index": 0, "delta": { "content": frag } } ] });
                sse.push_str(&format!("data: {ev}\n\n"));
            }
            sse.push_str("data: [DONE]\n\n");
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                sse,
            )
                .into_response()
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, seen)
}

/// Sum the `delta.content` across the SSE `data:` events in a raw stream body.
fn sse_content(raw: &str) -> String {
    let mut acc = String::new();
    for line in raw.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(c) = v.pointer("/choices/0/delta/content").and_then(Value::as_str) {
                    acc.push_str(c);
                }
            }
        }
    }
    acc
}

#[tokio::test]
async fn e2e_streaming_deanonymizes_split_placeholder() {
    let (upstream, seen) = spawn_sse_mock().await;
    let proxy = spawn_proxy(upstream).await;

    let raw = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({
            "model": "gpt-x",
            "stream": true,
            "messages": [{ "role": "user", "content": "please email bob@test.com now" }]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // Upstream saw the masked value, never the raw email.
    let upstream_saw = seen.lock().unwrap().clone();
    assert!(!upstream_saw.contains("bob@test.com"), "leaked email upstream");
    assert!(upstream_saw.contains("[EMAIL_1]"), "expected placeholder upstream");

    // The client's reassembled stream carries the real value, no placeholder —
    // even though `[EMAIL_1]` was split across SSE events.
    let content = sse_content(&raw);
    assert_eq!(content, "please email bob@test.com now", "got: {content}");
    assert!(!raw.contains("[EMAIL_1]"), "placeholder leaked to client: {raw}");
    assert!(raw.contains("[DONE]"), "terminator preserved");
}

/// Mock upstream that answers *any* request (even `stream:true`) with a non-2xx
/// `application/json` error — a provider rate-limit / auth error.
async fn spawn_json_error_mock() -> SocketAddr {
    async fn handler() -> impl IntoResponse {
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": { "message": "rate limited", "type": "rate_limit" } })),
        )
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test]
async fn e2e_streaming_non_sse_error_falls_back_to_json() {
    // M3-R1: a `stream:true` request the upstream answers with a non-SSE JSON error
    // must reach the client as that JSON error (real status + content-type), not be
    // force-wrapped as an event-stream.
    let proxy = spawn_proxy(spawn_json_error_mock().await).await;

    let resp = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({
            "model": "gpt-x",
            "stream": true,
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(ct.contains("application/json"), "got content-type: {ct}");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "rate_limit");
}

#[tokio::test]
async fn e2e_clean_request_passes_through_unchanged() {
    let proxy = spawn_proxy(spawn_mock_upstream().await).await;

    let reply = chat(
        proxy,
        json!({ "messages": [{ "role": "user", "content": "just say hi" }] }),
    )
    .await;

    // No PII → no augmentation system message injected upstream.
    let seen = &reply["upstream_received"];
    assert_eq!(seen["messages"].as_array().unwrap().len(), 1);
    assert_eq!(seen["messages"][0]["role"], "user");
    assert_eq!(
        reply["choices"][0]["message"]["content"],
        "You said: just say hi"
    );
}

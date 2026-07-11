//! End-to-end tests: a real client → the proxy → a mock upstream provider
//! (E2E-01 / E2E-03 in `docs/TESTING.md`).
//!
//! The mock upstream echoes back, under a non-standard `upstream_received` key,
//! exactly the (masked) body it was sent — that field is outside the proxy's
//! restore path, so the test can inspect what the provider actually saw. The
//! mock also replies by quoting the masked last user message, which the proxy
//! *does* de-mask, so the test can confirm the client gets the real values back.

use std::net::SocketAddr;

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
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: None,
        max_body_bytes: llm_proxy_pii_rust::config::DEFAULT_MAX_BODY_BYTES,
    };
    let app = build_router(AppState::new(&config));
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

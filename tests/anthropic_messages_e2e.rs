//! End-to-end tests for the native Anthropic `/v1/messages` route (M6): a real
//! client → the proxy → a mock **native** Anthropic upstream.
//!
//! The mock echoes the (masked) body it received under a non-standard
//! `upstream_received` key and the auth headers under `upstream_auth` — both
//! outside the proxy's `content[]` restore path, so a test can inspect what the
//! provider actually saw. Its assistant reply quotes the masked last user message
//! in a `text` block, which the proxy *does* de-mask, so a test can confirm the
//! client gets real values back.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{http::HeaderMap, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use llm_proxy_pii_rust::config::{Config, DEFAULT_MAX_BODY_BYTES};
use llm_proxy_pii_rust::server::{build_router, AppState};

/// The content of the last `user` message, as a plain string (the tests send
/// string content, the common Claude Code shape).
fn last_user_text(body: &Value) -> String {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .rfind(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// The auth-relevant request headers, echoed so a test can assert the passthrough.
fn echoed_auth(headers: &HeaderMap) -> Value {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    };
    json!({
        "authorization": get("authorization"),
        "x-api-key": get("x-api-key"),
        "anthropic-version": get("anthropic-version"),
    })
}

/// A buffered native Messages mock: reply with a `content[]` text block quoting
/// the masked last user message, plus the received body and auth for inspection.
async fn mock_messages_handler(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    let last = last_user_text(&body);
    Json(json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [ { "type": "text", "text": format!("You said: {last}") } ],
        "stop_reason": "end_turn",
        "upstream_received": body,
        "upstream_auth": echoed_auth(&headers),
    }))
}

async fn spawn_mock_messages() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/messages", post(mock_messages_handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn anthropic_config(upstream: SocketAddr, api_key: Option<String>) -> Config {
    Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: api_key,
        max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        provider: "anthropic".to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_messages_path: "/v1/messages".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: vec![
            "anthropic-version".to_string(),
            "anthropic-beta".to_string(),
        ],
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask: false,
    }
}

/// Spawn the proxy for the `anthropic` provider pointing at `upstream`.
async fn spawn_proxy(upstream: SocketAddr, api_key: Option<String>) -> SocketAddr {
    spawn_proxy_with_config(anthropic_config(upstream, api_key)).await
}

async fn spawn_proxy_with_config(config: Config) -> SocketAddr {
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// POST a native Messages request through the proxy (no auth) → JSON reply.
async fn messages(proxy: SocketAddr, request: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{proxy}/v1/messages"))
        .json(&request)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn messages_buffered_roundtrip_masks_upstream_and_restores_to_client() {
    // A configured proxy key is injected as `x-api-key`, so the request forwards.
    let proxy = spawn_proxy(
        spawn_mock_messages().await,
        Some("sk-ant-api-k".to_string()),
    )
    .await;

    let reply: Value = messages(
        proxy,
        json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 64,
            "system": "you are helpful",
            "messages": [{
                "role": "user",
                "content": "Mail bob@test.com, phone 555-111-2222, IBAN IT60X0542811101000000123456"
            }]
        }),
    )
    .await
    .json()
    .await
    .unwrap();

    // Non-vacuity: the mock actually replied (not a 401/400 whose body has no
    // `upstream_received`, which would let every `!contains` assert pass on "null").
    assert!(
        reply.get("upstream_received").is_some_and(|v| !v.is_null()),
        "expected the mock's echo, got: {reply}"
    );

    // Upstream saw masked values, never the raw PII.
    let seen = reply["upstream_received"].to_string();
    assert!(
        !seen.contains("bob@test.com"),
        "leaked email upstream: {seen}"
    );
    assert!(!seen.contains("555-111-2222"), "leaked phone upstream");
    assert!(
        !seen.contains("IT60X0542811101000000123456"),
        "leaked IBAN upstream"
    );
    assert!(
        seen.contains("[EMAIL_1]") && seen.contains("[PHONE_1]") && seen.contains("[IBAN_1]"),
        "expected placeholders in: {seen}"
    );

    // The augmentation was merged into the top-level `system` (native, in place).
    let system = reply["upstream_received"]["system"].as_str().unwrap();
    assert!(
        system.starts_with("you are helpful"),
        "system preserved: {system}"
    );
    assert!(
        system.contains("placeholder"),
        "augmentation injected: {system}"
    );

    // The client got the real values back (de-masked `content[].text`).
    let text = reply["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("bob@test.com"), "email not restored: {text}");
    assert!(text.contains("555-111-2222"), "phone not restored: {text}");
    assert!(
        text.contains("IT60X0542811101000000123456"),
        "IBAN not restored: {text}"
    );
}

#[tokio::test]
async fn messages_masks_system_content_blocks_and_tool_definitions() {
    let proxy = spawn_proxy(
        spawn_mock_messages().await,
        Some("sk-ant-api-k".to_string()),
    )
    .await;

    let reply: Value = messages(
        proxy,
        json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 64,
            "system": [ { "type": "text", "text": "escalate to sys@corp.com" } ],
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "text a@b.com" },
                    { "type": "tool_use", "id": "t1", "name": "f", "input": { "to": "c@d.com" } },
                    { "type": "tool_result", "tool_use_id": "t1", "content": "row e@f.com" }
                ]
            }],
            "tools": [
                { "name": "lookup", "description": "for g@h.com",
                  "input_schema": { "type": "object",
                    "properties": { "x": { "type": "string", "description": "id i@j.com" } } } }
            ]
        }),
    )
    .await
    .json()
    .await
    .unwrap();

    assert!(
        reply.get("upstream_received").is_some_and(|v| !v.is_null()),
        "expected the mock's echo, got: {reply}"
    );
    let seen = reply["upstream_received"].to_string();
    assert!(seen.contains("[EMAIL_1]"), "nothing masked: {seen}");
    for raw in [
        "sys@corp.com",
        "a@b.com",
        "c@d.com",
        "e@f.com",
        "g@h.com",
        "i@j.com",
    ] {
        assert!(!seen.contains(raw), "leaked {raw} upstream: {seen}");
    }
}

#[tokio::test]
async fn messages_unknown_block_type_fails_closed_400() {
    let proxy = spawn_proxy(spawn_mock_messages().await, None).await;
    let resp = messages(
        proxy,
        json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 64,
            "messages": [{ "role": "user", "content": [ { "type": "quantum_block", "x": 1 } ] }]
        }),
    )
    .await;
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "blocked");
}

#[tokio::test]
async fn messages_route_is_404_when_provider_is_not_anthropic() {
    // The route is registered only for the `anthropic` provider; other providers
    // 404 a native body rather than mis-routing it.
    let mut config = anthropic_config(spawn_mock_messages().await, None);
    config.provider = "openai".to_string();
    let proxy = spawn_proxy_with_config(config).await;

    let resp = messages(
        proxy,
        json!({ "model": "x", "max_tokens": 8, "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn messages_client_bearer_is_forwarded_verbatim_never_as_x_api_key() {
    // The Claude Code OAuth token rides in `Authorization: Bearer` and must reach
    // the upstream verbatim there — never copied into `x-api-key` (Anthropic 401).
    let proxy = spawn_proxy(spawn_mock_messages().await, Some("proxy-key".to_string())).await;

    let reply: Value = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/messages"))
        .header("authorization", "Bearer sk-ant-oat01-CLIENT")
        .json(&json!({
            "model": "claude-3-5-sonnet", "max_tokens": 8,
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let auth = &reply["upstream_auth"];
    assert_eq!(auth["authorization"], "Bearer sk-ant-oat01-CLIENT");
    // The client credential wins over the configured key, and the OAuth token is
    // NOT in x-api-key.
    assert!(
        auth["x-api-key"].is_null(),
        "OAuth leaked into x-api-key: {auth}"
    );
}

#[tokio::test]
async fn messages_proxy_key_is_injected_as_x_api_key_when_client_sends_none() {
    let proxy = spawn_proxy(
        spawn_mock_messages().await,
        Some("sk-ant-api-PROXY".to_string()),
    )
    .await;

    let reply: Value = messages(
        proxy,
        json!({ "model": "claude-3-5-sonnet", "max_tokens": 8,
                "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await
    .json()
    .await
    .unwrap();

    let auth = &reply["upstream_auth"];
    assert_eq!(auth["x-api-key"], "sk-ant-api-PROXY");
    assert!(
        auth["authorization"].is_null(),
        "no Authorization expected: {auth}"
    );
    // A default anthropic-version is injected when the client omitted it.
    assert_eq!(auth["anthropic-version"], "2023-06-01");
}

#[tokio::test]
async fn messages_no_credential_returns_401_without_forwarding() {
    // No client auth and no configured proxy key → 401, nothing forwarded.
    let proxy = spawn_proxy(spawn_mock_messages().await, None).await;
    let resp = messages(
        proxy,
        json!({ "model": "claude-3-5-sonnet", "max_tokens": 8,
                "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await;
    assert_eq!(resp.status(), 401);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "unauthorized");
}

#[tokio::test]
async fn messages_client_anthropic_version_is_forwarded_not_overridden() {
    let proxy = spawn_proxy(spawn_mock_messages().await, Some("k".to_string())).await;
    let reply: Value = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/messages"))
        .header("anthropic-version", "2024-10-22")
        .json(&json!({
            "model": "claude-3-5-sonnet", "max_tokens": 8,
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reply["upstream_auth"]["anthropic-version"], "2024-10-22");
}

// ── Streaming (SSE) ─────────────────────────────────────────────────────────

/// A streaming native Messages mock: emit the masked last-user text back as
/// Anthropic `content_block_delta` `text_delta` events in 4-char fragments, so a
/// placeholder like `[EMAIL_1]` lands split across events. Captures the received
/// body so the test can assert the upstream saw only masked values.
async fn spawn_mock_messages_sse() -> (SocketAddr, Arc<Mutex<String>>) {
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_for_handler = seen.clone();
    let handler = move |Json(body): Json<Value>| {
        let seen = seen_for_handler.clone();
        async move {
            *seen.lock().unwrap() = body.to_string();
            let last = last_user_text(&body);

            let mut sse = String::new();
            sse.push_str("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\"}}\n\n");
            sse.push_str("event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n");
            let chars: Vec<char> = last.chars().collect();
            for chunk in chars.chunks(4) {
                let frag: String = chunk.iter().collect();
                let ev = json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": frag }
                });
                sse.push_str(&format!("event: content_block_delta\ndata: {ev}\n\n"));
            }
            sse.push_str("event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n");
            sse.push_str("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n");
            sse.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                sse,
            )
                .into_response()
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/messages", post(handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, seen)
}

/// Sum `content_block_delta` → `text_delta.text` across an SSE response body.
fn sse_text(raw: &str) -> String {
    let mut acc = String::new();
    for line in raw.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                if v.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                    if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str) {
                        acc.push_str(t);
                    }
                }
            }
        }
    }
    acc
}

#[tokio::test]
async fn messages_streaming_deanonymizes_split_placeholder() {
    let (upstream, seen) = spawn_mock_messages_sse().await;
    let proxy = spawn_proxy(upstream, Some("k".to_string())).await;

    let raw = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/messages"))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 64,
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
    assert!(
        !upstream_saw.contains("bob@test.com"),
        "leaked email upstream"
    );
    assert!(
        upstream_saw.contains("[EMAIL_1]"),
        "expected placeholder upstream"
    );

    // The client's reassembled stream carries the real value, no placeholder —
    // even though `[EMAIL_1]` was split across `text_delta` events.
    assert_eq!(
        sse_text(&raw),
        "please email bob@test.com now",
        "got: {raw}"
    );
    assert!(
        !raw.contains("[EMAIL_1]"),
        "placeholder leaked to client: {raw}"
    );
    assert!(raw.contains("message_stop"), "stream terminator preserved");
}

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

use axum::{http::HeaderMap, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use llm_proxy_pii_rust::config::Config;
use llm_proxy_pii_rust::server::{build_router, AppState};

async fn mock_upstream_handler(Json(body): Json<Value>) -> Json<Value> {
    let last_user = body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .rfind(|m| m["role"] == "user")
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
        upstream_messages_path: "/v1/messages".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: Vec::new(),
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask,
        pii_cache_entries: 0,
        pii_max_phone_validations:
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Spawn the proxy with the S3 detection cache ON (`cache_entries` entries). Exercises the cache
/// in the real pipeline: a repeated large field must still mask on every request (S3 soundness).
async fn spawn_proxy_cached(upstream: SocketAddr, cache_entries: usize) -> SocketAddr {
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: None,
        max_body_bytes: llm_proxy_pii_rust::config::DEFAULT_MAX_BODY_BYTES,
        provider: "openai".to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_messages_path: "/v1/messages".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: Vec::new(),
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask: false,
        pii_cache_entries: cache_entries,
        pii_max_phone_validations:
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Spawn the proxy with the **shipped domestic-phone region set** and an explicit per-request
/// validation allowance (M10-R28).
///
/// Two departures from [`spawn_proxy`], both deliberate. The locales are `default_locales()` rather
/// than this file's usual `it,us`, because a refusal driven by one region is not the tier the product
/// ships. And the allowance is a *test* number: crossing the shipped 500,000 costs ~25 s unoptimized,
/// and what E2E-05 asserts — that the refusal reaches `error.message` intact — is not a claim about
/// the size of the number. DOS-BUD pins the number itself, on `--release`.
async fn spawn_proxy_budgeted(upstream: SocketAddr, validation_units: usize) -> SocketAddr {
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: None,
        max_body_bytes: llm_proxy_pii_rust::config::DEFAULT_MAX_BODY_BYTES,
        provider: "openai".to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_messages_path: "/v1/messages".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: Vec::new(),
        pii_locales: llm_proxy_pii_rust::config::default_locales(),
        debug_skip_demask: false,
        pii_cache_entries: 0,
        pii_max_phone_validations: validation_units,
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Spawn the proxy with a specific provider preset (path kept OpenAI-compatible so
/// the shared mock serves it). Used to prove masking is provider-independent (M4).
async fn spawn_proxy_provider(upstream: SocketAddr, provider: &str) -> SocketAddr {
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: format!("http://{upstream}"),
        upstream_api_key: None,
        max_body_bytes: llm_proxy_pii_rust::config::DEFAULT_MAX_BODY_BYTES,
        provider: provider.to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_messages_path: "/v1/messages".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: Vec::new(),
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask: false,
        pii_cache_entries: 0,
        pii_max_phone_validations:
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
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
    assert!(
        content.contains("[EMAIL_1]"),
        "expected placeholder, got: {content}"
    );
    assert!(
        !content.contains("bob@test.com"),
        "value leaked to client: {content}"
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
    assert!(
        content.contains("ops@corp.com"),
        "email not restored: {content}"
    );
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
        resp.headers()
            .get("x-ratelimit-remaining")
            .map(|v| v.to_str().unwrap()),
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
                .rfind(|m| m["role"] == "user")
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
                if let Some(c) = v
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                {
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
    assert!(
        !upstream_saw.contains("bob@test.com"),
        "leaked email upstream"
    );
    assert!(
        upstream_saw.contains("[EMAIL_1]"),
        "expected placeholder upstream"
    );

    // The client's reassembled stream carries the real value, no placeholder —
    // even though `[EMAIL_1]` was split across SSE events.
    let content = sse_content(&raw);
    assert_eq!(content, "please email bob@test.com now", "got: {content}");
    assert!(
        !raw.contains("[EMAIL_1]"),
        "placeholder leaked to client: {raw}"
    );
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
async fn e2e_masking_is_provider_agnostic() {
    // M4/M5 (LOC-11): the masker walks the OpenAI-shaped JSON; provider presets only
    // affect routing (path / headers), never masking — so the masked body reaching
    // the upstream is identical regardless of the configured provider. Covers all
    // three mock-upstream shapes M5 asks for: OpenAI, Copilot, Anthropic.
    let upstream = spawn_mock_upstream().await;
    let request = json!({
        "model": "x",
        "messages": [{
            "role": "user",
            "content": "mail bob@test.com and IBAN IT60X0542811101000000123456"
        }]
    });

    let via_openai = chat(
        spawn_proxy_provider(upstream, "openai").await,
        request.clone(),
    )
    .await;
    let via_copilot = chat(
        spawn_proxy_provider(upstream, "copilot").await,
        request.clone(),
    )
    .await;
    let via_anthropic = chat(spawn_proxy_provider(upstream, "anthropic").await, request).await;

    assert_eq!(
        via_openai["upstream_received"], via_copilot["upstream_received"],
        "masking must be provider-independent (copilot)"
    );
    assert_eq!(
        via_openai["upstream_received"], via_anthropic["upstream_received"],
        "masking must be provider-independent (anthropic)"
    );
    let seen = via_openai["upstream_received"].to_string();
    assert!(seen.contains("[EMAIL_1]"));
    assert!(seen.contains("[IBAN_1]"));
    assert!(!seen.contains("bob@test.com"));
}

/// Mock upstream that echoes back, in the assistant reply, the content of the
/// **last message with the given role** (instead of always the last `user`
/// message) — used to drive PII sitting in `tool` result messages (E2E-02/04),
/// while still exposing the exact (masked) body it received via
/// `upstream_received`.
async fn spawn_mock_upstream_echo_role(role: &'static str) -> SocketAddr {
    let handler = move |Json(body): Json<Value>| async move {
        let last = body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .rfind(|m| m["role"] == role)
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string();

        Json(json!({
            "choices": [{
                "message": { "role": "assistant", "content": format!("You said: {last}") }
            }],
            "upstream_received": body
        }))
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test]
async fn e2e02_csv_tool_result_pii_is_masked_and_restored() {
    // TC-02 / E2E-02: PII inside a CSV `tool_result` is masked upstream and
    // restored to the client.
    let proxy = spawn_proxy(spawn_mock_upstream_echo_role("tool").await).await;
    let csv = "name,email,phone,iban\nBob,bob@test.com,555-111-2222,IT60X0542811101000000123456";

    let reply = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [
                { "role": "user", "content": "here is the contacts export" },
                { "role": "tool", "tool_call_id": "c1", "content": csv }
            ]
        }),
    )
    .await;

    let seen_text = reply["upstream_received"].to_string();
    assert!(!seen_text.contains("bob@test.com"), "leaked email upstream");
    assert!(!seen_text.contains("555-111-2222"), "leaked phone upstream");
    assert!(
        !seen_text.contains("IT60X0542811101000000123456"),
        "leaked IBAN upstream"
    );
    assert!(seen_text.contains("[EMAIL_1]"));
    assert!(seen_text.contains("[PHONE_1]"));
    assert!(seen_text.contains("[IBAN_1]"));

    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("bob@test.com"), "got: {content}");
    assert!(content.contains("555-111-2222"), "got: {content}");
    assert!(
        content.contains("IT60X0542811101000000123456"),
        "got: {content}"
    );
}

#[tokio::test]
async fn e2e04_db_query_result_all_categories_masked_and_restored() {
    // TC-04 / E2E-04: a `SELECT … FROM DUAL`-style tabular result carrying every
    // structured category in one field — all masked upstream, all restored to
    // the client.
    let proxy = spawn_proxy(spawn_mock_upstream_echo_role("tool").await).await;
    let secret = "sk-ant-api01-test0000000000000000000000000000000000000000000000000";
    let db_result = format!(
        "SELECT * FROM DUAL: email=bob@test.com, phone=555-111-2222, ssn=123-45-6789, \
         card=4111111111111111, iban=IT60X0542811101000000123456, secret={secret}"
    );

    let reply = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [
                { "role": "user", "content": "run the customer lookup query" },
                { "role": "tool", "tool_call_id": "c1", "content": db_result }
            ]
        }),
    )
    .await;

    let seen_text = reply["upstream_received"].to_string();
    for raw in [
        "bob@test.com",
        "555-111-2222",
        "123-45-6789",
        "4111111111111111",
        "IT60X0542811101000000123456",
        secret,
    ] {
        assert!(
            !seen_text.contains(raw),
            "{raw} leaked upstream: {seen_text}"
        );
    }
    for placeholder in [
        "[EMAIL_1]",
        "[PHONE_1]",
        "[SSN_1]",
        "[CARD_1]",
        "[IBAN_1]",
        "[SECRET_1]",
    ] {
        assert!(
            seen_text.contains(placeholder),
            "expected {placeholder} in {seen_text}"
        );
    }

    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    for raw in [
        "bob@test.com",
        "555-111-2222",
        "123-45-6789",
        "4111111111111111",
        "IT60X0542811101000000123456",
        secret,
    ] {
        assert!(content.contains(raw), "{raw} not restored, got: {content}");
    }
}

/// Mock upstream that ignores the request content and always answers with an
/// assistant `tool_calls` response whose `arguments` reference a fixed
/// placeholder (`[EMAIL_1]`) — the token a single-email request masks to.
async fn spawn_mock_upstream_tool_call() -> SocketAddr {
    async fn handler(Json(body): Json<Value>) -> Json<Value> {
        Json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "1",
                        "type": "function",
                        "function": { "name": "send_email", "arguments": "{\"to\":\"[EMAIL_1]\"}" }
                    }]
                }
            }],
            "upstream_received": body
        }))
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test]
async fn e2e_tool_call_arguments_round_trip_over_http() {
    // Full HTTP round-trip version of INT-03 (which drives `PrivacyStage`
    // directly): the vault is populated from the masked request, and the
    // response's `tool_calls[].function.arguments` — carrying the placeholder
    // the provider echoed back — is de-anonymized before the client sees it.
    let proxy = spawn_proxy(spawn_mock_upstream_tool_call().await).await;

    let reply = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [{ "role": "user", "content": "email jane@example.com please" }]
        }),
    )
    .await;

    let seen_text = reply["upstream_received"].to_string();
    assert!(!seen_text.contains("jane@example.com"), "leaked upstream");
    assert!(seen_text.contains("[EMAIL_1]"));

    let args = reply["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();
    assert!(args.contains("jane@example.com"), "got: {args}");
    assert!(!args.contains("[EMAIL_1]"), "got: {args}");
}

#[tokio::test]
async fn e2e_multi_turn_determinism_across_stateless_requests() {
    // Each HTTP request gets a fresh `Vault` (the proxy is stateless), so
    // "multi-turn determinism" means: resending the same conversation history —
    // exactly what an OpenAI-style stateless client does every turn — must
    // reassign the **same** placeholder to a repeated value, because it is
    // still the same value at the same first-occurrence position in reading
    // order. Two real HTTP round-trips against the same proxy instance.
    let upstream = spawn_mock_upstream().await;
    let proxy = spawn_proxy(upstream).await;

    let turn1 = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [{ "role": "user", "content": "contact bob@test.com" }]
        }),
    )
    .await;
    let turn1_seen = turn1["upstream_received"].to_string();
    assert!(turn1_seen.contains("[EMAIL_1]"), "got: {turn1_seen}");

    // Turn 2 resends the full (de-masked, client-visible) history plus a new
    // message that repeats bob@test.com and introduces a second value.
    let history_reply = turn1["choices"][0]["message"]["content"]
        .as_str()
        .unwrap()
        .to_string();
    let turn2 = chat(
        proxy,
        json!({
            "model": "gpt-x",
            "messages": [
                { "role": "user", "content": "contact bob@test.com" },
                { "role": "assistant", "content": history_reply },
                { "role": "user", "content": "also cc alice@a.com, and again bob@test.com" }
            ]
        }),
    )
    .await;
    let turn2_seen = turn2["upstream_received"].to_string();

    assert!(
        !turn2_seen.contains("bob@test.com") && !turn2_seen.contains("alice@a.com"),
        "leaked upstream: {turn2_seen}"
    );
    // bob@test.com is still the first distinct value in reading order → still [EMAIL_1].
    assert!(
        turn2_seen.contains("[EMAIL_1]"),
        "the repeated value must keep its token across turns: {turn2_seen}"
    );
    assert!(
        turn2_seen.contains("[EMAIL_2]"),
        "the new value must get the next token: {turn2_seen}"
    );
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

#[tokio::test]
async fn e2e_cache_on_a_repeated_large_field_still_masks_both_times() {
    // S3 (M7.1): with the detection cache ON, a byte-identical large field sent twice must mask on
    // BOTH requests — the second is served from the cache, and a cache hit must never mask *less*
    // than a fresh scan. This is the pipeline-level proof of the soundness argument in `pii::cache`.
    let proxy = spawn_proxy_cached(spawn_mock_upstream().await, 16).await;

    // A field over MIN_CACHEABLE_LEN (256 B) carrying a real email — so it is both cached and
    // PII-bearing. The steady boilerplate is exactly the system-prompt shape S3 exists for.
    let system = format!(
        "You are a helpful assistant. {}Reach the operator at cacheprobe@example.com anytime.",
        "Some steady boilerplate context. ".repeat(12)
    );
    let body = json!({
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": "hi" }
        ]
    });

    for attempt in 1..=2 {
        let reply = chat(proxy, body.clone()).await;
        let sent_system = reply["upstream_received"]["messages"][0]["content"]
            .as_str()
            .expect("the masked system message");
        assert!(
            !sent_system.contains("cacheprobe@example.com"),
            "attempt {attempt}: raw email reached upstream (cache masked less than a fresh scan): {sent_system}"
        );
        assert!(
            sent_system.contains("[EMAIL_1]"),
            "attempt {attempt}: the email must be masked to a placeholder: {sent_system}"
        );
    }
}

/// Mock upstream that records how many requests it was asked to serve, so a test can
/// assert **nothing was forwarded** rather than merely that the client saw a 400.
async fn spawn_counting_mock_upstream() -> (SocketAddr, Arc<Mutex<usize>>) {
    let hits = Arc::new(Mutex::new(0usize));
    let seen = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(_body): Json<Value>| {
            let seen = seen.clone();
            async move {
                *seen.lock().unwrap() += 1;
                Json(json!({ "choices": [] }))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

/// E2E-05 (M10-R27 / M10-R32) — a field that exhausts the validation budget is refused, and the
/// refusal **the client reads** is the actionable one.
///
/// M10-R27 rewrote that message so an agent receiving the 400 can act on it instead of retrying
/// the identical body forever. It was verified by hand, once, at the moment it was written — and
/// the only automated assertions on it live in DOS-06, which reads `err.message` straight off the
/// detector and passes byte-for-byte on the message M10-R27 *replaced* (M10-R32). So the half that
/// carries the whole point — that the string survives `DetectError`'s `Display`, `ctx.block`, and
/// `error_response` into `error.message` — was pinned nowhere at all. Both have moved before.
///
/// Three properties, and (c) is the one that must never be dropped: this is the single string in
/// the codebase deliberately built from request bytes, so it is where the never-log-raw-PII bar is
/// easiest to breach by accident.
#[tokio::test]
async fn e2e05_budget_refusal_reaches_the_client_intact_and_carries_no_input_bytes() {
    /// A **test** allowance, not the shipped 500,000 — see `spawn_proxy_budgeted`. Named because
    /// assertion (c) below has to know it: the refusal may carry this number and the field's length,
    /// and nothing else.
    const VALIDATION_UNITS: usize = 20_000;

    // The same odometer as DOS-06: `(b, c)` enumerated over 81 M combinations, so no candidate
    // recurs and the per-scan memo is inert. A modular-hash generator silently repeats after its
    // period and reports "the budget was never reached" as the product's doing (M10-R20).
    let mut field = String::with_capacity(256 * 1024 + 64);
    let mut i = 0u64;
    while field.len() < 256 * 1024 {
        i += 1;
        field.push_str(&format!(
            "row {:02} {:04} {:04} end ",
            10 + (i / 81_000_000) % 80,
            1000 + i % 9000,
            1000 + (i / 9000) % 9000
        ));
    }

    let (upstream, hits) = spawn_counting_mock_upstream().await;
    let proxy = spawn_proxy_budgeted(upstream, VALIDATION_UNITS).await;
    let resp = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({ "messages": [{ "role": "user", "content": field }] }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "the budget refusal must reach the client as a 400"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "blocked");
    let message = body["error"]["message"]
        .as_str()
        .expect("the 400 body must carry error.message")
        .to_string();

    // (a) it names what happened.
    assert!(
        message.contains("budget"),
        "the refusal must say what was exceeded, got: {message}"
    );
    // (b) it is *actionable* — the half DOS-06 cannot distinguish from the message it replaced.
    //     An agent that retries the identical body gets the identical 400; the message has to say
    //     so, and name the field-shrinking move that does work.
    assert!(
        message.contains("Retrying it unchanged will fail identically"),
        "the refusal must tell its client that a bare retry is futile, got: {message}"
    );
    assert!(
        message.contains("LIMIT"),
        "the refusal must name a concrete way to shrink the field, got: {message}"
    );

    // (c) no byte of the request survives in it. The message is one `format!` over two integers —
    //     the allowance and the field's length — so this is phrased as **"only these appear"**
    //     rather than "nothing forbidden appears" (M10-R40).
    //
    //     **The negative phrasing had a hole exactly where it mattered.** It read
    //     `!field.contains(run) || run.len() < 4`, and that disjunct passes *unconditionally* for
    //     any 1-, 2- or 3-digit run drawn from the body — which is the length a truncation leaks
    //     (M10-R1's orphaned `912`, three digits of a real Portuguese number). An assertion
    //     disabled precisely where the interesting leak would sit is not an assertion (M10-R13).
    //     The exemption existed to avoid false positives from short numbers; stating the allowed
    //     set instead removes the need for it, and cannot be satisfied vacuously.
    assert!(
        !message.contains("row "),
        "the refusal must carry no input-derived text, got: {message}"
    );
    let allowed = [VALIDATION_UNITS.to_string(), field.len().to_string()];
    let digit_runs: Vec<&str> = message
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        !digit_runs.is_empty(),
        "no digits at all in the refusal — this check would pass vacuously: {message}"
    );
    for run in &digit_runs {
        assert!(
            allowed.iter().any(|a| a == run),
            "the refusal carries the digit run {run:?}, which is neither the allowance ({}) nor \
             the field length ({}). Every number in this string must be one the code put there \
             deliberately — a third one is request-derived text in a message that is both logged \
             and returned to the client: {message}",
            allowed[0],
            allowed[1]
        );
    }

    // And the refusal is a refusal: the upstream was never asked.
    assert_eq!(
        *hits.lock().unwrap(),
        0,
        "a blocked request must not be forwarded — the upstream saw it"
    );
}

/// The last `user` message as the upstream actually received it.
///
/// **Why this and not `body.to_string()` (the vacuity trap).** The augmentation instruction the
/// proxy injects *exemplifies* `[TAXID_1]` by design — that is `AUG-01`'s whole subject. So a
/// guard asserting `[TAXID_1]` appears somewhere in the upstream body is satisfied by the
/// injected boilerplate alone, and would stay green with the user's VAT number forwarded in
/// clear. The claim has to be made about the field the PII was in.
fn upstream_last_user(seen: &Value) -> String {
    seen["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .rfind(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// **VAT-18 (M11 Track A) — the new `TaxId` kind over real HTTP, in every shipped rendering.**
///
/// M11's headline feature had **no end-to-end coverage at all**: every VAT guard tested the
/// detector, and review round 1 confirmed the HTTP round trip *by hand*. A capability verified by
/// hand is a capability nothing will notice losing — and the `[TAXID_n]` token is new vocabulary
/// on the wire, which is precisely the kind of thing a serde or restore-path change breaks
/// silently. So this pins what `E2E-01` pins for email/phone/IBAN, for the kind M11 added.
///
/// One value per rendering family, because the resolver and the vault see them differently: the
/// bare Italian form (11 digits, no context — the one that carries the over-mask cost), the VIES
/// prefixed form, a second country's prefixed form, and the format-only NL form whose
/// `Confidence` is `Structural` rather than `Verified`. `00…`-leading P.IVAs are used on purpose:
/// they are the sub-shape the phone tier cannot claim (`VAT-17`), so this guard measures the VAT
/// path rather than the collision.
#[tokio::test]
async fn vat_numbers_round_trip_over_http_in_every_rendering() {
    let proxy = spawn_proxy(spawn_mock_upstream().await).await;

    // (value, why this rendering is here)
    const RENDERINGS: &[&str] = &[
        "00905811006",    // IT bare — ENI, and `00…` so the phone tier cannot claim it
        "IT00159560366",  // IT VIES form — Ferrari
        "DE136695976",    // DE — the German administration's documented vector
        "NL111222333B01", // NL — format-anchored, `Confidence::Structural`
    ];
    let sent = format!(
        "Fattura a {}, P.IVA {}, USt {} e btw {} — grazie.",
        RENDERINGS[1], RENDERINGS[0], RENDERINGS[2], RENDERINGS[3]
    );

    let reply = chat(
        proxy,
        json!({ "model": "gpt-x", "messages": [{ "role": "user", "content": sent }] }),
    )
    .await;

    let seen = &reply["upstream_received"];
    let masked_user = upstream_last_user(seen);

    // Nothing left in clear — asserted on the field the values were in, not on the whole body.
    for value in RENDERINGS {
        assert!(
            !masked_user.contains(value),
            "{value} reached the upstream in clear: {masked_user}"
        );
    }
    // ...and the masked field really does carry placeholders, one per value.
    let placeholders = masked_user.matches("[TAXID_").count();
    assert_eq!(
        placeholders,
        RENDERINGS.len(),
        "expected one [TAXID_n] per rendering in the masked user message, got {placeholders}: \
         {masked_user}"
    );
    // The augmentation instruction is injected, and it is a *separate* field from the one above.
    assert_eq!(seen["messages"][0]["role"], "system");

    // The client gets every value back byte-identically, and no placeholder escapes.
    let content = reply["choices"][0]["message"]["content"].as_str().unwrap();
    for value in RENDERINGS {
        assert!(
            content.contains(value),
            "{value} was not restored on the response path: {content}"
        );
    }
    assert!(
        !content.contains("[TAXID_"),
        "a placeholder leaked to the client: {content}"
    );
}

/// **VAT-19 (M11 Track A) — a `[TAXID_n]` split across SSE chunk boundaries is still restored.**
///
/// The streaming de-masker buffers across events because a placeholder can straddle them
/// (`E2E`'s split-placeholder guard proves it for `[EMAIL_1]`). `[TAXID_1]` is a **different
/// length**, and the buffering logic is length-sensitive by nature — it has to decide how much to
/// hold back — so "email works" does not imply "taxid works". The mock fragments the reply into
/// 4-character pieces, which splits a 9-character token three ways.
#[tokio::test]
async fn a_split_taxid_placeholder_is_restored_in_a_stream() {
    let (upstream, seen) = spawn_sse_mock().await;
    let proxy = spawn_proxy(upstream).await;

    let sent = "la partita IVA e' 00905811006 grazie";
    let raw = reqwest::Client::new()
        .post(format!("http://{proxy}/v1/chat/completions"))
        .json(&json!({
            "model": "gpt-x",
            "stream": true,
            "messages": [{ "role": "user", "content": sent }]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let body: Value = serde_json::from_str(&seen.lock().unwrap().clone()).unwrap();
    let masked_user = upstream_last_user(&body);
    assert!(
        !masked_user.contains("00905811006"),
        "the P.IVA reached the upstream in clear: {masked_user}"
    );
    assert!(
        masked_user.contains("[TAXID_1]"),
        "expected the placeholder in the masked user message: {masked_user}"
    );

    let content = sse_content(&raw);
    assert_eq!(
        content, sent,
        "the reassembled stream must carry the real P.IVA back, byte-identically"
    );
    assert!(
        !raw.contains("[TAXID_"),
        "a placeholder leaked to the client mid-stream: {raw}"
    );
    assert!(raw.contains("[DONE]"), "terminator preserved");
}

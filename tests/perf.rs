//! System-level performance / load harness (M5, PERF-01) — the companion to
//! `tests/complexity.rs`, which pins the masking *algorithm*'s complexity in
//! isolation (no HTTP). This file exercises the real request path: concurrent
//! connections and streaming throughput, over the actual axum router.
//!
//! **What it proves.** The M4-R19 architecture claim is that masking is
//! CPU-bound and runs on `tokio::task::spawn_blocking`, specifically so a
//! handful of concurrent large-body requests can never starve the async
//! executor — `/healthz` must keep answering regardless. That claim was
//! previously only checked by hand (`docs/ARCHITECTURE.md`: "eight
//! concurrently in 4.3 s while `/healthz` answers in 48 ms"); this makes it a
//! repeatable guard.
//!
//! **Budgets, not benchmarks.** Like `tests/complexity.rs`, these are generous
//! wall-clock ceilings — orders of magnitude above the measured numbers — so a
//! regression back to *seconds-to-minutes* fails the suite, without the test
//! flaking on a slower CI runner. Exact timings are `eprintln!`'d (`--nocapture`)
//! and recorded in `docs/DEVLOG.md`, not asserted precisely.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::{response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::timeout;

use llm_proxy_pii_rust::config::{Config, DEFAULT_MAX_BODY_BYTES};
use llm_proxy_pii_rust::server::{build_router, AppState};

const CONCURRENT_BUDGET: Duration = Duration::from_secs(30);
const HEALTHZ_BUDGET: Duration = Duration::from_secs(2);
const STREAM_BUDGET: Duration = Duration::from_secs(20);

async fn spawn_mock_upstream_echo() -> SocketAddr {
    async fn handler(Json(_body): Json<Value>) -> Json<Value> {
        Json(json!({ "choices": [{ "message": { "role": "assistant", "content": "ok" } }] }))
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
        pii_max_phone_validations:
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// A body with many small PII entities — the DOS-04 shape, the one that
/// actually costs CPU time on the masking path (many placeholders spliced, not
/// just a big string). ~350 KB / 50 K entities per request is enough to keep
/// eight of them busy on the blocking pool without making the test itself slow.
fn many_entities_body() -> Value {
    let content = "a@b.co ".repeat(50_000);
    json!({
        "model": "gpt-x",
        "messages": [{ "role": "user", "content": content }]
    })
}

#[tokio::test]
async fn healthz_stays_responsive_under_concurrent_masking_load() {
    let proxy = spawn_proxy(spawn_mock_upstream_echo().await).await;
    let client = reqwest::Client::new();

    let result = timeout(CONCURRENT_BUDGET, async {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let client = client.clone();
            let body = many_entities_body();
            handles.push(tokio::spawn(async move {
                client
                    .post(format!("http://{proxy}/v1/chat/completions"))
                    .json(&body)
                    .send()
                    .await
                    .expect("request failed")
                    .status()
            }));
        }

        // Give the concurrent requests a moment to actually start masking, then
        // probe /healthz *while that CPU-bound work is in flight*.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = Instant::now();
        let healthz_status = client
            .get(format!("http://{proxy}/healthz"))
            .send()
            .await
            .expect("healthz request failed")
            .status();
        let healthz_elapsed = started.elapsed();

        let mut statuses = Vec::with_capacity(handles.len());
        for handle in handles {
            statuses.push(handle.await.expect("request task panicked"));
        }
        (statuses, healthz_status, healthz_elapsed)
    })
    .await
    .expect("concurrent load did not complete within budget — the async executor may be starved");

    let (statuses, healthz_status, healthz_elapsed) = result;
    eprintln!("8x 50K-entity concurrent requests; /healthz during load: {healthz_elapsed:?}");

    for status in statuses {
        assert!(status.is_success(), "a concurrent request failed: {status}");
    }
    assert!(healthz_status.is_success());
    assert!(
        healthz_elapsed < HEALTHZ_BUDGET,
        "/healthz took {healthz_elapsed:?} while masking ran concurrently — the executor is starved"
    );
}

/// A mock upstream that ignores the request and always streams a large,
/// **placeholder-laden** response in small SSE fragments — so the incremental
/// de-anonymizer's hold-back buffer does real, repeated restore work across a
/// large number of events, not a single lucky placeholder.
async fn spawn_sse_placeholder_mock(payload: String) -> SocketAddr {
    let handler = move |Json(_body): Json<Value>| {
        let payload = payload.clone();
        async move {
            let chars: Vec<char> = payload.chars().collect();
            let mut sse = String::with_capacity(payload.len() * 2);
            for chunk in chars.chunks(8) {
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
    addr
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
async fn streaming_throughput_of_repeated_placeholder_restoration_stays_within_budget() {
    // The request itself carries PII, so the vault is populated with
    // `bob@test.com -> [EMAIL_1]` — then the (fixed) mock response references
    // that placeholder thousands of times, split across small SSE fragments, so
    // this genuinely exercises `SseDemasker`'s restore path at scale rather than
    // streaming through untouched (an empty vault takes the passthrough branch).
    let unit = "please email [EMAIL_1] now and again [EMAIL_1], ";
    let payload = unit.repeat(3_000); // ~150 KB, ~6000 placeholder occurrences
    let proxy = spawn_proxy(spawn_sse_placeholder_mock(payload.clone()).await).await;

    let started = Instant::now();
    let raw = timeout(STREAM_BUDGET, async {
        reqwest::Client::new()
            .post(format!("http://{proxy}/v1/chat/completions"))
            .json(&json!({
                "model": "gpt-x",
                "stream": true,
                "messages": [{ "role": "user", "content": "please remember bob@test.com" }]
            }))
            .send()
            .await
            .expect("request failed")
            .text()
            .await
            .expect("reading the stream body failed")
    })
    .await
    .expect("streaming restoration did not complete within budget");
    let elapsed = started.elapsed();
    eprintln!(
        "streaming ~{} KB of repeated placeholders through the de-anonymizer: {elapsed:?}",
        payload.len() / 1024
    );

    assert!(raw.contains("[DONE]"));
    assert!(
        !raw.contains("[EMAIL_1]"),
        "a placeholder survived the stream unresolved"
    );
    let content = sse_content(&raw);
    let expected = "please email bob@test.com now and again bob@test.com, ".repeat(3_000);
    assert_eq!(content, expected, "every occurrence must be restored");
}

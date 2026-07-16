//! Real-provider smoke tests against Anthropic — **both routes** (E2E-INT-01 in
//! `docs/TESTING.md`). Anthropic is the only provider we have credentials for, and
//! these are strictly opt-in: `#[ignore]`d, need a real `ANTHROPIC_API_KEY`, make
//! one real network call each, and must never run in CI (no key is configured there).
//!
//! Two routes ship, so both get a live check — they are **not** interchangeable:
//!
//! - [`e2e_int01_anthropic_real_provider_roundtrip`] — the **OpenAI-compat** route
//!   (`/v1/chat/completions`), what Cline / Continue / opencode / Copilot-BYOK speak.
//! - [`e2e_int01_anthropic_native_messages_roundtrip`] — the **native** route
//!   (`/v1/messages`, M6), what Claude Code and the Anthropic SDK speak.
//!
//! The manual `CC-*` battery (`docs/MANUAL_VERIFICATION.md`) drives a real Claude
//! Code *client* through the native route; it proves something different from these,
//! and neither replaces the other. **By prefix: `E2E-INT-*` is cargo, `CC-*` is a
//! human.**
//!
//! Run explicitly:
//!
//! ```text
//! set ANTHROPIC_API_KEY=<your token>
//! cargo test --test anthropic_smoke -- --ignored --nocapture
//! ```
//!
//! **Either credential mode works** (see `docs/MANUAL_VERIFICATION.md`). `ANTHROPIC_API_KEY`
//! is the token this test forwards; whether it is an API key or a token issued to your own
//! user does not matter, because the proxy forwards the client's `Authorization` **verbatim**
//! and never inspects it. This test drives the "client passes its own token" path — it sends
//! the credential as a client `Authorization` header and leaves the proxy's own
//! `upstream_api_key` **unset**, which is both the recommended posture and the stricter thing
//! to prove: the proxy never has to hold the key.
//!
//! Optional overrides: `ANTHROPIC_BASE_URL` (default `https://api.anthropic.com`),
//! `ANTHROPIC_MODEL` (default `claude-3-5-haiku-latest`). Proves the full chain
//! against a real upstream: the request leaves masked, and the client still gets
//! the real value back. For a from-the-outside, trace-logged check against a real
//! provider, see `docs/MANUAL_VERIFICATION.md` (E2E-INT-02) — that procedure is
//! manual by nature (it compares two runs' *logs*, which a `#[test]` can't assert).

use std::net::SocketAddr;

use serde_json::{json, Value};
use tokio::net::TcpListener;

use llm_proxy_pii_rust::config::{Config, DEFAULT_MAX_BODY_BYTES};
use llm_proxy_pii_rust::server::{build_router, AppState};

/// The opt-in credential; `None` skips (the test panics with the reason).
fn token() -> String {
    std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY to run this opt-in real-provider smoke test")
}

fn model() -> String {
    std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-3-5-haiku-latest".to_string())
}

/// Spawn the proxy against the **real** Anthropic upstream and return its address.
///
/// `upstream_api_key` is deliberately `None`: the token is sent by the *client* in
/// each test, and the proxy forwards a client credential verbatim in preference to
/// its own. That is both the recommended posture and the stricter thing to prove —
/// the proxy never holds the credential at all. (Configuring a key here would
/// exercise the other, weaker path and would mask a regression in this one.)
async fn spawn_proxy_against_real_anthropic() -> SocketAddr {
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: base_url,
        upstream_api_key: None,
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
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// A value unlikely to appear in the model's training data, so a match in the reply
/// can only be our own round-trip, not a coincidence.
const CHECK_EMAIL: &str = "roundtrip-check-m5@example.com";

fn literal_prompt(email: &str) -> String {
    format!("Reply with exactly this sentence and nothing else: contact {email} for details.")
}

/// Assert the client-visible text carries the real value and no stray placeholder.
fn assert_restored(text: &str, email: &str) {
    assert!(
        text.contains(email),
        "expected the real email restored in the client-visible reply, got: {text}"
    );
    assert!(
        !text.contains("[EMAIL_1]"),
        "a raw placeholder reached the client unresolved: {text}"
    );
}

/// E2E-INT-01a — the **OpenAI-compat** route (`/v1/chat/completions`).
#[tokio::test]
#[ignore]
async fn e2e_int01_anthropic_real_provider_roundtrip() {
    let addr = spawn_proxy_against_real_anthropic().await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .header("authorization", format!("Bearer {}", token()))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": model(),
            "max_tokens": 64,
            "messages": [{ "role": "user", "content": literal_prompt(CHECK_EMAIL) }]
        }))
        .send()
        .await
        .expect("request to the real proxy failed");

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.expect("valid JSON reply");
    assert_restored(
        body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default(),
        CHECK_EMAIL,
    );
}

/// E2E-INT-01b — the **native** Messages route (`/v1/messages`, M6).
///
/// The automated companion to the manual CC battery: same proof (the request leaves
/// masked, the client still gets the real value back), but on the native schema —
/// a top-level `content[]` reply rather than `choices[].message`, and the native
/// auth path (client `Authorization` forwarded verbatim, never into `x-api-key`).
#[tokio::test]
#[ignore]
async fn e2e_int01_anthropic_native_messages_roundtrip() {
    let addr = spawn_proxy_against_real_anthropic().await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("authorization", format!("Bearer {}", token()))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": model(),
            "max_tokens": 64,
            "messages": [{ "role": "user", "content": literal_prompt(CHECK_EMAIL) }]
        }))
        .send()
        .await
        .expect("request to the real proxy failed");

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.expect("valid JSON reply");

    // The native reply is a top-level `content[]` array; concatenate its text blocks.
    let text: String = body["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    assert_restored(&text, CHECK_EMAIL);
}

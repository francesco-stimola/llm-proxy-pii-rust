//! Real-provider smoke test against Anthropic's OpenAI-compatible endpoint
//! (E2E-INT-01 in `docs/TESTING.md`). This is the **only** real-provider smoke —
//! Anthropic is the only provider we have credentials for — and it is strictly
//! opt-in: `#[ignore]`d, needs a real `ANTHROPIC_API_KEY`, makes one real network
//! call, and must never run in CI (no key is ever configured there).
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

use serde_json::{json, Value};
use tokio::net::TcpListener;

use llm_proxy_pii_rust::config::{Config, DEFAULT_MAX_BODY_BYTES};
use llm_proxy_pii_rust::server::{build_router, AppState};

#[tokio::test]
#[ignore]
async fn e2e_int01_anthropic_real_provider_roundtrip() {
    let token = std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY to run this opt-in real-provider smoke test");
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let model =
        std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-3-5-haiku-latest".to_string());

    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream_base_url: base_url,
        // Deliberately `None`: the token is sent by the *client* below, and the proxy forwards
        // a client `Authorization` verbatim in preference to its own configured key. This is
        // both the recommended posture and the stricter thing to prove — the proxy never holds
        // the credential at all. (Setting `upstream_api_key` here would exercise the other,
        // weaker path and would mask a regression in the client-token one.)
        upstream_api_key: None,
        max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        provider: "anthropic".to_string(),
        upstream_chat_path: "/v1/chat/completions".to_string(),
        upstream_extra_headers: Vec::new(),
        forward_request_headers: vec!["anthropic-version".to_string()],
        pii_locales: vec!["it".to_string(), "us".to_string()],
        debug_skip_demask: false,
    };
    let app = build_router(AppState::new(&config).await.expect("build app state"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // A value unlikely to appear in the model's training data, so a match in the
    // reply can only be our own round-trip, not a coincidence.
    let email = "roundtrip-check-m5@example.com";
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .header("authorization", format!("Bearer {token}"))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": model,
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": format!(
                    "Reply with exactly this sentence and nothing else: contact {email} for details."
                )
            }]
        }))
        .send()
        .await
        .expect("request to the real proxy failed");

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: Value = resp.json().await.expect("valid JSON reply");
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();

    assert!(
        content.contains(email),
        "expected the real email restored in the client-visible reply, got: {content}"
    );
    assert!(
        !content.contains("[EMAIL_1]"),
        "a raw placeholder reached the client unresolved: {content}"
    );
}

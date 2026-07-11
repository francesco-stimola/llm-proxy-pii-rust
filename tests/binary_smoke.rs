//! Live smoke test of the compiled binary itself.
//!
//! The other integration tests drive the router in-process (`build_router`), so
//! they skip the actual startup path. This one spawns the real `.exe`
//! (`main` → `Config::from_env` → `run` → bind a real listener) against an
//! in-process mock upstream and confirms a single PII request is masked outbound,
//! augmented, and restored inbound.
//!
//! Kept to ONE case on purpose: subprocess tests are slower and more
//! timing-sensitive, so the bulk of coverage stays in-process.

use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command};
use std::time::Duration;

use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use tokio::net::TcpListener as TokioListener;

/// Kills the spawned proxy process when the test ends, even on a panic.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Mock upstream: echo the (masked) body under `upstream_received` — which is
/// outside the proxy's restore path — and reply quoting the masked last user
/// message, which the proxy *does* restore.
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
        "choices": [{ "message": { "role": "assistant", "content": format!("You said: {last_user}") } }],
        "upstream_received": body
    }))
}

async fn spawn_mock_upstream() -> SocketAddr {
    let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_upstream_handler));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Grab an OS-assigned free port, then release it for the child to bind.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn binary_boots_and_does_the_pii_roundtrip() {
    let upstream = spawn_mock_upstream().await;
    let port = free_port();

    let child = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .env("LISTEN_ADDR", format!("127.0.0.1:{port}"))
        .env("UPSTREAM_BASE_URL", format!("http://{upstream}"))
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("failed to spawn the proxy binary");
    let _guard = ChildGuard(child);

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");

    // Wait (up to ~10s) for the real binary to bind and serve.
    let mut healthy = false;
    for _ in 0..100 {
        if let Ok(resp) = client.get(format!("{base}/healthz")).send().await {
            if resp.status().is_success() {
                healthy = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(healthy, "proxy binary did not become healthy on {base}");

    let reply: Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "gpt-x",
            "messages": [{
                "role": "user",
                "content": "Scrivimi a mario.rossi@example.com, IBAN IT60X0542811101000000123456, \
                            chiave sk-ant-api01-test000000000000000000000000000000000000"
            }]
        }))
        .send()
        .await
        .expect("request to the proxy failed")
        .json()
        .await
        .expect("proxy reply was not JSON");

    // 1) What the upstream saw (echoed, outside the restore path): masked + augmented.
    let seen = reply["upstream_received"].to_string();
    assert!(!seen.contains("mario.rossi@example.com"), "email leaked upstream");
    assert!(
        !seen.contains("IT60X0542811101000000123456"),
        "IBAN leaked upstream"
    );
    assert!(!seen.contains("sk-ant-api01-test0"), "secret leaked upstream");
    assert!(
        seen.contains("[EMAIL_1]") && seen.contains("[IBAN_1]") && seen.contains("[SECRET_1]"),
        "expected typed placeholders upstream, got: {seen}"
    );
    assert_eq!(
        reply["upstream_received"]["messages"][0]["role"], "system",
        "augmentation system message was not injected"
    );

    // 2) What the client got back: the real values, restored.
    let content = reply["choices"][0]["message"]["content"]
        .as_str()
        .expect("assistant content was not a string");
    assert!(
        content.contains("mario.rossi@example.com"),
        "email not restored: {content}"
    );
    assert!(
        content.contains("IT60X0542811101000000123456"),
        "IBAN not restored: {content}"
    );
    assert!(
        content.contains("sk-ant-api01-test0"),
        "secret not restored: {content}"
    );
}

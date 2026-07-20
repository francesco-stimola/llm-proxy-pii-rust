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

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
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
        .rfind(|m| m["role"] == "user")
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
    assert!(
        !seen.contains("mario.rossi@example.com"),
        "email leaked upstream"
    );
    assert!(
        !seen.contains("IT60X0542811101000000123456"),
        "IBAN leaked upstream"
    );
    assert!(
        !seen.contains("sk-ant-api01-test0"),
        "secret leaked upstream"
    );
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

/// **CLI-01 (M9-R4).** An unrecognized argument must be **refused**, not ignored.
///
/// Before M9 the binary took no arguments, so discarding them was harmless. Once
/// `--bench-providers` existed, a near-miss (`--bench-provider`, `--bench-providers=1`, `--help`
/// before it was handled) fell through to `Config::from_env` + `run` and started a **live proxy
/// pointed at the configured upstream**, while the operator believed they had run a diagnostic.
/// That is fail-*open* on unexpected input, which nothing else in this codebase does — including
/// this milestone's own `NER_EXECUTION_PROVIDER` typo check.
///
/// Both halves matter: a non-zero exit alone would still pass for a binary that served for a
/// while and then exited, so this also asserts the port **never becomes healthy**.
#[tokio::test]
async fn unknown_cli_argument_is_refused_and_never_binds() {
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");

    let child = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .arg("--bench-provider") // the singular typo — the way this is actually hit
        .env("LISTEN_ADDR", format!("127.0.0.1:{port}"))
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("failed to spawn the proxy binary");
    let mut guard = ChildGuard(child);

    // It must exit, and exit non-zero.
    let status = {
        let mut waited = None;
        for _ in 0..100 {
            match guard.0.try_wait().expect("try_wait failed") {
                Some(status) => {
                    waited = Some(status);
                    break;
                }
                None => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
        waited.expect("binary did not exit after an unrecognized argument — it is still running")
    };
    assert!(
        !status.success(),
        "an unrecognized argument must exit non-zero, got {status:?}"
    );

    // And it must never have served: nothing ever answers on the port it was given.
    let client = reqwest::Client::new();
    assert!(
        client.get(format!("{base}/healthz")).send().await.is_err(),
        "a proxy became reachable on {base} after an unrecognized argument — it fell through to \
         serving instead of refusing"
    );
}

/// **CLI-03 (M9-R14, M9-R16).** `--bench-providers` must never tell an operator to rebuild with
/// an `ep-*` cargo feature.
///
/// Every platform's accelerator is wired **per-target** in `Cargo.toml`, so `--features onnx`
/// already carries it — advice naming a feature sends the operator to rebuild something they
/// have. Worse, `ep-directml` is Windows-only, so naming it to a macOS or Linux operator points
/// at a backend no hardware of theirs can provide. M9-R14 fixed this in the `onnx` branch and
/// **left the `cfg(not(onnx))` branch saying it verbatim** — the defect survived because the test
/// M9-R14 asked for was never written. This is that test, and it is deliberately written to run
/// in **both** builds so neither branch can drift back.
#[tokio::test]
async fn bench_providers_never_advises_an_ep_feature_rebuild() {
    let out = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .arg("--bench-providers")
        .env("RUST_LOG", "error")
        .output()
        .expect("failed to spawn the proxy binary");

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for feature in [
        "ep-directml",
        "ep-cuda",
        "ep-coreml",
        "ep-rocm",
        "ep-openvino",
        "ep-tensorrt",
        "ep-webgpu",
    ] {
        assert!(
            !text.contains(feature),
            "--bench-providers advised `{feature}`, but the platform accelerator is wired \
             per-target and needs no such rebuild (M9-R14/M9-R16). Output was:\n{text}"
        );
    }
}

/// **CLI-02 (M9-R4).** `--help` prints usage and exits **0** — the companion to refusing unknown
/// arguments, so the natural way to ask "what does this take?" is not itself an error.
#[tokio::test]
async fn help_prints_usage_and_exits_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .arg("--help")
        .env("RUST_LOG", "warn")
        .output()
        .expect("failed to spawn the proxy binary");

    assert!(
        out.status.success(),
        "--help must exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--bench-providers"),
        "usage must document the flags, got: {stdout}"
    );
}

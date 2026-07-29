//! Live smoke test of the compiled binary itself.
//!
//! The other integration tests drive the router in-process (`build_router`), so
//! they skip the actual startup path. This one spawns the real `.exe`
//! (`main` → `Config::from_env` → `run` → bind a real listener) against an
//! in-process mock upstream and confirms a single PII request is masked outbound,
//! augmented, and restored inbound.
//!
//! Everything else here is the **CLI surface** (CLI-01…CLI-06) — argument handling, the
//! version/help output and the log defaults. Those live here for the same reason: each is a
//! property of the *process*, unreachable from `build_router`. The bulk of coverage still
//! stays in-process, because subprocess tests are slower and more timing-sensitive; nothing
//! joins this file unless spawning the real `.exe` is what makes the assertion true.

use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command};
use std::time::Duration;

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener as TokioListener;

/// Kills the spawned proxy process when the test ends, even on a panic.
///
/// `Option` so a test can [`take`](ChildGuard::take) the child back when it needs to
/// `wait_with_output()` — the guard then has nothing left to kill, and an assertion that
/// panics before the take still cleans up.
struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().expect("child already taken")
    }

    fn take(&mut self) -> Child {
        self.0.take().expect("child already taken")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    let _guard = ChildGuard(Some(child));

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
    let mut guard = ChildGuard(Some(child));

    // It must exit, and exit non-zero.
    let status = {
        let mut waited = None;
        for _ in 0..100 {
            match guard.child().try_wait().expect("try_wait failed") {
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

    // **Position must not matter (M10-R11).** `--version` / `--help` used to dispatch as the
    // scan reached them, so anything *after* one was never examined: `--version --bogus` printed
    // the version and exited 0 while `--bogus --version` was correctly refused. Neither order
    // binds a listener, so M9-R4's sharp risk was never reopened — but "an unrecognized argument
    // is a mistake" should not depend on where the mistake sits in the line.
    for args in [
        ["--version", "--bogus"],
        ["--help", "--bogus"],
        ["--bench-providers", "--bogus"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
            .args(args)
            .env("RUST_LOG", "warn")
            .output()
            .expect("failed to spawn the proxy binary");
        assert!(
            !out.status.success(),
            "{args:?} must be refused whatever the order, got {:?}",
            out.status
        );
    }
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

/// **CLI-04 (M10).** `--version` prints the manifest version and exits **0**.
///
/// A release asset is a bare executable: once it is saved next to an older copy, nothing
/// identifies it — and because unknown arguments are refused (CLI-01), asking the obvious way
/// did not merely print nothing, it **failed to start**. The rejected alternative was putting the
/// version in the artifact filename: a filename is a convention a rename can break, while a
/// binary reporting `CARGO_PKG_VERSION` cannot lie (and the filename approach also forfeits
/// GitHub's stable `releases/latest/download/<name>` redirect).
///
/// It must also work **without a valid upstream configuration** — this test sets none, so a
/// `--version` handled after `Config::from_env` would fail here.
#[tokio::test]
async fn version_prints_the_manifest_version_and_exits_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .arg("--version")
        .env("RUST_LOG", "warn")
        .env("UPSTREAM_BASE_URL", "") // deliberately useless config
        .output()
        .expect("failed to spawn the proxy binary");

    assert!(
        out.status.success(),
        "--version must exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "--version must print the manifest version {}, got: {stdout}",
        env!("CARGO_PKG_VERSION")
    );
    // "Which build is this?" is three questions, and the third decides whether names are
    // masked at all — so the target and the ML layer are part of the answer.
    assert!(
        stdout.contains("target:") && stdout.contains("ml (onnx feature):"),
        "--version must report the target triple and whether the ML layer is compiled in, \
         got: {stdout}"
    );
}

/// **CLI-05 (M10).** Every environment variable the code reads is **named** in `--help`.
///
/// Configuration here is *entirely* environment variables with no config file, so a help text
/// that lists the flags and defers to `README.md` means you need the repo to run the program.
/// The risk of writing that text by hand is drift, and a stale help text is worse than a short
/// one — so this extracts the keys from the source and fails the first time someone adds a
/// variable without documenting it.
///
/// **Deliberate scope limit:** it proves each key is *named*, not that its description is
/// accurate. A wrong default in the help text is not caught here (nor by any test) — it is
/// caught by review.
#[tokio::test]
async fn help_names_every_environment_variable_the_code_reads() {
    // **Walk the tree, don't list files (M10-R8).** The first version scanned a hand-written
    // three-file list and was already incomplete the day it was written: `src/pii/hf.rs`
    // reads `HF_HUB_CACHE` and `HF_HOME`, both operator-facing and both documented in
    // `docs/SETUP.md`, and the guard reported success because it never looked there. A list
    // of places to check is the same kind of artefact as the help text it is guarding.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src/ must be readable") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path
                    .strip_prefix(root.parent().unwrap())
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                sources.push((name, std::fs::read_to_string(&path).expect("readable")));
            }
        }
    }
    assert!(
        sources.len() >= 10,
        "found only {} source files under src/ — the walk is broken",
        sources.len()
    );

    // Variables the program *reads* but does not *define*: OS-provided, and documenting them
    // in our `--help` would claim ownership we don't have. Listed here so the exclusion is a
    // visible decision rather than an invisible gap.
    const NOT_OURS: &[&str] = &["USERPROFILE", "HOME"];

    // Only *literal* keys: `env::var(key)` inside the `env_flag` / `env_or` helpers is a
    // variable, not a key, and must not be extracted.
    let pattern =
        regex::Regex::new(r#"(?:env::var(?:_os)?|env_flag|env_or)\(\s*"([A-Z][A-Z0-9_]*)"#)
            .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .arg("--help")
        .env("RUST_LOG", "warn")
        .output()
        .expect("failed to spawn the proxy binary");
    let help = String::from_utf8_lossy(&out.stdout);

    let mut missing: Vec<String> = Vec::new();
    let mut found = 0usize;
    for (path, source) in &sources {
        for caps in pattern.captures_iter(source) {
            let key = &caps[1];
            if NOT_OURS.contains(&key) {
                continue;
            }
            found += 1;
            if !help.contains(key) {
                missing.push(format!("{key} (read in {path})"));
            }
        }
    }

    // A guard that finds nothing to check is not a guard — if the extraction regex ever stops
    // matching the code's shape, this catches it instead of passing vacuously.
    assert!(
        found >= 20,
        "the env-key extractor matched only {found} keys — the source shape changed and this \
         guard is no longer checking anything"
    );
    assert!(
        missing.is_empty(),
        "these environment variables are read by the code but not named in `--help`:\n  {}\n\
         Add them to `ENV_REFERENCE` in src/main.rs — the shipped binary must be able to say \
         what it accepts.",
        missing.join("\n  ")
    );
}

/// **CLI-06 (M10).** With `RUST_LOG` **unset** the binary still logs, and every timestamp
/// carries an **explicit** UTC offset.
///
/// Two defects in one line, both invisible from the source and both found by running the
/// shipped artifact:
///
/// 1. `EnvFilter::from_default_env()` falls back to ERROR-only, so the released binary printed
///    **nothing at all**. That breaks the one check this project asks operators to make ("if the
///    `ONNX NER detector loaded` line is missing you are running structured-only"), because with
///    no `RUST_LOG` *every* line is missing — and a silent process looks exactly like a healthy
///    one.
/// 2. Timestamps were UTC on a proxy that runs next to the person reading its logs. Local time
///    is the fix, but the regression it invites is a *bare* local time (`12:37:04`) — correct on
///    the author's machine, ambiguous everywhere else. So the assertion is on the **offset**,
///    which is what makes the line readable anywhere, not on the wall-clock value, which depends
///    on the machine.
///
/// `env_remove` rather than `env` — the child must inherit an environment with no `RUST_LOG`,
/// and mutating this process's env would race the other tests.
#[tokio::test]
async fn default_log_level_is_info_and_timestamps_carry_an_offset() {
    use std::process::Stdio;

    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_llm-proxy-pii-rust"))
        .env("LISTEN_ADDR", format!("127.0.0.1:{port}"))
        .env_remove("RUST_LOG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn the proxy binary");
    let mut guard = ChildGuard(Some(child));

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
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

    // Stop it, then drain what it wrote — `wait_with_output` consumes the child, so take it
    // out of the guard (which has kept it alive through every assertion above).
    let _ = guard.child().kill();
    let out = guard
        .take()
        .wait_with_output()
        .expect("failed to collect output");
    // **Strip ANSI before matching (M10-R4).** `tracing-subscriber` does no TTY detection:
    // it colours whenever `NO_COLOR` is unset, *including into a pipe*. So the raw line
    // starts with an escape sequence, not with the timestamp, and an `^`-anchored regex
    // fails — on an ordinary developer terminal and in CI (`ci.yml` sets
    // `CARGO_TERM_COLOR: always` and no `NO_COLOR`), while passing in a shell that happens
    // to export `NO_COLOR=1`. Stripping here keeps the guard asserting a property of the
    // **binary** rather than of the environment it was run in; setting `NO_COLOR` instead
    // would have done the opposite.
    let ansi = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    let logged = ansi
        .replace_all(
            &format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            "",
        )
        .into_owned();

    assert!(
        logged.contains("listening on"),
        "with RUST_LOG unset the binary logged nothing at INFO — a released proxy that starts \
         silently looks identical to a broken one. Got:\n{logged}"
    );

    // `Z` or `[+-]HH:MM`, but never a bare local time.
    let stamped = regex::Regex::new(
        r"(?m)^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}(?:Z|[+-]\d{2}:\d{2})\s",
    )
    .unwrap();
    let lines: Vec<&str> = logged
        .lines()
        .filter(|l| l.contains("listening on"))
        .collect();
    assert!(
        !lines.is_empty() && lines.iter().all(|line| stamped.is_match(line)),
        "every log line must start with a timestamp carrying an EXPLICIT offset (`Z` or \
         `+HH:MM`) — a bare local time reads correctly only on the machine that wrote it. \
         Got:\n{}",
        lines.join("\n")
    );

    // The strip must not be able to swallow the thing being tested (M10-R4): a bare local
    // time still fails, ANSI-coloured or not.
    let bare = "\x1b[2m2026-07-29T12:37:04.844780\x1b[0m  INFO llm_proxy: listening on x";
    assert!(
        !stamped.is_match(&ansi.replace_all(bare, "")),
        "the offset check must still reject a timestamp with no zone at all"
    );
}

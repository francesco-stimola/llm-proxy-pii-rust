//! Dependency-footprint guard (M2.5-R1).
//!
//! The ONNX / HuggingFace stack — `hf-hub` (→ `hf-xet` → `aws-lc-rs`, a second
//! `reqwest`), `ort`, `tokenizers` — must stay behind the **`onnx`** feature so the
//! **default** build is native-dep-free. This asserts it directly from `cargo tree`
//! on the default feature set, so a future accidental un-gating (or a `default`
//! feature enabling `onnx`) is caught.
//!
//! For reference, the intended `--features onnx` footprint *does* include that stack
//! (hf-hub 1.x pulls `hf-xet` + `rustls`/`aws-lc-rs` + a second `reqwest 0.13`); that
//! is a deliberate, documented trade-off (docs/reviews/M2.5.md#m25-r1), not a regression.

use std::process::Command;

/// Crate names that must never appear in the default (`onnx`-off) dependency tree.
const FORBIDDEN_IN_DEFAULT: &[&str] = &[
    "hf-hub v",
    "hf-xet v",
    "aws-lc-rs v",
    "aws-lc-sys v",
    "ort v",
    "tokenizers v",
];

#[test]
fn default_build_excludes_the_onnx_and_hf_stack() {
    // Cargo sets `CARGO` to its own path for tests; fall back to PATH lookup.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        // Default features only (`default = []`), shipped edges only (no dev-deps),
        // offline + locked so it never mutates Cargo.lock or hits the network.
        .args([
            "tree",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--offline",
            "--locked",
        ])
        .output();

    let output = match output {
        Ok(o) => o,
        // No cargo on PATH (unusual for a test run) — skip rather than fail spuriously.
        Err(e) => {
            eprintln!("skipping footprint guard: could not run `cargo tree`: {e}");
            return;
        }
    };
    assert!(
        output.status.success(),
        "`cargo tree` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN_IN_DEFAULT {
        assert!(
            !tree.contains(forbidden),
            "default build unexpectedly pulls `{}` — the ONNX/HF stack must stay behind \
             the `onnx` feature (M2.5-R1)",
            forbidden.trim_end_matches(" v")
        );
    }
}

/// **DEP-02 (M10-R9) — the *property*, not a denylist of six names.**
///
/// The rule this project states three times — *"the default build is native-dep-free, and
/// that is now enforced by `tests/dependency_footprint.rs`, not by a comment"* — was enforced
/// by six literal strings (`hf-hub`, `hf-xet`, `aws-lc-rs`, `aws-lc-sys`, `ort`,
/// `tokenizers`). That is a **regression guard for one known stack**, and it cannot observe a
/// *new* native dependency at all — so "absent from the list" carries no information about
/// native-dep-freedom. M10 nonetheless cited it that way to justify adding `time` to the
/// default build. The conclusion happened to be right; the argument could not support it.
///
/// This asserts the property instead: nothing reachable in the default build may **compile or
/// wrap C**. The markers are `cc` / `cmake` / `bindgen` (a build script that invokes a C
/// toolchain) and the `*-sys` naming convention (a crate wrapping a native library).
///
/// **The edge filter is `normal,build`, and that detail is load-bearing.** `-e build` alone
/// descends *only* through build-dependency edges, so from a root with no build-deps it
/// returns a one-line tree and the guard passes seeing nothing at all. Checked against the
/// `onnx` feature set, where the answer must be non-empty: that flags `ort-sys`, `aws-lc-sys`,
/// `cc` and `cmake` exactly as it should.
///
/// **Allowances are explicit**, because a blanket ban would fail on things that are not what
/// the rule is about.
#[test]
fn the_default_build_compiles_no_native_code() {
    // `windows-sys` carries the `-sys` suffix but is **not** a native dependency in the sense
    // this rule cares about: it is pure Rust declarations over the OS's own API, with no C
    // compiled and nothing vendored, and it arrives via tokio/reqwest on every Windows build.
    // The rule is about *our build compiling or vendoring a C library*, not about calling the
    // operating system.
    const ALLOWED: &[&str] = &["windows-sys"];

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args([
            "tree",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--offline",
            "--locked",
        ])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skipping native-code guard: could not run `cargo tree`: {e}");
            return;
        }
    };
    assert!(
        output.status.success(),
        "`cargo tree -e normal,build` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8_lossy(&output.stdout);

    // Non-vacuity: a guard that inspects an empty tree passes for the wrong reason, which is
    // precisely the failure mode DEP-01 had.
    assert!(
        tree.lines().count() > 50,
        "`cargo tree` returned only {} lines — the query is wrong and this guard is checking \
         nothing",
        tree.lines().count()
    );

    let mut offenders: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !ALLOWED.contains(name))
        .filter(|name| matches!(*name, "cc" | "cmake" | "bindgen") || name.ends_with("-sys"))
        .collect();
    offenders.sort_unstable();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "the DEFAULT build now reaches native code via {offenders:?}. That breaks the \
         native-dep-free guarantee (M2.5-R1) — the whole point of keeping the ONNX/HF stack \
         behind the `onnx` feature. If it is deliberate, it belongs in ARCHITECTURE → \
         Supply-chain as a documented decision, not in a green test."
    );
}

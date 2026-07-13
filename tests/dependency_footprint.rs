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

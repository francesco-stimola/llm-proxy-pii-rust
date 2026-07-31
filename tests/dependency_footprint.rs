//! Dependency-footprint guard (M2.5-R1).
//!
//! The ONNX / HuggingFace stack — `hf-hub` (→ `hf-xet` → `aws-lc-rs`, a second
//! `reqwest`), `ort`, `tokenizers` — must stay behind the **`onnx`** feature so the
//! **default** build stays free of it. This asserts it directly from `cargo tree`
//! on the default feature set, so a future accidental un-gating (or a `default`
//! feature enabling `onnx`) is caught.
//!
//! For reference, the intended `--features onnx` footprint *does* include that stack
//! (hf-hub 1.x pulls `hf-xet` + `rustls`/`aws-lc-rs` + a second `reqwest 0.13`); that
//! is a deliberate, documented trade-off (docs/reviews/M2.5.md#m25-r1), not a regression.
//!
//! # Why every query here names a target
//!
//! Both guards used to ask `cargo tree` about **the machine running them**, and the
//! answer to "does the default build compile C?" is not the same on every platform.
//! Written and run on Windows, they were green; the first time CI ran them on Linux
//! (2026-07-31) they were red, and correctly so. `reqwest` is pinned to `native-tls`
//! *because* reqwest 0.13 flipped its default to rustls + `aws-lc-rs` — but `native-tls`
//! **is** the platform's own TLS: schannel on Windows (pure declarations, no C),
//! **OpenSSL on Linux**, Security.framework on macOS. The comment in `Cargo.toml` that
//! justified the pin as *"keeps the default build native-dep-free"* was therefore true on
//! one platform and false on the other two — a conclusion drawn from one point of a grid,
//! which is the failure mode M10 met eleven times in its own measurements.
//!
//! So the rule is stated in the form it is actually true in: **the default build never
//! reaches the ONNX/HF stack, on any target, and compiles no C beyond the TLS
//! implementation its host operating system provides.** Each allowance below names the
//! crates that carry that platform's TLS and nothing else, so a *new* native dependency
//! is still caught everywhere.

use std::process::Command;

/// Crate names that must never appear in the default (`onnx`-off) dependency tree,
/// **on any target**. These are the ONNX/HF stack — the thing the `onnx` feature exists
/// to gate — plus `aws-lc-rs`, which is what a silently-flipped reqwest default would drag in.
const FORBIDDEN_IN_DEFAULT: &[&str] = &[
    "hf-hub v",
    "hf-xet v",
    "aws-lc-rs v",
    "aws-lc-sys v",
    "ort v",
    "tokenizers v",
];

/// One release target and the native crates the *platform's own TLS* legitimately brings.
struct Target {
    triple: &'static str,
    /// Why these are allowed — printed on failure so the next reader does not have to
    /// re-derive it from crate names.
    tls: &'static str,
    allowed: &'static [&'static str],
}

/// The targets a release is actually cut for (`release-build.yml`'s matrix). A guard that
/// checks fewer platforms than we ship is a guard that is green by accident of who ran it.
const RELEASE_TARGETS: &[Target] = &[
    Target {
        triple: "x86_64-pc-windows-msvc",
        tls: "schannel — the OS TLS, reached through pure-Rust declarations",
        allowed: &["windows-sys"],
    },
    Target {
        triple: "aarch64-pc-windows-msvc",
        tls: "schannel — the OS TLS, reached through pure-Rust declarations",
        allowed: &["windows-sys"],
    },
    Target {
        triple: "x86_64-unknown-linux-gnu",
        tls: "OpenSSL — the system libssl, linked through openssl-sys (whose build script uses cc)",
        allowed: &["openssl-sys", "cc"],
    },
    Target {
        triple: "aarch64-unknown-linux-gnu",
        tls: "OpenSSL — the system libssl, linked through openssl-sys (whose build script uses cc)",
        allowed: &["openssl-sys", "cc"],
    },
    Target {
        triple: "aarch64-apple-darwin",
        tls: "Security.framework — the OS TLS, via Apple's system frameworks",
        allowed: &[
            "core-foundation-sys",
            "security-framework-sys",
            "system-configuration-sys",
        ],
    },
];

/// Escape hatch for working without a registry: check only the host target.
///
/// Deliberately explicit and deliberately *named* in the skip message. Querying a
/// non-host target needs that target's packages in the local registry (`num_threads` is
/// Linux-only, for instance), which an offline machine may not have. CI never sets this,
/// so the full grid is always checked where it matters — and an offline developer gets a
/// guard that says what it did not check rather than one that quietly checks one platform.
const HOST_ONLY: &str = "DEP_GUARD_HOST_ONLY";

/// `cargo tree` for one target. `None` when the query could not run at all.
///
/// `--locked` is the load-bearing flag: it is what guarantees this never mutates
/// `Cargo.lock`. The host query adds `--offline` because its packages are necessarily
/// present; cross-target queries cannot, because their platform-specific packages may
/// never have been fetched here.
fn tree_for(target: Option<&str>, edges: &str) -> Option<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.args(["tree", "--edges", edges, "--prefix", "none", "--locked"]);
    match target {
        Some(triple) => {
            cmd.args(["--target", triple]);
        }
        None => {
            cmd.arg("--offline");
        }
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        eprintln!(
            "`cargo tree` for {} failed: {}",
            target.unwrap_or("host"),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Every tree this run managed to obtain, labelled. Always includes the host.
///
/// A tree of fewer than 50 lines means the query is wrong and the caller is inspecting
/// nothing — the exact way DEP-01 once passed for the wrong reason — so that is a failure
/// here, not a skip.
fn trees(edges: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let host = tree_for(None, edges).expect("`cargo tree` must run for the host target");
    assert!(
        host.lines().count() > 50,
        "`cargo tree` returned only {} lines for the host — the query is wrong and this \
         guard is checking nothing",
        host.lines().count()
    );
    out.push(("host".to_string(), host));

    if std::env::var(HOST_ONLY).is_ok() {
        eprintln!("{HOST_ONLY} is set — checked the host target only, not the release matrix.");
        return out;
    }

    let mut unreachable = Vec::new();
    for target in RELEASE_TARGETS {
        match tree_for(Some(target.triple), edges) {
            Some(tree) => out.push((target.triple.to_string(), tree)),
            None => unreachable.push(target.triple),
        }
    }
    assert!(
        unreachable.is_empty(),
        "could not query {unreachable:?} — this guard checks the whole release matrix, and a \
         target it cannot see is a target it cannot vouch for. Cross-target queries need those \
         platforms' packages in the local registry, so run this online, or set {HOST_ONLY}=1 to \
         deliberately check the host alone."
    );
    out
}

/// **DEP-01** — the ONNX/HF stack stays behind the `onnx` feature, on every released target.
#[test]
fn default_build_excludes_the_onnx_and_hf_stack() {
    for (label, tree) in trees("normal") {
        for forbidden in FORBIDDEN_IN_DEFAULT {
            assert!(
                !tree.contains(forbidden),
                "the default build for {label} unexpectedly pulls `{}` — the ONNX/HF stack must \
                 stay behind the `onnx` feature (M2.5-R1)",
                forbidden.trim_end_matches(" v")
            );
        }
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
/// wrap C**, beyond the TLS its host OS provides. The markers are `cc` / `cmake` / `bindgen`
/// (a build script invoking a C toolchain) and the `*-sys` naming convention (a crate wrapping
/// a native library).
///
/// **The edge filter is `normal,build`, and that detail is load-bearing.** `-e build` alone
/// descends *only* through build-dependency edges, so from a root with no build-deps it
/// returns a one-line tree and the guard passes seeing nothing at all. Checked against the
/// `onnx` feature set, where the answer must be non-empty: that flags `ort-sys`, `aws-lc-sys`,
/// `cc` and `cmake` exactly as it should.
///
/// **Allowances are per-target and explicit**, because the platform TLS is a different crate
/// on each one and a blanket ban would fail on what the rule is not about. Note the one thing
/// a `cc` allowance costs: on Linux, a *new* dependency that compiles C via `cc` alone would
/// not be flagged by that marker — it would still have to escape the `*-sys` convention too,
/// which is how such crates are named.
#[test]
fn the_default_build_compiles_no_native_code() {
    for (label, tree) in trees("normal,build") {
        let target = RELEASE_TARGETS.iter().find(|t| t.triple == label);
        // The host is whatever machine is running this; allow every platform's TLS crates
        // there, since the release matrix entries are what pin each one precisely.
        let allowed: Vec<&str> = match target {
            Some(t) => t.allowed.to_vec(),
            None => RELEASE_TARGETS
                .iter()
                .flat_map(|t| t.allowed.iter().copied())
                .collect(),
        };

        let mut offenders: Vec<&str> = tree
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|name| !allowed.contains(name))
            .filter(|name| matches!(*name, "cc" | "cmake" | "bindgen") || name.ends_with("-sys"))
            .collect();
        offenders.sort_unstable();
        offenders.dedup();

        assert!(
            offenders.is_empty(),
            "the DEFAULT build for {label} now reaches native code via {offenders:?}, which is \
             not that platform's TLS ({}). That breaks the guarantee that the ONNX/HF stack — \
             and any other native dependency — stays out of the default build (M2.5-R1). If it \
             is deliberate, it belongs in ARCHITECTURE → Supply-chain as a documented decision, \
             and in this target's allowance with the reason beside it, not in a green test.",
            target.map(|t| t.tls).unwrap_or("this host's own"),
        );
    }
}

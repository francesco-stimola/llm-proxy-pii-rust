//! **Algorithmic-complexity guards (M4-R19)** — detection must stay *linear* in the size
//! of the field it scans.
//!
//! The overlap rescan (M4-R17) resumes the regex one `char` past every match's **start**,
//! so a value hidden *inside* an earlier match of the same recognizer still becomes a
//! candidate. That probes O(n) start positions — harmless while a match is **bounded** in
//! length (a card is ≤ 19 digits, an ID ≤ 18 chars), but the two **unbounded** patterns —
//! Email (`local@domain`) and Secret (`sk-…`) — re-matched an O(n)-long value at every one
//! of them: **O(n²)**. A ~1 MB `content` field, trivially under the 16 MiB body limit,
//! pegged a core for *minutes* on an unauthenticated path (151 s at 200 KB).
//!
//! These are **timing** guards, so they run the work on a worker thread against a
//! wall-clock budget: a quadratic regression fails the suite in seconds instead of sitting
//! there for hours. Each one also asserts the value is still *masked and round-tripped* —
//! a "fix" that made detection fast by making it blind must fail here too.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::PiiDetector;

/// Wall-clock budget per case. Deliberately generous — `cargo test` builds unoptimized, and
/// the bar is *linear vs quadratic*, not a benchmark. It is still orders of magnitude below
/// the pre-fix behaviour: these inputs are ~1 MB, and 200 KB alone took 151 s.
const BUDGET: Duration = Duration::from_secs(10);

/// Run `work` on a worker thread and fail if it doesn't finish inside [`BUDGET`].
///
/// A plain `Instant::elapsed()` assertion would only fail *after* the quadratic scan
/// finished — which, on these inputs, is hours. Timing out is the point.
fn within_budget<T: Send + 'static>(label: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let started = Instant::now();
        let _ = tx.send((work(), started.elapsed()));
    });
    match rx.recv_timeout(BUDGET) {
        Ok((value, elapsed)) => {
            eprintln!("{label}: {elapsed:?}");
            value
        }
        Err(_) => panic!(
            "{label}: still running after {BUDGET:?} — the candidate scan is super-linear (M4-R19)"
        ),
    }
}

/// Detect + mask-to-a-fixpoint, the exact pair the request path runs.
fn detect_and_mask(input: String) -> (usize, String, bool) {
    let detector = StructuredRecognizers::new();
    let found = detector.detect(&input).len();
    let mut vault = Vault::new();
    let masked = vault
        .mask_all(&input, &detector)
        .expect("masking must succeed");
    let round_trips = vault.demask(&masked) == input;
    (found, masked, round_trips)
}

#[test]
fn a_huge_email_local_part_does_not_blow_up() {
    // DOS-01. The Email pattern is unbounded on both sides of the `@`, so the rescan used
    // to re-match a ~1 MB value from every one of its ~1 M start positions.
    let input = format!("{}@b.co", "a".repeat(1_000_000));
    let expected = input.clone();

    let (found, masked, round_trips) = within_budget("email", move || detect_and_mask(input));

    assert_eq!(
        found, 1,
        "the oversized email must still be detected, not skipped"
    );
    assert_eq!(
        masked, "[EMAIL_1]",
        "it must be masked whole, not left in clear"
    );
    assert!(round_trips, "the round-trip must stay exact");
    assert!(expected.ends_with("@b.co")); // the value really was the pathological one
}

#[test]
fn a_huge_run_of_secret_prefixes_does_not_blow_up() {
    // DOS-02. Secret's `sk-[A-Za-z0-9_-]{6,}` tail is unbounded too, and `sk-` re-anchors
    // at every ASCII word boundary inside the run — ~350 K start positions, each matching
    // the whole ~1 MB run.
    let input = "sk-".repeat(350_000);

    let (found, masked, round_trips) = within_budget("secret", move || detect_and_mask(input));

    assert_eq!(found, 1, "the secret run must still be detected");
    assert!(
        !masked.contains("sk-"),
        "no secret fragment may survive in clear: {masked}"
    );
    // The whole ~1 MB run collapsed into one placeholder. (It masks as `[SECRET_1]-`: the
    // input ends in `-`, and the trailing ASCII `\b` correctly leaves that dangling hyphen —
    // not PII — outside the match.)
    assert!(
        masked.len() < 32,
        "the run must be masked whole, not partially: {masked}"
    );
    assert!(round_trips, "the round-trip must stay exact");
}

#[test]
fn a_long_row_of_card_groups_stays_linear() {
    // DOS-03. The *bounded* recognizers keep the M4-R17 overlap rescan, so this pins the
    // other half of the claim: a card matches at every 4-digit group boundary — ~200 K
    // overlapping windows — yet each match is ≤ 19 chars, so the scan stays O(n · 19) and
    // the coalescing keeps the candidate set small. If the rescan is ever made unbounded
    // again, this is where it shows up.
    let input = "4111 1111 1111 1111 ".repeat(50_000);

    let (found, masked, round_trips) = within_budget("cards", move || detect_and_mask(input));

    assert!(found >= 1, "the cards must still be detected");
    assert!(
        !masked.contains("4111"),
        "no card digit group may survive in clear"
    );
    assert!(round_trips, "the round-trip must stay exact");
}

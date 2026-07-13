//! **Algorithmic-complexity guards (M4-R19, M4-R24)** — the masking path must stay *linear*
//! in **both** of its dimensions. Availability is a privacy property here: a proxy pegged
//! for minutes on an unauthenticated path protects nothing.
//!
//! **The two dimensions, and why it takes two kinds of test:**
//!
//! 1. **Field size *n*** (**DOS-01…03**, M4-R19). The overlap rescan (M4-R17) resumes the
//!    regex one `char` past every match's **start**, so a value hidden *inside* an earlier
//!    match still becomes a candidate. That probes O(n) start positions — harmless while a
//!    match is **bounded** (a card is ≤ 19 digits), but the two **unbounded** patterns,
//!    Email and Secret, re-matched an O(n)-long value at every one: **O(n²)**. 151 s on a
//!    200 KB field.
//! 2. **Entity count *k*** (**DOS-04**, M4-R24). `Vault::mask` spliced placeholders in
//!    right-to-left with `replace_range`, and every splice memmoves the whole tail: Θ(n·k).
//!    A field of many *small* values (`a@b.co `) has k growing with n, so that is **Θ(n²)**
//!    again — 13 MiB of repeated emails burned **~7 minutes**.
//!
//! **DOS-01…03 were blind to (2), and that is the lesson worth keeping.** Every one of them
//! pins a *single* entity (DOS-03's card row coalesces to k≈1), so they held n large and k
//! at one — and a per-entity quadratic lived right underneath them. *The complexity guards
//! must vary the entity **count**, not just the field **size***: it is the M4-R13 lesson
//! ("a test corpus has a shape, and that shape is a blind spot") recurring on the DoS guards
//! themselves. The smoking gun: 13.4 MiB as **one** email masks in 219 ms; the **same**
//! 13.4 MiB as many small emails took 421 s.
//!
//! These are **timing** guards, so they run the work on a worker thread against a wall-clock
//! budget: a quadratic regression fails the suite in seconds instead of sitting there for
//! hours. Each one also asserts the value is still *masked and round-tripped* — a "fix" that
//! bought speed with blindness must fail here too.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::PiiDetector;

/// Wall-clock budget per case. Deliberately generous — `cargo test` builds unoptimized, and
/// the bar is *linear vs quadratic*, not a benchmark. Every case sits orders of magnitude on
/// either side of it (measured, debug profile): DOS-04's splice, for instance, is **~0.2 s**
/// linear against **~52 s** quadratic.
const BUDGET: Duration = Duration::from_secs(10);

/// Run `work` on a worker thread and fail if it doesn't finish inside [`BUDGET`].
///
/// A plain `Instant::elapsed()` assertion would only fail *after* the quadratic work
/// finished — which, on these inputs, is minutes to hours. Timing out is the point.
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
            "{label}: still running after {BUDGET:?} — the masking path went super-linear \
             (M4-R19 / M4-R24)"
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

#[test]
fn masking_many_small_entities_stays_linear() {
    // DOS-04 (M4-R24) — the guard DOS-01…03 could not be: it varies the entity **count**.
    //
    // `Vault::mask` used to splice right-to-left with `String::replace_range`, and each
    // splice memmoves the entire tail of the string. With k entities in n bytes that shifts
    // Θ(n·k) bytes, and a field of many *small* values has k growing with n — so Θ(n²). A
    // 13 MiB body of repeated `a@b.co ` burned ~7 minutes of CPU on the unauthenticated
    // masking path. Detection being linear (DOS-01…03) does NOT bound this: the splice is a
    // separate cost, which is exactly why it survived the M4-R19 pass.
    //
    // We time the **splice alone**, because that is the code under test and it makes the
    // guard decisive rather than marginal: measured in the debug profile these tests run in,
    // 600 K entities splice in ~0.2 s linear against ~52 s quadratic (~250×). Detection and
    // de-masking are linear in n and are pinned elsewhere, so they stay outside the clock.
    let reps = 600_000;
    let input = "a@b.co ".repeat(reps); // ~4.2 MB, one entity every 7 bytes
    let expected = input.clone();

    let detector = StructuredRecognizers::new();
    let entities = detector.detect(&input);
    assert_eq!(
        entities.len(),
        reps,
        "the corpus must really carry one entity per repetition — otherwise this guard is \
         back to testing k=1 and is blind all over again"
    );

    let (vault, masked) = within_budget("many-entities-splice", move || {
        let mut vault = Vault::new();
        let masked = vault.mask(&input, &entities);
        (vault, masked)
    });

    // Fast is worthless if it stopped masking: every value is gone, and it all comes back.
    assert!(
        !masked.contains("a@b.co"),
        "an email survived masking in clear"
    );
    assert!(masked.contains("[EMAIL_1]"), "expected typed placeholders");
    assert_eq!(
        vault.demask(&masked),
        expected,
        "the round-trip must stay exact across {reps} entities"
    );
}

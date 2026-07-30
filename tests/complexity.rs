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
use llm_proxy_pii_rust::pipeline::privacy::PrivacyStage;
use llm_proxy_pii_rust::pipeline::{RequestContext, Stage};
use llm_proxy_pii_rust::proxy::ProxyRequest;

/// Digit groups the phone families match, **all distinct**: `offset` picks a stretch of the
/// sequence, so two calls with different offsets share no candidate.
///
/// **An odometer, not a modular hash, and the difference is a finding.** A `(i * 7) % 9000` style
/// generator looks distinct and silently starts repeating after its period — DOS-06's first draft did
/// exactly that, produced a 4 MiB body the per-scan memo absorbed, and reported *"the budget was
/// never reached"* as if that were the product's doing rather than the generator's. The digits below
/// enumerate `(b, c)` over 81 M combinations, so nothing recurs within any body this suite builds.
fn distinct_digit_groups(bytes: usize, offset: u64) -> String {
    let mut s = String::with_capacity(bytes + 64);
    let mut i = offset;
    while s.len() < bytes {
        i += 1;
        s.push_str(&format!(
            "row {:02} {:04} {:04} end ",
            10 + (i / 81_000_000) % 80,
            1000 + i % 9000,
            1000 + (i / 9000) % 9000
        ));
    }
    s
}

/// Wall-clock budget per case. Deliberately generous — `cargo test` builds unoptimized, and
/// the bar is *linear vs quadratic*, not a benchmark. Every case sits orders of magnitude on
/// either side of it (measured, debug profile): DOS-04's splice, for instance, is **~0.2 s**
/// linear against **~52 s** quadratic.
///
/// **30 s, not 10 s, and the reason is contention rather than the code (M10).** Measured serially in
/// the debug profile, the guarded cases run: splice 0.32 s · email 0.82 s · dense-real 0.89 s ·
/// secret 1.16 s · arbitrary-groups 1.41 s · distinct-groups 1.78 s · **cards 2.25–3.34 s**. At
/// 10 s the slowest of those had a 3× margin — and `cargo test --features onnx` runs every test
/// binary at once, so the ONNX NER cases saturate the cores and a 1.9 s case was observed crossing
/// 10 s. That is a **false red on a busy machine**, which is the worst failure mode a guard can
/// have: it teaches its reader to re-run rather than to look.
///
/// 30 s keeps the separation it is actually for. The linear side needs a **9×** slowdown to reach
/// it; the quadratic side is already past it when the machine is *idle* (52 s splice, 151 s for
/// DOS-01's 200 KB field). Widening past ~30 s would start to shelter DOS-04's real regression, so
/// this is not "make it big enough to never fail" — the ceiling is set by the fastest quadratic case
/// on record.
const BUDGET: Duration = Duration::from_secs(30);

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

/// **DOS-05 (M10-R2) — the third axis: *what the bytes are*.**
///
/// DOS-01…04 vary field **size** and entity **count**, and hold the **character class**
/// constant: `"a"*1M + "@b.co"`, `"sk-"*350_000`, `"4111 1111 1111 1111 "*50_000`,
/// `"a@b.co "*n`. **None of them produces a single phone candidate** — the card row's groups
/// are four digits, and the domestic-phone families need a 2–3-digit group at an ASCII word
/// boundary, which `4111` never offers. So M10's whole change was invisible to the DoS guard,
/// and a legal 12 MiB body of digit groups cost **105 s** of CPU on an unauthenticated path
/// while every guard stayed green.
///
/// *A quantity a test never varies is a quantity the test cannot see* — and here the
/// un-varied quantity was not size or count but the alphabet.
///
/// **Both cases here REPEAT a unit, and that is now known to be the weaker half — see DOS-06.**
///
/// Two shapes, because they stress different halves: a run of **real** numbers makes every
/// candidate an entity (validation *and* the splice), while a run of arbitrary digit groups
/// makes most candidates rejections — and a rejection is the expensive verdict, since the
/// region loop short-circuits only on *accept*.
///
/// Deliberately **not** asserted: that the arbitrary run masks nothing. Repeating any group
/// pattern produces rotations of itself, and with nine plans enabled some rotation is
/// usually somebody's real number (`22 45 12 33` is a valid Latvian one). That is the
/// documented over-mask, not a defect, and pinning it here would make this guard a precision
/// test that breaks whenever the region set changes.
#[test]
fn a_field_of_digit_groups_stays_affordable() {
    for (label, unit, expect_entities) in [
        // A real Italian number, repeated: every candidate is accepted and masked.
        ("dense-real-numbers", "06 69821234 ", true),
        // Arbitrary groups: mostly rejections, i.e. the worst case for the validator.
        ("arbitrary-digit-groups", "45 12 33 22 ", false),
    ] {
        // **200 KB, not the megabytes the other cases use, and the size is a measurement not
        // a guess.** `phonenumber::parse` is ~50× slower in the unoptimized profile `cargo
        // test` builds, so this case is debug-bound rather than algorithm-bound. At 200 KB
        // the guard still discriminates by a wide margin: post-fix ≈ 1.5–3 s here, while the
        // pre-fix validator cost ~1 s per 200 KB *in release*, i.e. tens of seconds here —
        // comfortably past the budget.
        let input = unit.repeat(200_000 / unit.len());
        let expected = input.clone();
        let (found, masked, round_trips) = within_budget(label, move || detect_and_mask(input));

        // Non-vacuity: the guard must be *reaching* the phone path, or it is measuring a
        // detector that does nothing — the exact way DOS-01…04 passed while blind to this.
        assert!(
            found > 5_000,
            "{label}: only {found} entities — this guard is not exercising the phone path"
        );
        if expect_entities {
            assert!(
                !masked.contains("06 69821234"),
                "{label}: a number survived"
            );
        }
        assert!(round_trips, "{label}: the round-trip must stay exact");
        assert_ne!(masked, expected, "{label}: nothing was masked at all");
    }
}

/// **DOS-06 (M10-R20) — the axis every one of this milestone's own measurements shared: the
/// input REPEATED.**
///
/// M10 bounded a ~100 s validation cost with a per-scan memo, and measured the fix on
/// `unit.repeat(n)` bodies — as does DOS-05, and as did every figure the milestone published.
/// A memo keyed on the matched bytes only helps candidates that **recur**, so all of those
/// numbers measured *the input's periodicity*, not the code. On a body whose candidates are
/// genuinely **distinct** the memo does nothing at all: same shape, same 4 MiB, varying only
/// the number of distinct candidates moved the cost **207 ms → 17,049 ms (82×)**, and a legal
/// 15 MiB body answered in **64.5 s** at the completely default configuration.
///
/// *A quantity a test never varies is a quantity the test cannot see* — M4-R24's lesson, and
/// then M10-R2's, arriving a third time on the same guard file. The un-varied quantity here was
/// not the field size, not the entity count, not even the alphabet, but **how often the same
/// bytes come round again**.
///
/// What bounds it is a fail-closed budget on validator calls per field, not a faster validator:
/// `phonenumber::parse().is_valid()` is ~6.5 µs per region however it is asked, and the one
/// attempt at a cheap pre-filter was a leak (M10-R13). So this asserts **both halves** — the
/// work stays inside the budget, and a body that exhausts it is *refused* rather than
/// forwarded with a partial scan.
#[test]
fn a_field_of_distinct_digit_groups_is_bounded_or_refused() {
    let distinct = |bytes: usize| distinct_digit_groups(bytes, 0);

    // **60 KB, sized for the unoptimized profile.** Every candidate here is a cache miss, so
    // the cost is the validator's ~6.5 µs per region — ~50× that in debug. The bar this case
    // carries is *correctness under the budget*; the bound itself is the second half.
    let input = distinct(60_000);
    let expected = input.clone();
    let (found, masked, round_trips) =
        within_budget("distinct-digit-groups", move || detect_and_mask(input));

    assert!(
        found > 500,
        "only {found} entities — this guard is not exercising the phone path"
    );
    assert!(round_trips, "the round-trip must stay exact");
    assert_ne!(masked, expected, "nothing was masked at all");

    // The other half: a field big enough to exhaust the validation budget must come back as an
    // **error**, not as a quietly partial scan. `detect()` is the infallible view and returns
    // nothing; `try_detect_within()` — the shape the request path uses — says why.
    //
    // **Against an explicit test allowance, not the shipped one.** Crossing the shipped 500,000
    // units costs ~25 s unoptimized, and what this half asserts is the *policy* — refuse rather
    // than truncate — which is not a claim about the number. Sizing the product's bound to fit the
    // test profile would be exactly backwards; so would leaving a 25 s case in the default suite
    // until someone marks it `#[ignore]`. DOS-BUD pins the real number on `--release`.
    let huge = distinct(200 * 1024);
    let detector = StructuredRecognizers::new();
    let budget = llm_proxy_pii_rust::pii::Budget::new(20_000);
    let err = detector.try_detect_within(&huge, &budget).expect_err(
        "a 200 KB field of distinct phone-shaped groups must exhaust a 20,000 unit budget and \
         be REFUSED — silently returning a partial scan is a miss, and a miss is a leak",
    );
    assert!(
        err.message.contains("budget"),
        "the error must say what happened, got {err:?}"
    );
    // Never log raw PII, not even in an error we control (DBG-02's rule applies here too).
    assert!(
        !err.message.contains("row "),
        "the error message must carry no input-derived text, got {err:?}"
    );
}

/// **DOS-07 (M10-R28) — the axis every complexity guard before it held constant: the number of
/// *fields*.**
///
/// DOS-01…03 vary field size, DOS-04 entity count, DOS-05 the alphabet, DOS-06 periodicity — and all
/// six measure **one string**, because `try_detect(&str)` is the shape they were handed. The masking
/// path takes a **body**. So M10-R20's fix bounded a field, the client kept choosing how many fields
/// it sent, and the same 15.6 MiB that is refused in one field answered `200` in 57 s across 78
/// legal `messages[].content` fields — indistinguishable from the binary with no budget at all.
///
/// This drives `PrivacyStage::on_request`, not `try_detect`, and every field here is **individually
/// far below** the allowance. That is the whole point: each one would pass alone, and the request
/// must still be refused, because the allowance belongs to the request.
///
/// It is deliberately the row from M10-R28's table where HEAD and the pre-fix build were
/// indistinguishable — if this ever passes by *forwarding*, the bound has gone back to being a rate.
#[test]
fn a_body_that_splits_digit_dense_text_across_many_fields_is_refused_as_one_request() {
    const FIELDS: usize = 20;
    const PER_FIELD: usize = 20 * 1024;
    /// A **test** allowance, not the shipped one. Each 20 KB field above spends ~4,900 units, so
    /// every field here is comfortably *under* this on its own and the twenty together are four
    /// times over it — which is exactly the shape of the finding. Crossing the shipped 500,000
    /// costs ~25 s unoptimized; the claim under test is the **unit** of the allowance, not its
    /// size, and DOS-BUD pins the size on `--release`.
    const TEST_BUDGET: usize = 20_000;

    let messages: Vec<serde_json::Value> = (0..FIELDS)
        .map(|i| {
            // A distinct stretch of the odometer per field, so no two fields share a candidate and
            // nothing is absorbed by a per-field memo. `PrivacyStage` here wraps the bare
            // recognizers (no `CachingDetector`), so identical fields would be charged anyway —
            // this is faithful to the finding's repro rather than convenient.
            serde_json::json!({
                "role": "user",
                "content": distinct_digit_groups(PER_FIELD, (i as u64) * 5_000_000),
            })
        })
        .collect();

    // **Non-vacuity first: one field alone must pass.** Without this the test could be green
    // because 20 KB is already over the allowance, which would prove nothing about the *request*
    // being the unit — the entire content of M10-R28.
    let one_field =
        PrivacyStage::with_validation_budget(Box::new(StructuredRecognizers::new()), TEST_BUDGET);
    let mut solo_ctx = RequestContext::new();
    let mut solo_req = ProxyRequest {
        body: serde_json::json!({
            "messages": [{ "role": "user", "content": distinct_digit_groups(PER_FIELD, 0) }]
        }),
    };
    one_field.on_request(&mut solo_req, &mut solo_ctx);
    assert!(
        solo_ctx.block.is_none(),
        "a single {PER_FIELD} byte field must be well under the allowance, or this guard proves \
         nothing about the request being the unit. Got: {:?}",
        solo_ctx.block
    );

    let stage =
        PrivacyStage::with_validation_budget(Box::new(StructuredRecognizers::new()), TEST_BUDGET);
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: serde_json::json!({ "messages": messages }),
    };
    stage.on_request(&mut req, &mut ctx);

    let reason = ctx.block.expect(
        "twenty fields that each pass on their own must be REFUSED together — if the request is \
         forwarded, the validation budget is scoped to a unit the client multiplies, which is a \
         rate and not a bound (M10-R28)",
    );
    assert!(
        reason.contains("budget"),
        "the refusal must say what was exhausted, got: {reason}"
    );
    // The message has to name the *request* as the unit, or it sends an agent to shrink one field
    // when the cost came from twenty (M10-R29 is what a confidently-misdirected refusal costs).
    assert!(
        reason.contains("per request"),
        "the refusal must name the unit the allowance actually has, got: {reason}"
    );
    // Never log raw PII, not even in an error we control — and this one is built from a request.
    assert!(
        !reason.contains("row "),
        "the refusal must carry no input-derived text, got: {reason}"
    );
}

/// **The sibling of CFG-01 that M10-R29 was missing.** CFG-01 pins that `PII_LOCALES=` means *no
/// region*; this pins what that has to **cost**: nothing.
///
/// The budget is sized for `phonenumber::parse()` at ~6.5 µs a call. It used to be decremented by
/// every recognizer that had a validator — including the nine always-on national-ID checksums, which
/// are arithmetic over ≤ 18 bytes. So 800 KB of bare 9-digit tokens was refused in 45 ms **with the
/// phone tier not loaded at all**: fifty thousand checksums, five milliseconds of real work, charged
/// as if it were a third of a second. The previous release masked and forwarded the same body.
///
/// An operator who set `PII_LOCALES=` in response to the `phonenumber` supply-chain advisory that
/// ARCHITECTURE documents was getting the phone tier's refusals without the phone tier.
#[test]
fn a_digit_dense_field_is_masked_not_refused_when_no_phone_region_is_enabled() {
    // 800 KB, the size that was a 400 before M10-R29 — distinct so nothing is memoized away.
    let mut field = String::with_capacity(800 * 1024 + 64);
    let mut i = 0u64;
    while field.len() < 800 * 1024 {
        i += 1;
        field.push_str(&format!("id={:09},", 100_000_000 + i % 800_000_000));
    }

    // The empty locale set: CFG-01's "none". No phone recognizer is constructed at all, so nothing
    // here may charge the phone budget.
    let detector = StructuredRecognizers::with_locales::<&str>(&[]);
    let stage = PrivacyStage::new(Box::new(detector));
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: serde_json::json!({ "messages": [{ "role": "user", "content": field }] }),
    };
    stage.on_request(&mut req, &mut ctx);

    assert!(
        ctx.block.is_none(),
        "with no phone region enabled, a digit-dense field must be masked and forwarded, not \
         refused — the phone budget may only be charged by the phone validator (M10-R29). Got: {:?}",
        ctx.block
    );
    // And it really was scanned: the always-on national-ID tier masks a good share of these
    // (M4-R6's mod-11 over-masks), so a silent empty pass would be the other way to fail.
    assert!(
        !ctx.vault.is_empty(),
        "the always-on national-ID recognizers must still have matched — an unrefused but \
         unscanned field would be the same leak wearing a 200"
    );
}

/// **DOS-BUD (M10-R30) — where the refusal line actually falls, on the shipped profile.**
///
/// `#[ignore]`d and run on demand:
/// `cargo test --release --test complexity -- --ignored --nocapture budget_refusal_line`
///
/// The published bound was quoted from a constant for three review rounds and was wrong in every
/// direction at once: stated per *field* when it was per *call*, at 0.5 s when a sub-budget field
/// measured 1.32 s, and about "phone-shaped" text when bare 9-digit tokens reached it too. The fix
/// for all three is that the number now comes from **here** — a measurement anyone can re-run —
/// rather than from prose next to the constant.
#[test]
#[ignore = "measurement, not a guard: run with --release to reproduce the published figures"]
fn budget_refusal_line_and_cost() {
    let detector = StructuredRecognizers::new();

    println!("\n--- one field, phone-shaped distinct groups (default region set) ---");
    println!(
        "{:>10}  {:>9}  {:>8}  {:>7}",
        "bytes", "verdict", "ms", "spent"
    );
    for kb in [128usize, 512, 1024, 2048, 3072, 4096, 8192] {
        let field = distinct_digit_groups(kb * 1024, 0);
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let started = Instant::now();
        let verdict = match detector.try_detect_within(&field, &budget) {
            Ok(found) => format!("{} spans", found.len()),
            Err(_) => "REFUSED".to_string(),
        };
        println!(
            "{:>10}  {:>9}  {:>8.0}  {:>7}",
            field.len(),
            verdict,
            started.elapsed().as_secs_f64() * 1000.0,
            budget.spent()
        );
    }

    println!("\n--- one request, the same total bytes split across N fields (M10-R28) ---");
    println!("{:>3}  {:>10}  {:>9}  {:>8}", "N", "total", "verdict", "ms");
    for fields in [1usize, 5, 20, 78] {
        let per = 200 * 1024;
        let messages: Vec<serde_json::Value> = (0..fields)
            .map(|i| {
                serde_json::json!({
                    "role": "user",
                    "content": distinct_digit_groups(per, (i as u64) * 5_000_000),
                })
            })
            .collect();
        let stage = PrivacyStage::new(Box::new(StructuredRecognizers::new()));
        let mut ctx = RequestContext::new();
        let mut req = ProxyRequest {
            body: serde_json::json!({ "messages": messages }),
        };
        let started = Instant::now();
        stage.on_request(&mut req, &mut ctx);
        println!(
            "{:>3}  {:>10}  {:>9}  {:>8.0}",
            fields,
            fields * per,
            if ctx.block.is_some() {
                "REFUSED"
            } else {
                "200"
            },
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    // **The case the threshold is actually chosen against**, and it is not the adversarial one: a
    // tool result from a database, where phone-shaped text is a *column* rather than the whole
    // payload. This is the body an agent produces by accident, so it is what "can real traffic reach
    // the budget?" has to be answered with — the question M10-R27 was raised to ask and M10-R30
    // found unanswerable from the doc comment alone.
    println!("\n--- a realistic SQL tool result: one phone column among ordinary columns ---");
    println!(
        "{:>10}  {:>6}  {:>9}  {:>8}  {:>7}",
        "bytes", "rows", "verdict", "ms", "spent"
    );
    for rows in [500usize, 5_000, 20_000, 50_000, 80_000] {
        let mut dump = String::from("id,customer,city,phone,email,total\n");
        for r in 0..rows as u64 {
            // **The phone column is an odometer too, and the first version of this was not.** A
            // `(r * 7) % 9000` column repeats after its period, and the per-scan memo then serves
            // the repeats for free: 20,000 rows measured the *same* 30,781 units as 10,000, which
            // reads as sub-linear cost when it is really a generator artefact. That is DOS-06's own
            // trap (M10-R20), reproduced here while measuring the fix for it — third time in this
            // milestone. Enumerating `(b, c)` keeps every row's number distinct.
            dump.push_str(&format!(
                "{},Customer Name {},Milano,3{:02} {:04} {:04},user{}@example.com,{}.50\n",
                10_000 + r,
                r,
                (r / 81_000_000) % 100,
                1000 + r % 9000,
                1000 + (r / 9000) % 9000,
                r,
                100 + r % 900
            ));
        }
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let started = Instant::now();
        let verdict = match detector.try_detect_within(&dump, &budget) {
            Ok(found) => format!("{} spans", found.len()),
            Err(_) => "REFUSED".to_string(),
        };
        println!(
            "{:>10}  {:>6}  {:>9}  {:>8.0}  {:>7}",
            dump.len(),
            rows,
            verdict,
            started.elapsed().as_secs_f64() * 1000.0,
            budget.spent()
        );
    }

    println!("\n--- the same body with the phone tier off, for the tier's share of the cost ---");
    let field = distinct_digit_groups(200 * 1024, 0);
    for (label, det) in [
        ("default (9 regions)", StructuredRecognizers::new()),
        (
            "PII_LOCALES= (none)",
            StructuredRecognizers::with_locales::<&str>(&[]),
        ),
    ] {
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let started = Instant::now();
        let verdict = match det.try_detect_within(&field, &budget) {
            Ok(found) => format!("{} spans", found.len()),
            Err(_) => "REFUSED".to_string(),
        };
        println!(
            "{label:>22}  {:>9}  {:>8.0} ms  spent {}",
            verdict,
            started.elapsed().as_secs_f64() * 1000.0,
            budget.spent()
        );
    }
    println!();
}

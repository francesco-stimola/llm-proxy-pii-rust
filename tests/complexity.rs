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
use llm_proxy_pii_rust::pii::cache::CachingDetector;
use llm_proxy_pii_rust::pii::composite::{CompositeDetector, FailOpen};
use llm_proxy_pii_rust::pii::recognizers::{
    StructuredRecognizers, MAX_PHONE_VALIDATIONS_PER_REQUEST,
};
use llm_proxy_pii_rust::pii::{Budget, PiiDetector};
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
        .mask_all(&input, &detector, &Budget::per_call())
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
    // nothing; `try_detect()` — the shape the request path uses — says why.
    //
    // **Against an explicit test allowance, not the shipped one.** Crossing the shipped 500,000
    // units costs ~25 s unoptimized, and what this half asserts is the *policy* — refuse rather
    // than truncate — which is not a claim about the number. Sizing the product's bound to fit the
    // test profile would be exactly backwards; so would leaving a 25 s case in the default suite
    // until someone marks it `#[ignore]`. DOS-BUD pins the real number on `--release`.
    let huge = distinct(200 * 1024);
    let detector = StructuredRecognizers::new();
    let budget = llm_proxy_pii_rust::pii::Budget::new(20_000);
    let err = detector.try_detect(&huge, &budget).expect_err(
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

/// **DOS-10 (M11-R18) — the alphabet every other case in this file holds constant: *letters
/// mixed with digits*, in lower case.**
///
/// DOS-01…03 vary field size, DOS-04 entity count, DOS-05 the alphabet — and its bytes are digit
/// groups; DOS-06 periodicity, DOS-07 field count. **Every one of them is digits, `sk-` runs or
/// `a@b.co`.** Not one produces a body of mixed alphanumeric groups, which is the only shape that
/// makes the IBAN pattern match at *every* position and reject at every position — and therefore
/// the only shape that runs `iban_case_gate` millions of times.
///
/// That gap is what M11-R18 is. The case fold (M11-R10) made a lowercase span matchable, the
/// shrink (M11-R13) made a rejected one retry at every interior separator, and `free()` made all
/// of it cost **nothing**: measured on 4 MiB of distinct lowercase `[a-z]{2}[0-9]{2}` groups,
/// **~0.7 million calls to the gate's arithmetic branch against a whole-request allowance of
/// 500 000 units, charged zero** — this guard's own generator measures 688 964 at 4 MiB. `MAX_PHONE_VALIDATIONS_PER_REQUEST` could not bound it — it is spent by
/// the phone validator alone — so the guarantee `ARCHITECTURE.md` derives from the budget did not
/// cover the term at all.
///
/// **So this guard asserts that the work is *charged*, not that it is fast.** A wall-clock
/// assertion here would be a claim about this box; `within_budget` already carries that job for
/// the rest of the file. What must never come back is the term being **invisible to the budget**,
/// and that is a property of the code: put `iban_case_gate` back inside `free()` and the
/// lowercase body's spend drops to **0** while the uppercase control's stays 0 too — the two
/// become indistinguishable, which is exactly the state M11-R18 found.
///
/// The uppercase control is the other half and it is doing real work: it is the same bytes with
/// the same candidate count, and it must stay near-free, because the gate short-circuits on a
/// span with no lowercase byte *without arithmetic*. A charge that fired on both would be a
/// charge on candidate count rather than on the expensive path, and it would refuse legal
/// uppercase bodies for nothing.
#[test]
fn a_field_of_lowercase_alphanumeric_groups_is_charged_for_the_case_gate() {
    /// `bytes` of distinct four-character `[a-z]{2}[0-9]{2}` groups, seven to a line.
    ///
    /// **Distinct on purpose** (DOS-06's lesson): the per-scan memo keys on the matched bytes, so
    /// a repeating body measures the generator's period rather than the code.
    fn alnum_groups(bytes: usize, upper: bool) -> String {
        let mut out = String::with_capacity(bytes + 64);
        let mut r = 0u64;
        while out.len() < bytes {
            for k in 0..7u32 {
                let a = (b'a' + ((r / 26u64.pow(k)) % 26) as u8) as char;
                let b = (b'a' + ((r / 26u64.pow(k + 1)) % 26) as u8) as char;
                let group = format!("{a}{b}{:02}", (r / (7 * (k as u64 + 1))) % 100);
                out.push_str(&if upper { group.to_uppercase() } else { group });
                out.push(' ');
            }
            out.push('\n');
            r += 1;
        }
        out
    }

    let detector = StructuredRecognizers::new();
    let spend = |body: &str| {
        let budget = llm_proxy_pii_rust::pii::Budget::new(usize::MAX / 2);
        detector
            .try_detect(body, &budget)
            .expect("an unlimited allowance must not be exhausted");
        budget.spent()
    };

    // Non-vacuity first: the body has to actually reach the IBAN recognizer, or a charge of zero
    // would mean "nothing to charge" rather than "not charged" (M4-R13).
    let lower = alnum_groups(200 * 1024, false);
    let upper = alnum_groups(200 * 1024, true);
    let detected = detector.detect(&upper).len();
    assert!(
        detected > 100,
        "only {detected} spans on the uppercase body — this guard is not reaching the IBAN \
         recognizer at all, so what it measures about charging is meaningless"
    );

    let lower_spend = spend(&lower);
    let upper_spend = spend(&upper);
    assert!(
        lower_spend > 10_000,
        "a 200 KB field of lowercase alphanumeric groups spent {lower_spend} validation units. \
         `iban_case_gate` runs its mod-97 arithmetic on every one of these candidates, once per \
         shrink retry, and this asserts the request is charged for it — M11-R18 is that work \
         being wrapped in `free()` and therefore bounded by nothing at all"
    );
    assert!(
        upper_spend * 10 < lower_spend,
        "the uppercase control spent {upper_spend} against {lower_spend} for the same bytes in \
         lower case. The gate short-circuits on a span with no lowercase byte without doing any \
         arithmetic, so a charge that fires on both is a charge on candidate count — it would \
         refuse legal uppercase bodies for work nobody did"
    );

    // And the charge is linear in the body, not quadratic: twice the bytes, about twice the units.
    let half = spend(&alnum_groups(100 * 1024, false));
    let ratio = lower_spend as f64 / half.max(1) as f64;
    assert!(
        (1.5..3.0).contains(&ratio),
        "doubling the field multiplied the charge by {ratio:.2} — the case-gate term must be \
         linear in the body, like every other term this file bounds"
    );
}

/// **DOS-11 (M11-R60) — the axis `DOS-10` does not have: an *ordinary* body must stay **under** the
/// allowance.**
///
/// `DOS-10` asserts that the case-gate term is **charged**. Nothing asserted that charging it
/// leaves legal traffic alone, and the cost of that gap was a real refusal: at one unit per
/// arithmetic call an ordinary `xxd` hex dump was blocked with a `400` at **8 MiB**, having been
/// masked and forwarded at 12 MiB by the same build with the charge removed. `PHONE-BUD` is this guard for the M7 turn and
/// for nothing else — and the M7 turn is instruction prose, which spends zero units and therefore
/// cannot see a term that only digit-dense text reaches.
///
/// **The fixture is `xxd`, and the reason is the whole finding.** The sample published beside the
/// decision was `abcd 1234 …` — a pure-letter group beside a pure-digit one, which cannot spell
/// `[A-Za-z]{2}\d{2}` and measures **0 units**. Real hex output is *uniform* hex per group, where
/// P(two letters then two digits) = (6/16)² · (10/16)² ≈ 5.5% of groups. So this asserts its own
/// fixture can spell the shape it is a sample of, **before** it asserts anything about cost:
/// *a sample that cannot spell the shape it samples measures the sample* (M4-R13, on the cost axis).
///
/// **Scaled rather than run at `MAX_BODY_BYTES`.** A 16 MiB body is ~80× this one in the debug
/// profile, and a slow guard is a guard somebody eventually marks `#[ignore]` — which in this
/// milestone alone has hidden three findings. The extrapolation is legitimate *because* `DOS-10`
/// asserts the same term is linear in the body; what this adds is the constant.
#[test]
fn an_ordinary_hex_dump_stays_inside_the_request_allowance_at_max_body_bytes() {
    /// `bytes` of `xxd`-format output: an offset column, eight groups of four **uniform** lowercase
    /// hex digits, and the ASCII gutter.
    fn xxd(bytes: usize) -> String {
        let mut out = String::with_capacity(bytes + 96);
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };
        let mut offset = 0u64;
        while out.len() < bytes {
            out.push_str(&format!("{offset:08x}: "));
            for _ in 0..8 {
                out.push_str(&format!("{:04x} ", next() % 0x1_0000));
            }
            out.push_str(" ................\n");
            offset += 16;
        }
        out
    }

    const SAMPLE: usize = 512 * 1024;
    let body = xxd(SAMPLE);

    // **Non-vacuity comes first, and it is the assertion the published sample failed.** A body that
    // cannot produce an IBAN candidate would pass every cost bar below while measuring nothing.
    let groups = regex::Regex::new(r"(?-u:\b)[0-9a-f]{4}(?-u:\b)")
        .unwrap()
        .find_iter(&body)
        .count();
    let ibanish = regex::Regex::new(r"(?-u:\b)[a-f]{2}[0-9]{2}(?-u:\b)")
        .unwrap()
        .find_iter(&body)
        .count();
    assert!(
        groups > 10_000 && ibanish * 100 / groups >= 3,
        "the fixture holds {ibanish} `[a-f]{{2}}[0-9]{{2}}` groups out of {groups} hex groups. \
         Uniform hex puts that near 5.5%; below 3% this is not a hex dump any more and the cost \
         bar below would pass for the wrong reason — which is exactly how M11-R60 got published."
    );

    let detector = StructuredRecognizers::new();
    let budget = llm_proxy_pii_rust::pii::Budget::new(usize::MAX / 2);
    let mut vault = Vault::new();
    vault
        .mask_all(&body, &detector, &budget)
        .expect("an unlimited allowance must not be exhausted");
    let spent = budget.spent();
    let gate = budget.spent_on(llm_proxy_pii_rust::pii::PiiKind::Iban);
    let phone = budget.spent_on(llm_proxy_pii_rust::pii::PiiKind::Phone);
    println!("DOS-11: {SAMPLE} bytes of xxd spend {spent} units — Iban {gate}, Phone {phone}");

    // **The invariant, stated as M10-R29 states it: a validator cheaper than a `parse()` must not
    // *dominate* the allowance on ordinary traffic.** That is what makes over-pricing visible, and
    // it is a ratio rather than a wall clock or an absolute count — so it says the same thing on
    // any box and at any body size. At one full unit per call (M11-R60) the gate outspent the
    // whole phone tier on this fixture; at the measured price it is a minority share.
    // **Both spenders must actually be charged, and this closes two holes at once (M11-R63).**
    // The ratio below is satisfied by `0 <= n` — so attribution recording *nothing* passed it, and
    // so did aiming both call sites at a constant `PiiKind::Phone`, which drives `gate` to zero
    // while `E2E-05`'s single-spender fixture cannot tell the difference. It is a property of the
    // **attribution** rather than of a kind, so a third spender inherits it: this fixture reaches
    // two validators, and the budget has to say so.
    assert!(
        gate > 0 && phone > 0,
        "the hex fixture reaches both the IBAN case gate and the phone tier, but the budget \
         attributes {gate} to Iban and {phone} to Phone. Either a spender is charging nothing, or \
         `Budget::attribute` is not following the recognizer that spent — and the refusal message \
         is generated from exactly this, so it would name the wrong tier with the suite green."
    );

    // **The bar is derived from this fixture, and the bar travels with it.** The gate-to-phone
    // ratio is *not* scale-free — the per-scan memo saturates the two terms differently, so on
    // this 512 KiB body it is **0.349** at the shipped price and **0.697** at the one-full-unit
    // price M11-R60 found, while at 4 MiB the same two prices measure 0.556 and 1.11. A bar of
    // one half separates them here with a factor of two either way; on a different fixture size
    // it would mean something else, which is why `SAMPLE` is a constant beside it and not an
    // argument. The **absolute** refusal line is `DOS-BUD`'s job, and it is a declared pre-tag
    // measurement rather than a `cargo test` assertion, because it needs `--release` and
    // multi-megabyte bodies.
    assert!(
        gate * 2 <= phone,
        "on an ordinary hex dump the IBAN case gate was charged {gate} units against the phone \
         tier's {phone} on this {SAMPLE}-byte fixture — over half, where the shipped price \
         measures a third. Per *call* the gate costs about a third of what a `parse()` does, so a \
         share this size means it is priced above what it costs, and over-pricing cheap work does \
         not make the bound safer — it refuses legal traffic (M10-R29, M11-R60). \
         `IBAN_GATE_CALLS_PER_UNIT` is bounded by two guards, this one below it and `DOS-10`'s \
         charge floor above it; re-derive it against that band and against `DOS-BUD`'s **µs per \
         call** column, never its µs/unit, which is denominated in the units the constant itself \
         defines (M11-R64)."
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
        let verdict = match detector.try_detect(&field, &budget) {
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
    //
    // **In BOTH renderings, because one is not a measurement of anything but itself (M10-R56/R58).**
    // The refusal line for a legal body lives here, and for three rounds this sweep ran only the
    // cheapest rendering — so the published band was an off-harness number nothing re-ran. A column
    // of `347 XXXXXXX` costs ~1 unit per row; the same numbers written `3XX XXX XXXX` cost ~8, and
    // that factor is the whole difference between "multi-megabyte, rare" and "sub-megabyte".
    println!("\n--- a realistic SQL tool result: one phone column, both renderings ---");
    println!(
        "{:>22}  {:>10}  {:>7}  {:>12}  {:>8}  {:>7}",
        "rendering", "bytes", "rows", "verdict", "ms", "spent"
    );
    for (label, grouped) in [("347 XXXXXXX", false), ("3XX XXX XXXX", true)] {
        for rows in [5_000usize, 20_000, 50_000, 62_500, 100_000, 250_000] {
            let mut dump = String::from("id,customer,city,phone,email,total\n");
            for r in 0..rows as u64 {
                // Odometers in both renderings: a modular column repeats and the per-scan memo then
                // serves the repeats for free, which reads as sub-linear cost when it is a generator
                // artefact. That trap has now produced a published number twice (M10-R49, M10-R56).
                let phone = if grouped {
                    format!(
                        "3{:02} {:03} {:04}",
                        20 + (r / 10_000_000) % 80,
                        (r / 10_000) % 1000,
                        r % 10_000
                    )
                } else {
                    format!("347 {:07}", 1_000_000 + r % 9_000_000)
                };
                dump.push_str(&format!(
                    "{},Customer Name {},Milano,{},user{}@example.com,{}.50\n",
                    10_000 + r,
                    r,
                    phone,
                    r,
                    100 + r % 900
                ));
            }
            let budget = llm_proxy_pii_rust::pii::Budget::new(
                llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
            );
            // Through `mask_all`, not `try_detect` — the whole fixpoint, which is what a request
            // pays (M10-R35). Measuring one pass is how the published figure came to be per-pass
            // while claiming to be per-field.
            let started = Instant::now();
            let mut vault = Vault::new();
            // Report how many phones were **masked**, not how many were left: "0 left" is what a
            // correctly-masked column and an entirely unmatched one both print (M10-R49).
            let verdict = match vault.mask_all(&dump, &detector, &budget) {
                Ok(masked) => format!("{} masked", masked.matches("[PHONE_").count()),
                Err(_) => "REFUSED".to_string(),
            };
            println!(
                "{label:>22}  {:>10}  {:>7}  {:>12}  {:>8.0}  {:>7}",
                dump.len(),
                rows,
                verdict,
                started.elapsed().as_secs_f64() * 1000.0,
                budget.spent()
            );
        }
    }

    // **The axis this grid held constant for four rounds: the VERDICT (M11-R39), and the axis
    // *that* fix still held constant: the candidate's SHAPE (M11-R38).** Every row in the grids
    // above is built from *valid* numbers, so `applicable.iter().any(..)` short-circuits on the
    // first region that accepts and only the cheapest branch is ever sampled. This file's own text
    // says a rejection is the expensive verdict — a rejected candidate is charged once per enabled
    // region, up to nine — and the published `~3 µs` unit was read off the accepting corner anyway.
    //
    // **So this grid prints µs/unit, not only ms/row**, because the per-row figure cannot separate
    // "more units" from "dearer units" and the whole of M11-R38 lives in that difference.
    // Measured on this box, `--release`, and the shape of the answer is the point:
    //
    // - The **verdict** moves the *unit count* and the budget already tracks it: a rejected
    //   candidate pays all nine regions, an accepted one stops at the first that says yes.
    // - The **shape** moves the *price of a unit*, and nothing tracks that: a three-group
    //   trunk-anchored candidate (`0NNN NNNN NNNN`, a zero-padded key column with three fields)
    //   measures several times the µs/unit of every other row here. That is the factor
    //   `ARCHITECTURE.md` publishes as a band rather than as a constant.
    //
    // A grid that varies size, rendering and layout but never the verdict cannot see the first;
    // one that varies the verdict but not the group count cannot see the second. That is DOS-05's
    // lesson — *the axis a test never varies is the one it cannot see* — twice on the same grid.
    // **A µs-per-*call* column beside µs-per-unit, and it is not a convenience (M11-R64).** The
    // unit column is `ms / spent`, and `spent` is denominated in the very units
    // `IBAN_GATE_CALLS_PER_UNIT` defines — so halving the gate's charge doubles its µs/unit, and a
    // reader re-deriving the constant from that column feeds the recipe its own output and lands
    // back on the value M11-R60 was. The call column divides it back out, so the number the
    // derivation asks for is the number the grid prints. *A measurement whose denominator is a
    // function of the thing being measured is not a measurement of it.*
    println!("\n--- the same column, by VERDICT and by SHAPE: what a unit costs (M11-R38/R39) ---");
    println!(
        "{:>24}  {:>10}  {:>7}  {:>12}  {:>8}  {:>9}  {:>9}  {:>9}",
        "shape", "bytes", "rows", "verdict", "ms", "spent", "us/unit", "us/call"
    );
    for (label, kind) in [
        ("3XX XXX XXXX (accept)", 0),
        ("0NNNN NNNN (reject)", 1),
        ("0NNN NNNN NNNN (3 grp)", 2),
        ("ab12 cd34 .. (lowercase)", 3),
    ] {
        for rows in [5_000usize, 20_000, 50_000] {
            let mut dump = String::from("id,customer,city,ref,email,total\n");
            for r in 0..rows as u64 {
                let token = match kind {
                    0 => format!(
                        "3{:02} {:03} {:04}",
                        20 + (r / 10_000_000) % 80,
                        (r / 10_000) % 1000,
                        r % 10_000
                    ),
                    // A zero-padded key column — `LPAD`-ed ids, which every ORM emits. Phone-shaped
                    // enough to be a candidate in every enabled region, and the **accept rate is
                    // what the run reports** rather than what this comment asserts: the first
                    // version of this row claimed "rejected in all of them" and measured 4 145 of
                    // 5 000 *masked*, on an odometer that repeated every 10 000 rows so the memo
                    // served most of it for free. Distinct now, and labelled by shape.
                    1 => format!("0{:04} {:04}", (r / 10_000) % 10_000, r % 10_000),
                    // The same key column with a third field — the shape M11-R38 measured and the
                    // one that moves the *price* of a unit rather than their number.
                    2 => format!(
                        "0{:03} {:04} {:04}",
                        r % 1000,
                        (r / 1000) % 10_000,
                        r % 10_000
                    ),
                    // Lowercase alphanumeric groups: the IBAN case gate's shape (M11-R18), charged
                    // since this milestone and previously `free()`. Not a phone shape at all, which
                    // is the point — it reaches the allowance through a different validator.
                    _ => {
                        let g = |k: u64| {
                            let a = (b'a' + ((r / 26u64.pow(k as u32)) % 26) as u8) as char;
                            let b = (b'a' + ((r / 26u64.pow(k as u32 + 1)) % 26) as u8) as char;
                            format!("{a}{b}{:02}", (r / (7 * (k + 1))) % 100)
                        };
                        (0..5).map(g).collect::<Vec<_>>().join(" ")
                    }
                };
                dump.push_str(&format!(
                    "{},Customer Name {},Milano,{},user{}@example.com,{}.50\n",
                    10_000 + r,
                    r,
                    token,
                    r,
                    100 + r % 900
                ));
            }
            let budget = llm_proxy_pii_rust::pii::Budget::new(
                llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
            );
            let started = Instant::now();
            let mut vault = Vault::new();
            let verdict = match vault.mask_all(&dump, &detector, &budget) {
                Ok(masked) => format!("{} masked", masked.matches("[PHONE_").count()),
                Err(_) => "REFUSED".to_string(),
            };
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let spent = budget.spent();
            // Only the lowercase row is charged fractionally, so only it has a call price that
            // differs from its unit price; every other row divides by one.
            let per_unit = if spent == 0 {
                0.0
            } else {
                ms * 1000.0 / spent as f64
            };
            let calls_per_unit = if kind == 3 {
                llm_proxy_pii_rust::pii::recognizers::IBAN_GATE_CALLS_PER_UNIT as f64
            } else {
                1.0
            };
            println!(
                "{label:>24}  {:>10}  {:>7}  {:>12}  {:>8.0}  {:>9}  {:>9.2}  {:>9.2}",
                dump.len(),
                rows,
                verdict,
                ms,
                spent,
                per_unit,
                per_unit / calls_per_unit
            );
        }
    }

    // **Where an *ordinary* body stops being accepted, by size (M11-R60).** Every other grid here
    // asks how much a shape costs; this one asks the question a refusal is actually about — *how
    // big can a legal payload be?* — on the one shape that reaches both spenders at once. `xxd`,
    // `od -x` and every debugger emit uniform hex per group, so ~5.5% of groups spell
    // `[A-Za-z]{2}\d{2}` and the IBAN case gate runs on them; the digit-heavy remainder feeds the
    // phone tier. `DOS-11` pins the *ratio* between the two on every `cargo test`; the **line**
    // needs `--release` and multi-megabyte bodies, so it lives here and is re-run before a tag.
    //
    // The per-kind split is what makes this readable rather than a single number that moved: it
    // says which spender is responsible at each size, which is the same information the refusal
    // message now carries to the client.
    println!("\n--- an ordinary xxd hex dump: where the refusal line falls, by size (M11-R60) ---");
    println!(
        "{:>10}  {:>12}  {:>9}  {:>9}  {:>9}",
        "body", "verdict", "spent", "of which Iban", "Phone"
    );
    for mb in [4usize, 8, 10, 12, 16] {
        let mut dump = String::with_capacity(mb * 1024 * 1024 + 128);
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut offset = 0u64;
        while dump.len() < mb * 1024 * 1024 {
            dump.push_str(&format!("{offset:08x}: "));
            for _ in 0..8 {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                dump.push_str(&format!("{:04x} ", (state >> 33) % 0x1_0000));
            }
            dump.push_str(" ................\n");
            offset += 16;
        }
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let mut vault = Vault::new();
        let verdict = match vault.mask_all(&dump, &detector, &budget) {
            Ok(_) => "masked",
            Err(_) => "REFUSED",
        };
        println!(
            "{:>8} MiB  {verdict:>12}  {:>9}  {:>13}  {:>9}",
            mb,
            budget.spent(),
            budget.spent_on(llm_proxy_pii_rust::pii::PiiKind::Iban),
            budget.spent_on(llm_proxy_pii_rust::pii::PiiKind::Phone)
        );
    }

    // **The allowance is a count of numbers, not a count of bytes — and publishing it in bytes is
    // what went wrong four times (M10-R56).** At a given rendering the cost per phone number is
    // flat, so the *row* limit is fixed; the *byte* limit is whatever surrounds the column. Three
    // layouts at the grouped rendering, all at the same 62,500 rows, show the same refusal at three
    // very different sizes — which is why the band is published as numbers with a density note
    // rather than as a megabyte figure.
    println!("\n--- one limit, three layouts: the refusal line is rows, not bytes ---");
    println!(
        "{:>26}  {:>10}  {:>7}  {:>12}  {:>7}",
        "layout", "bytes", "rows", "verdict", "spent"
    );
    for (layout, rows) in [
        ("phone only", 62_500usize),
        ("phone only (one row less)", 62_499),
        ("name,phone", 62_500),
        ("6-column export", 62_500),
    ] {
        let mut dump = String::new();
        for r in 0..rows as u64 {
            let phone = format!(
                "3{:02} {:03} {:04}",
                20 + (r / 10_000_000) % 80,
                (r / 10_000) % 1000,
                r % 10_000
            );
            match layout {
                "name,phone" => dump.push_str(&format!("Customer Name {r},{phone}\n")),
                "6-column export" => dump.push_str(&format!(
                    "{},Customer Name {},Milano,{},user{}@example.com,{}.50\n",
                    10_000 + r,
                    r,
                    phone,
                    r,
                    100 + r % 900
                )),
                _ => dump.push_str(&format!("{phone}\n")),
            }
        }
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let mut vault = Vault::new();
        let verdict = match vault.mask_all(&dump, &detector, &budget) {
            Ok(masked) => format!("{} masked", masked.matches("[PHONE_").count()),
            Err(_) => "REFUSED".to_string(),
        };
        println!(
            "{layout:>26}  {:>10}  {:>7}  {:>12}  {:>7}",
            dump.len(),
            rows,
            verdict,
            budget.spent()
        );
    }
    // **Per-candidate cost is not per-row cost, and only the second one answers the question
    // (M10-R57).** In isolation an FR pair-separated number costs up to 46 units; in a *column* of
    // them the per-scan memo absorbs the repeated sub-candidate prefixes and the marginal cost
    // collapses. The refusal line a real payload meets is the column figure, so that is what the band
    // has to be published from — the isolation table above measures how a rendering behaves, not what
    // a body costs.
    println!("\n--- cost per row in a COLUMN of 20,000, by rendering ---");
    println!(
        "{:>26}  {:>8}  {:>10}  {:>12}",
        "column of", "spent", "units/row", "verdict"
    );
    for (label, make) in [
        (
            "347 XXXXXXX (IT LongBlock)",
            (|r: u64| format!("347 {:07}", 1_000_000 + r % 9_000_000)) as fn(u64) -> String,
        ),
        ("3XX XXX XXXX (IT grouped)", |r: u64| {
            format!(
                "3{:02} {:03} {:04}",
                20 + (r / 10_000_000) % 80,
                (r / 10_000) % 1000,
                r % 10_000
            )
        }),
        ("0X XX XX XX XX (FR pairs)", |r: u64| {
            format!(
                "0{} {:02} {:02} {:02} {:02}",
                1 + (r / 100_000_000) % 9,
                (r / 1_000_000) % 100,
                (r / 10_000) % 100,
                (r / 100) % 100,
                r % 100
            )
        }),
        ("6XX XX XX XX (ES grouped)", |r: u64| {
            format!(
                "6{:02} {:02} {:02} {:02}",
                10 + (r / 1_000_000) % 90,
                (r / 10_000) % 100,
                (r / 100) % 100,
                r % 100
            )
        }),
        ("+39 3XX XXXXXXX (+CC)", |r: u64| {
            format!("+39 3{:02} {:07}", 20 + r % 80, 1_000_000 + r % 9_000_000)
        }),
    ] {
        let mut dump = String::new();
        for r in 0..20_000u64 {
            dump.push_str(&make(r));
            dump.push('\n');
        }
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let mut vault = Vault::new();
        let verdict = match vault.mask_all(&dump, &detector, &budget) {
            Ok(masked) => format!("{} masked", masked.matches("[PHONE_").count()),
            Err(_) => "REFUSED".to_string(),
        };
        println!(
            "{label:>26}  {:>8}  {:>10.2}  {:>12}",
            budget.spent(),
            budget.spent() as f64 / 20_000.0,
            verdict
        );
    }
    // **The axis this harness held constant for three rounds: how the number is *written* (M10-R54).**
    //
    // Rows and bytes were varied; rendering, region and density were not — so every conclusion drawn
    // from the table was a fact about `347 XXXXXXX`, which turns out to be the **global minimum**:
    // `LongBlock` is the only single-region family and the only shape no other family's regex matches
    // inside. `Scan::Overlapping` resumes one `char` past each match's *start*, so any other rendering
    // also proposes sub-candidates from inside itself, each rejected and each paying its family's
    // whole region list. That is a 1-vs-29 spread, and on it rests the answer to *"can real traffic
    // reach the allowance?"* — which two consecutive rounds published wrongly, in the reassuring
    // direction, from one point of this grid (M10-R49, M10-R53).
    println!("\n--- the same number, written every legal way: units for one candidate ---");
    println!("{:>18}  {:>6}  note", "rendering", "units");
    for (number, note) in [
        ("347 1234567", "IT mobile, LongBlock — the global minimum"),
        ("030 12345678", "DE"),
        ("011 5627111", "IT landline"),
        ("020 7946 0958", "GB"),
        ("010 12345678", "CN landline"),
        ("912 345 678", "ES"),
        ("020 123 4567", "NL"),
        ("138 0013 8000", "CN mobile"),
        ("320 123 4567", "IT mobile, grouped — same country as row 1"),
        ("210 123 456", "PT"),
        ("612 34 56 78", "ES grouped"),
        ("67 22 33 44", "LV"),
        ("01 23 45 67 89", "FR TrunkPairs — the maximum"),
    ] {
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let _ = detector.try_detect(&format!("call {number} now"), &budget);
        println!("{number:>18}  {:>6}  {note}", budget.spent());
    }

    // **The question the whole harness exists for: can a *legal* phone-bearing body reach the
    // allowance?** `MAX_BODY_BYTES` caps a request at 16 MiB, so this asks the SQL shape at exactly
    // that size — in **two** renderings, because one is not a measurement of anything but itself.
    println!("\n--- the same SQL shape at MAX_BODY_BYTES, in two legal renderings ---");
    for (label, grouped) in [
        ("347 XXXXXXX (cheapest)", false),
        ("3XX XXX XXXX (grouped)", true),
    ] {
        let mut dump = String::from("id,customer,city,phone,email,total\n");
        let mut r = 0u64;
        while dump.len() < 16 * 1024 * 1024 {
            let phone = if grouped {
                // The same Italian mobiles, written the way an export usually writes them.
                //
                // **An odometer over `(a, b, c)`, and the first draft was `(20 + r%80, r%1000,
                // r%10000)` — period 10,000 rows.** The per-scan memo then served 95% of a 16 MiB
                // dump for free and the grouped rendering measured *cheaper* than the cheapest one,
                // which is the opposite of the finding this row exists to show. **Sixth appearance
                // of this trap in M10** (DOS-05, DOS-06's draft, PHONE-NAT-10, DOS-BUD's SQL column,
                // DOS-09's draft, here). It is not carelessness — it is that a modular generator
                // *looks* varied at every call site, and only the aggregate shows it is not.
                format!(
                    "3{:02} {:03} {:04}",
                    20 + (r / 10_000_000) % 80,
                    (r / 10_000) % 1000,
                    r % 10_000
                )
            } else {
                format!("347 {:07}", 1_000_000 + r % 9_000_000)
            };
            dump.push_str(&format!(
                "{},Customer Name {},Milano,{},user{}@example.com,{}.50\n",
                10_000 + r,
                r,
                phone,
                r,
                100 + r % 900
            ));
            r += 1;
        }
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let mut vault = Vault::new();
        let started = Instant::now();
        let verdict = match vault.mask_all(&dump, &detector, &budget) {
            Ok(masked) => format!("{} masked", masked.matches("[PHONE_").count()),
            Err(_) => "REFUSED".to_string(),
        };
        println!(
            "{label:>24}  {:>9} bytes  {:>7} rows  {:>14}  {:>7.0} ms  spent {} of {}",
            dump.len(),
            r,
            verdict,
            started.elapsed().as_secs_f64() * 1000.0,
            budget.spent(),
            budget.initial()
        );
    }

    // **What the budget does NOT bound, measured rather than inferred.** The allowance caps
    // *validator* calls. Regex scanning and the mask rewrite are linear in the body and bounded only
    // by `MAX_BODY_BYTES`, so the per-request CPU ceiling is `validation ≤ budget × per-call` **plus**
    // that floor — and the floor is what a refused 6 MB body spends before it is refused. Publishing
    // the validation term alone as "the ceiling" is the M10-R30 mistake in a new place, so this row
    // exists to make the other term a number too.
    println!("\n--- the unbudgeted linear floor: MAX_BODY_BYTES with the phone tier OFF ---");
    {
        let none = StructuredRecognizers::with_locales::<&str>(&[]);
        let body = distinct_digit_groups(16 * 1024 * 1024, 0);
        let budget = llm_proxy_pii_rust::pii::Budget::new(
            llm_proxy_pii_rust::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        );
        let mut vault = Vault::new();
        let started = Instant::now();
        let outcome = vault.mask_all(&body, &none, &budget);
        println!(
            "{:>10} bytes  {:>9}  {:>8.0} ms  spent {}",
            body.len(),
            if outcome.is_ok() { "200" } else { "REFUSED" },
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
        let verdict = match det.try_detect(&field, &budget) {
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

/// Every **wrapper position** the detector chain can put between `PrivacyStage` and the recognizers,
/// so a guard about the chain is not secretly a guard about one arrangement (M10-R42 / M10-R48).
///
/// `build_detector` composes `CachingDetector` (optional, `PII_CACHE_ENTRIES`) over
/// `CompositeDetector` over the recognizers, and wraps a non-required NER in `FailOpen`. The list
/// below is the cross-product of those positions plus the bare leaf.
///
/// **It is a hand-written list, and that is a known weakness rather than an oversight (M10-R48).**
/// Nothing ties it to `AppState::new`, so a seventh wrapper added to the wiring does not appear here
/// by itself — the same shape as the finding this function exists to close, one level up. It cannot
/// be derived today (`build_detector` takes a `Config` and returns an opaque `Box<dyn PiiDetector>`,
/// with no way to enumerate what it built), so the honest mitigation is to name the gap here and to
/// keep the list in the same file as the wiring's own review notes. **`FailOpen` was the position it
/// omitted**, and it is the one where the line that makes a budget refusal un-swallowable lives —
/// see `FAILOPEN-BUD` in `pii::composite`, which asserts that line directly.
fn shipped_chains() -> Vec<(&'static str, Box<dyn PiiDetector>)> {
    fn leaf() -> Box<dyn PiiDetector> {
        Box::new(StructuredRecognizers::new())
    }
    fn composed() -> Box<dyn PiiDetector> {
        Box::new(CompositeDetector::new(vec![leaf()]))
    }
    // The recognizers are never `FailOpen`-wrapped by today's wiring — the NER is. Including the
    // shape anyway is the point: "not reachable today" is what round 4 concluded about the fail-open
    // question, and the reason it was true turned out to be the defect (M10-R41).
    fn fail_open_leaf() -> Box<dyn PiiDetector> {
        Box::new(FailOpen(leaf()))
    }
    vec![
        ("bare", leaf()),
        ("composed", composed()),
        ("cached", Box::new(CachingDetector::new(leaf(), 16))),
        (
            "cached+composed",
            Box::new(CachingDetector::new(composed(), 16)),
        ),
        ("fail-open", fail_open_leaf()),
        (
            "composed(fail-open)",
            Box::new(CompositeDetector::new(vec![fail_open_leaf()])),
        ),
        (
            "cached+composed(fail-open)",
            Box::new(CachingDetector::new(
                Box::new(CompositeDetector::new(vec![fail_open_leaf()])),
                16,
            )),
        ),
    ]
}

/// **DOS-08 (M10-R35, widened by M10-R42) — every fixpoint pass is charged to the request, through
/// every shape the detector chain takes.**
///
/// M10-R28 made the allowance per-request and M10-R30 named the fixpoint as the other half of why the
/// old bound did not exist: `Vault::mask_all` calls detection up to five times per field. The fix
/// threaded a budget through a `_within` seam whose *default* silently minted a fresh allowance, and
/// `StructuredRecognizers` — the leaf — overrode one of the two methods and not the other. A legal
/// 15.63 MiB body answered `200` in 17.2 s, and **not one test could see it**.
///
/// **The first version of this guard could not see it either, one level up.** It constructed a bare
/// `StructuredRecognizers` and was described in three documents as *"phrased over the trait, so a
/// seventh detector that drops the allowance fails here"*. What was phrased over the trait were the
/// *method names it called*; what it quantified over was **one concrete type — the one already
/// fixed**. Reintroducing the identical defect in `CachingDetector::redetect`, a shipped wrapper on
/// the default request path, left all 218 tests green while the request under-charged **21x**
/// (205,065 -> 9,765 units) and forwarded a body it must refuse. *A guard aimed at the instance
/// relocates the blind spot* — M4-R7 -> R9's "a fix that only re-ranks relocates the leak", in its
/// testing form.
///
/// So it loops over [`shipped_chains`]. The axis it now varies is the **detector composition**, and
/// what it holds constant is written into TESTING's table rather than left as a dash — that dash is
/// what the sixth arrival of this lesson looked like.
#[test]
fn every_fixpoint_pass_is_charged_to_the_request_budget() {
    // Small enough to be instant, dense enough that a scan certainly validates something.
    let field = distinct_digit_groups(4_000, 0);

    for (chain, detector) in shipped_chains() {
        // (a) The direct claim: `redetect` — the fixpoint's passes 1..n — charges like `try_detect`.
        //     With an allowance of one unit, both must refuse. Before M10-R35 the second returned
        //     `Ok` with a full scan and spent nothing, which is the shape a partial scan takes.
        for method in ["try_detect", "redetect"] {
            let budget = Budget::new(1);
            let outcome = if method == "try_detect" {
                detector.try_detect(&field, &budget)
            } else {
                detector.redetect(&field, &budget)
            };
            assert!(
                outcome.is_err(),
                "{chain}: {method} did not refuse on a one-unit allowance — it is scanning without \
                 charging the request, so the fixpoint's later passes are unbounded (M10-R35)"
            );
            assert!(
                budget.spent() > 0,
                "{chain}: {method} spent nothing on a field full of phone-shaped candidates"
            );
        }

        // (b) One budget across a whole `mask_all`, on text where the **second** pass does phone
        //     work.
        //
        //     **The obvious two-pass example is the wrong one here, and finding that out is the
        //     point.** M4-R17's `4111111111111111555 867 5309` does need two passes — masking the
        //     phone exposes a Luhn-valid card — but the exposed value is a *card*, and card
        //     validation is deliberately free (M10-R29). Total spend equals pass 0's, and the
        //     assertion below fails on correct code. What is needed is masking that exposes a
        //     **phone**, and an ASCII word boundary is what does it: in `...com347 1234567` there is
        //     no boundary between `m` and `3`, so the phone family cannot match. Mask the email and
        //     `[EMAIL_1]347 1234567` puts a `]` there.
        let two_pass = "write to bob@test.com347 1234567 today";
        let budget = Budget::new(MAX_PHONE_VALIDATIONS_PER_REQUEST);
        let mut vault = Vault::new();
        let masked = vault
            .mask_all(two_pass, detector.as_ref(), &budget)
            .unwrap_or_else(|e| panic!("{chain}: this must converge, not refuse: {e}"));
        assert!(
            masked.contains("[PHONE_"),
            "{chain}: the phone exposed by masking the email must be masked too — without that this \
             case is not exercising a later pass at all: {masked}"
        );

        let one_pass_only = Budget::new(MAX_PHONE_VALIDATIONS_PER_REQUEST);
        let _ = detector
            .try_detect(two_pass, &one_pass_only)
            .unwrap_or_else(|e| panic!("{chain}: pass 0 alone must not refuse: {e}"));
        assert!(
            budget.spent() > one_pass_only.spent(),
            "{chain}: the whole fixpoint ({} units) cost no more than its first pass ({} units) — \
             the later passes are not being charged to the request's allowance (M10-R35)",
            budget.spent(),
            one_pass_only.spent()
        );
    }
}

/// **DOS-09 (M10-R42) — the same claim end to end, on the shape the cache makes reachable.**
///
/// DOS-07 drives `PrivacyStage` with **distinct** fields and no cache; that is the M10-R28 attack. The
/// M10-R35 attack is its complement: **identical** fields with the cache on, so pass 0 is served from
/// the cache for free while `redetect` — deliberately never cached, because later passes run on
/// per-request masked text — is where the whole cost lands. A `redetect` that drops the allowance is
/// therefore invisible to every guard that omits the cache, which is exactly how the defect survived
/// one level up (M10-R42).
///
/// The body is M10-R35's own: a lead `11` group no enabled plan accepts, so nothing is masked and
/// every pass re-validates the whole field. Measured on the shipped chain, 20 x 20 KB fields:
/// **205,065** units charged against **9,765** with the defect present — a 21x differential, which
/// decides refuse-versus-forward at any allowance between the two.
#[test]
fn identical_fields_through_the_cached_chain_are_charged_on_every_pass() {
    const FIELDS: usize = 20;
    const TEST_BUDGET: usize = 20_000;

    // One field, repeated verbatim — the case the detection cache exists for (M7's system prompt),
    // and the one where pass 0 becomes free while the later passes do not.
    //
    // **Identical across fields, distinct *within* one — and getting that backwards is this
    // milestone's oldest trap arriving for the fifth time.** The first draft repeated a single
    // literal group, so the per-scan memo collapsed a 20 KB field to *one* validated candidate and
    // the request was forwarded — which reads as "the budget is not charged" when it is really the
    // generator saying nothing was there to charge for (M10-R20, DOS-06's odometer, DOS-BUD's SQL
    // column). The cache keys on the whole field, so identical fields still hit it; the odometer is
    // what keeps each field's own scan expensive.
    //
    // The lead `11` is rejected by every enabled plan, so nothing is masked and every pass
    // re-validates the whole field — which is the point: the cost lands on `redetect`, which is
    // never cached.
    let mut field = String::from("a@b.com ");
    let mut i = 0u64;
    while field.len() < 20 * 1024 {
        i += 1;
        field.push_str(&format!(
            "row 11 {:04} {:04} end ",
            1000 + i % 9000,
            1000 + (i / 9000) % 9000
        ));
    }
    let messages: Vec<serde_json::Value> = (0..FIELDS)
        .map(|_| serde_json::json!({ "role": "user", "content": field.clone() }))
        .collect();

    let cached = CachingDetector::new(
        Box::new(CompositeDetector::new(vec![Box::new(
            StructuredRecognizers::new(),
        )])),
        16,
    );
    let stage = PrivacyStage::with_validation_budget(Box::new(cached), TEST_BUDGET);
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: serde_json::json!({ "messages": messages }),
    };
    stage.on_request(&mut req, &mut ctx);

    let reason = ctx.block.expect(
        "twenty identical digit-dense fields must be REFUSED through the cached chain. A cache hit \
         legitimately spends nothing on pass 0 — but the later passes are never cached, so if this \
         request is forwarded, some wrapper is handing the detector a fresh allowance on `redetect` \
         (M10-R42)",
    );
    assert!(
        reason.contains("budget") && reason.contains("per request"),
        "the refusal must name the allowance and its unit, got: {reason}"
    );
}

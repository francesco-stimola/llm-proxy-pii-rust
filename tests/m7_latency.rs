//! M7 / S0 — the NER latency question, measured on a **realistic** Claude Code turn.
//!
//! **This file exists because the numbers that opened M7 were measured on the wrong shape.**
//! DEVLOG 2026-07-16 reports ~0.96 s/KB from a fixture densely packed with names
//! (`"Il cliente Mario Rossi di Acme SpA a Milano…"` ×450). Real Claude Code traffic is the
//! opposite: ~30 KB of instruction boilerplate and tool schemas carrying almost no PII, plus a
//! ~100-byte user message that carries all of it. And [`Vault::mask_all`] runs **per field**, so
//! what decides the turn cost is the **field distribution**, not the body size.
//!
//! The lesson this whole milestone rests on (M4-R13, then M5's PERF-01, now this):
//! **a corpus has a shape, and that shape is a blind spot. The fixture IS the experiment.**
//!
//! ## What is deliberately NOT done here
//!
//! A **captured** real body is tempting — the trace log has one — but it is already **masked**, so
//! its NER pass finds nothing and the measurement lies in the *optimistic* direction. We synthesize
//! the **shape**, not the content, and assert that shape below so it cannot silently drift.
//!
//! ## Running
//!
//! ```text
//! set NER_MODEL_PATH=…\onnx\model_quantized.onnx
//! set NER_TOKENIZER_PATH=…\tokenizer.json
//! set NER_LABELS=O,B-DATE,I-DATE,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC
//! cargo test-onnx --test m7_latency -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **`--test-threads=1` is not optional here, and leaving it off is a measurement bug (M7-R12).**
//! Cargo's harness runs tests concurrently by default, so without it these benchmarks measure the
//! product **against four other copies of itself**. Measured on the reference box at constant power:
//! **1.50×** on the absolute (default 4,757 ms isolated → 7,142 ms contended). This file spent three
//! review rounds attributing that kind of gap to power management. The ratio each test prints
//! survives it — that is the point of the calibration leg — but the millisecond columns do not.
#![cfg(feature = "onnx")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use llm_proxy_pii_rust::pii::anonymizer::Vault;
use llm_proxy_pii_rust::pii::composite::CompositeDetector;
use llm_proxy_pii_rust::pii::onnx::{
    available_cores, resolve_pool_and_intra, ExecutionProvider, OnnxNerDetector,
};
use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pii::{DetectError, PiiDetector, PiiEntity};

// ---------------------------------------------------------------------------
// The fixture: one realistic Claude Code turn
// ---------------------------------------------------------------------------

// Lives in a shared module because M10's over-mask guard needs the SAME turn while running
// in the **default** build — this file is `onnx`-only, and two copies of a fixture whose
// whole value is "text nobody curated" would drift apart. The shape assertion below is part of
// an `#[ignore]`d test in this `onnx`-gated file, so it does NOT run in an ordinary `cargo
// test` — `tests/phone_overmask.rs` is what pins the fixture's size, part distribution and span
// set on every default run (M10-R18).
#[path = "common/m7_turn.rs"]
mod m7_turn;

use m7_turn::{realistic_turn, Field, Part};

// ---------------------------------------------------------------------------
// A detector that counts what `mask_all` asks of it
// ---------------------------------------------------------------------------

/// Wraps the real hybrid and records, per `try_detect` call, how many entities came back.
///
/// This is what makes the **fixpoint pass count** observable per field: `mask_all` calls the
/// detector once, and only calls it again if the first call found something. So the recorded
/// sequence `[0]` means one pass, `[3, 0]` means two — the difference M4-R21 priced at ~2x and S0
/// claims the boilerplate never pays.
struct CountingDetector<'a> {
    inner: &'a dyn PiiDetector,
    calls: AtomicUsize,
    bytes: AtomicUsize,
    found: Mutex<Vec<usize>>,
}

impl<'a> CountingDetector<'a> {
    fn new(inner: &'a dyn PiiDetector) -> Self {
        Self {
            inner,
            calls: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            found: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Entity counts for the calls made since `from`, i.e. the passes of one field.
    fn found_since(&self, from: usize) -> Vec<usize> {
        self.found.lock().expect("not poisoned")[from..].to_vec()
    }
}

impl PiiDetector for CountingDetector<'_> {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        self.try_detect(input).unwrap_or_default()
    }

    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(input.len(), Ordering::Relaxed);
        let out = self.inner.try_detect(input)?;
        self.found.lock().expect("not poisoned").push(out.len());
        Ok(out)
    }
}

/// The production detector: structured recognizers + the ONNX NER, merged through the shared
/// overlap resolver. **Unwrapped** — no `FailOpen` — so an inference error surfaces here instead of
/// being silently swallowed into a structured-only measurement (which is exactly how the first M6
/// live run measured half the product).
fn build_hybrid_with(pool: usize, intra: usize) -> CompositeDetector {
    let model =
        std::env::var("NER_MODEL_PATH").expect("set NER_MODEL_PATH (see this file's doc comment)");
    let tokenizer = std::env::var("NER_TOKENIZER_PATH").expect("set NER_TOKENIZER_PATH");
    let labels = std::env::var("NER_LABELS").expect("set NER_LABELS");
    let id2label: Vec<String> = labels.split(',').map(str::to_string).collect();
    let ner = OnnxNerDetector::load(
        &model,
        &tokenizer,
        id2label,
        pool,
        intra,
        false,
        ExecutionProvider::Cpu,
    )
    .expect("load NER");
    CompositeDetector::new(vec![Box::new(StructuredRecognizers::new()), Box::new(ner)])
}

/// **Exactly what `server.rs` runs** — same function, same constant, same env vars (M7-R1).
///
/// This used to resolve its own pool default of `1` while the server defaulted to `2`, so M7's
/// executable bar measured the *personal-proxy* shape and reported the number as the *default's*.
/// The two shapes differed, so that was a guard with 28% headroom on a config nobody ran and none on
/// the one they did. Both now route through this function and the server's `DEFAULT_POOL_SIZE`.
/// (Since the 2026-07-17 flip that default *is* `pool=1`, so the personal shape and the default have
/// converged — but they are still resolved here, not assumed, and `bar_shapes` also guards the
/// pooled `NER_POOL_SIZE=2` shape a centralizing operator sets.)
fn build_hybrid() -> CompositeDetector {
    let (pool, intra) = resolve_pool_and_intra(
        std::env::var("NER_POOL_SIZE").ok().as_deref(),
        std::env::var("NER_INTRA_THREADS").ok().as_deref(),
        available_cores(),
    );
    build_hybrid_with(pool, intra)
}

/// The shapes M7's bar must hold for, resolved through the **server's own** policy.
///
/// Both are shipped configurations — the single-session default an operator gets by setting nothing
/// (the personal shape, since the 2026-07-17 flip; `pool=1` → the whole box), and the pooled
/// `NER_POOL_SIZE=2` shape a *centralizing* operator sets for concurrent clients. The bar is
/// asserted on each, so the trade is documented in the one place that *fails* when it stops being
/// true.
fn bar_shapes() -> Vec<(&'static str, usize, usize)> {
    let cores = available_cores();
    let (default_pool, default_intra) = resolve_pool_and_intra(None, None, cores);
    let (shared_pool, shared_intra) = resolve_pool_and_intra(Some("2"), None, cores);
    vec![
        (
            "default / personal (NER_POOL_SIZE unset)",
            default_pool,
            default_intra,
        ),
        ("centralized (NER_POOL_SIZE=2)", shared_pool, shared_intra),
    ]
}

/// Median of a sorted-in-place sample. Reported alongside the **minimum** because on a noisy box
/// the minimum is the closest thing to the interference-free cost, while the median says whether
/// the sample is stable enough to conclude anything at all (M7-R2).
fn min_and_median(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let min = samples[0];
    let median = samples[samples.len() / 2];
    (min, median)
}

/// How many times each configuration is measured. **1 was not enough, and that is a finding
/// (M7-R2):** at n=1 this harness reported "SMT helps, 12 threads beat 6 by 18%" — a conclusion
/// that inverts run to run, because the same configuration spans ~39% across repeats. An 18% effect
/// read off a 39% spread is noise wearing a conclusion's clothes.
///
/// **And repeats alone are still not enough (M7-R9).** Min-of-N removes *jitter*; it cannot remove a
/// **regime shift**, because all N reps sit inside the same regime — they agree tightly and
/// confidently report the wrong number. Measured on the reference box: the same shipped default
/// masked this fixture in 2,462 ms, 3,943 ms and 4,933 ms on three different occasions, each with a
/// within-run spread under 7%. **Precise, and wrong.** That is why the bar below asserts a *ratio*.
const REPS: usize = 3;

/// **The pre-M7 shape** — `pool=2, intra=1`, i.e. what shipped before this milestone. Used as an
/// in-run **calibration leg**: measured seconds away from the shapes under test, on the same box in
/// whatever state it is in, so that state **cancels out of the ratio** (M7-R9).
const PRE_M7_SHAPE: (usize, usize) = (2, 1);

/// What [`PRE_M7_SHAPE`] measured on the reference box, isolated (`--test-threads=1`), on its
/// energy-efficiency plan: **~10,100 ms**. Not a bar and never asserted — a **yardstick**, so the
/// harness can tell you how far your box is from the one the READMEs quote *before* you go hunting
/// for the difference in the code (M7-R12: *a calibration leg you print but never compare to
/// anything is half a calibration*).
const REFERENCE_PRE_M7_MS: f64 = 10_100.0;

/// **M7's deliverable, stated regime-invariantly.** The absolute wall clock is a property of the
/// box; the *speedup over the pre-M7 shape* is a property of the change. What that speedup cancels
/// is the box's **power/scheduling state** — verified: it held at ~1.7–2.3× while the pre-M7
/// absolute swung from ~4,400 ms to ~9,000 ms across occasions (isolated vs contended, one run vs
/// another — *not* AC vs battery, which on the reference box are the same energy-efficiency plan;
/// M7-R17). It does **not** fully cancel box *speed* at fixed cores, so a faster box compresses the
/// ratio toward the floor (M7-R18: 2.19× on the reference box, 1.74× on a faster one). Hence the
/// durable claim is **this floor**, not any single observed band — the floor is what the guard
/// enforces and what the docs should quote, precisely because the band keeps being undercut by the
/// next clean run.
const MIN_SPEEDUP_VS_PRE_M7: f64 = 1.5;

/// A **loose** absolute ceiling — deliberately far above the ~3 s product bar (M7-R9).
///
/// A hard 3 s assert on an uncontrolled box is a box-state detector, not a regression detector: it
/// goes red because a laptop is unplugged, while a genuine 20% regression (2,462 → 2,954) still
/// ships green. This catches the failure that actually matters — an order-of-magnitude one, the
/// 27 s → ~5 s win being undone — and stays quiet through a regime shift. **The ~3 s bar lives on as
/// a *reported product claim* (the READMEs), which is the honest home for a statement about
/// user-perceived latency on a reference box.**
///
/// **15 s, not the 8 s this shipped as (M7-R14).** 8 s was calibrated against uncontended runs and
/// was not loose at all: the reviewer's *documented-command* run measured a **median of 10,391 ms**
/// — the ceiling fired on the harness's own recipe, pointing the reader at their power state for
/// something that was really test concurrency. A ceiling that fires on a correct build is worse than
/// no ceiling; the win being guarded is 27 s → ~5 s, so 15 s still catches it with room.
const ABSOLUTE_SANITY_CEILING_MS: f64 = 15_000.0;

/// Below this many cores the S2 ratio guard is skipped. **The reason changed with the 2026-07-17
/// default flip (M7.1), while the number did not.** Under the old `pool=2` default `intra` floored at
/// 1 below 4 cores, so the derived default *was* [`PRE_M7_SHAPE`] and the ratio was 1.0 **by
/// construction** — nothing to measure (M7-R13). Under `pool=1` the derivation is `intra = cores`, so
/// the default equals the pre-M7 *thread count* (ratio 1.0 by construction) **only at 1 core**;
/// between 2 and 3 it does add threads, but too few to clear the [`MIN_SPEEDUP_VS_PRE_M7`] floor
/// reliably. Measured only on the ≥12-core reference box, **4 is the conservative line** below which
/// the few-thread shapes are untested against the floor and would false-fire. Either way the
/// takeaway is unchanged: *M7's speedup scales with the core count, and this guard has nothing
/// dependable to say below 4 cores.*
///
/// `resolve_pool_and_intra(None, None, 1)` → `(1, 1)` — intra = the pre-M7 shape's — pinned in
/// `onnx::thread_tests::the_default_gives_one_session_the_whole_box`.
const MIN_CORES_FOR_A_MEANINGFUL_RATIO: usize = 4;

/// Measure one shape: warm the arenas, then the best of [`REPS`] turns.
fn measure_shape(pool: usize, intra: usize, fields: &[Field]) -> (f64, f64) {
    let detector = build_hybrid_with(pool, intra);
    let _ = mask_a_turn(&detector, fields); // warm-up; never measured
    let samples: Vec<f64> = (0..REPS)
        .map(|_| mask_a_turn(&detector, fields).as_secs_f64() * 1000.0)
        .collect();
    min_and_median(samples)
}

/// Mask the whole turn with one vault, as production does. Returns the wall clock.
fn mask_a_turn(detector: &dyn PiiDetector, fields: &[Field]) -> std::time::Duration {
    let mut vault = Vault::new();
    let started = Instant::now();
    for f in fields {
        vault
            .mask_all(&f.text, detector)
            .unwrap_or_else(|e| panic!("{}: masking must converge: {e}", f.name));
    }
    started.elapsed()
}

fn part_label(p: Part) -> &'static str {
    match p {
        Part::System => "system",
        Part::ToolDescription => "tool desc",
        Part::SchemaDescription => "schema desc",
        Part::UserMessage => "user msg",
    }
}

// ---------------------------------------------------------------------------
// S0 — the measurement
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn m7_s0_a_realistic_claude_code_turn_measured_per_field() {
    let hybrid = build_hybrid();
    let detector = CountingDetector::new(&hybrid);
    let fields = realistic_turn();

    // ---- The fixture is the experiment: assert its shape, or the numbers mean nothing. ----
    let total: usize = fields.iter().map(|f| f.text.len()).sum();
    // KiB, and labelled as such: the ms/KB columns below divide by 1024, so quoting the size in
    // decimal kB made the docs internally inconsistent (M7-R6).
    eprintln!(
        "\n=== fixture: {} fields, {total} bytes ({:.1} KiB) ===",
        fields.len(),
        total as f64 / 1024.0
    );
    assert!(
        (20_000..50_000).contains(&total),
        "the fixture must be the size of a real Claude Code turn (20-50 KB), got {total} — \
         a fixture that drifts out of shape measures something else, which is the whole reason \
         this file exists"
    );
    assert!(
        fields.iter().filter(|f| f.part == Part::System).count() == 1,
        "the system prompt must be ONE field — if it were many, the per-field cost model changes"
    );
    assert!(
        fields
            .iter()
            .filter(|f| f.part == Part::SchemaDescription)
            .count()
            > 50,
        "the schema tier must be many small fields — that asymmetry is what S0 is measuring"
    );

    // ---- Mask the turn exactly as production does: ONE vault, field by field, in order. ----
    let mut vault = Vault::new();
    let mut rows: Vec<(Part, String, usize, u128, Vec<usize>)> = Vec::new();
    let turn_started = Instant::now();
    for f in &fields {
        let calls_before = detector.calls();
        let started = Instant::now();
        let _masked = vault
            .mask_all(&f.text, &detector)
            .unwrap_or_else(|e| panic!("{}: masking must converge: {e}", f.name));
        let elapsed = started.elapsed().as_millis();
        rows.push((
            f.part,
            f.name.clone(),
            f.text.len(),
            elapsed,
            detector.found_since(calls_before),
        ));
    }
    let turn = turn_started.elapsed();

    // ---- Per-part subtotals: which tier actually costs the turn? ----
    eprintln!("\n=== per part ===");
    eprintln!(
        "{:<12} {:>6} {:>9} {:>9} {:>8} {:>7}",
        "part", "fields", "bytes", "ms", "ms/KB", "passes"
    );
    for part in [
        Part::System,
        Part::ToolDescription,
        Part::SchemaDescription,
        Part::UserMessage,
    ] {
        let sel: Vec<_> = rows.iter().filter(|r| r.0 == part).collect();
        let bytes: usize = sel.iter().map(|r| r.2).sum();
        let ms: u128 = sel.iter().map(|r| r.3).sum();
        let passes: usize = sel.iter().map(|r| r.4.len()).sum();
        eprintln!(
            "{:<12} {:>6} {:>9} {:>9} {:>8.0} {:>7}",
            part_label(part),
            sel.len(),
            bytes,
            ms,
            if bytes > 0 {
                ms as f64 / (bytes as f64 / 1024.0)
            } else {
                0.0
            },
            passes
        );
    }

    // ---- The S0 hypothesis, stated as data rather than hope. ----
    eprintln!("\n=== the S0 hypothesis: does the boilerplate really take ONE pass? ===");
    for (part, name, bytes, ms, found) in &rows {
        if *part == Part::System || found.len() > 1 || found.first().is_some_and(|n| *n > 0) {
            eprintln!(
                "  {:<12} {:<58} {:>6} B {:>7} ms  passes={} found={:?}",
                part_label(*part),
                name,
                bytes,
                ms,
                found.len(),
                found
            );
        }
    }

    let multi_pass: Vec<_> = rows.iter().filter(|r| r.4.len() > 1).collect();
    eprintln!(
        "\nfields needing >1 pass: {}/{} ({} of {} bytes)",
        multi_pass.len(),
        rows.len(),
        multi_pass.iter().map(|r| r.2).sum::<usize>(),
        total
    );
    eprintln!(
        "\n=== TURN TOTAL: {:?} ({} detector calls over {} bytes) ===\n",
        turn,
        detector.calls(),
        detector.bytes()
    );

    // No bar assert here, deliberately (M7-R1/M7-R2). This test's job is the per-field
    // *breakdown*, and it takes ONE sample — which on this harness carries a ~39% run-to-run
    // spread. The bar has ~5% headroom against the shipped default, so asserting it on a single
    // sample would be a coin flip dressed as a guard. The bar lives in
    // `m7_s2_the_bar_holds_for_every_shipped_shape`, over repeats, on every shape we ship.
    eprintln!(
        "(reported, not asserted: one sample. The bar is asserted in \
         m7_s2_the_bar_holds_for_every_shipped_shape, over {REPS} reps × every shipped shape.)\n"
    );
}

/// **M7's deliverable, guarded the only way an uncontrolled box allows: as a RATIO (M7-R9).**
///
/// The bar was declared *before* the numbers (ROADMAP → M7, S2): **a realistic turn under ~3 s
/// ships**, and if threads alone get there, S3 (a cache) and S4 (skipping the NER on later fixpoint
/// passes) are not built, because both put real risk on the masking path.
///
/// **So why doesn't this assert 3 s?** Because that assert cannot tell the two failures apart. This
/// fixture, this code, this box, three occasions: **2,462 / 3,943 / 4,933 ms** — each with a
/// within-run spread under 7%, differing in the box's scheduling/power state (*not* AC vs battery,
/// which on the reference box are the same energy-efficiency plan; M7-R17). A hard 3 s assert on
/// that box is a **box-state detector**: it goes red because the box is in a slow state, while a
/// genuine 20% regression (2,462 → 2,954) ships green. It fires on what doesn't matter and is blind
/// to what does.
///
/// **The ratio is the part that is about the code.** [`PRE_M7_SHAPE`] is measured as a calibration
/// leg *in this same run*, seconds away from the shapes under test, so whatever the box is doing
/// divides out. It held ~**1.7–2.3×** across every regime while the absolute moved ~2×. **The claim
/// to quote is the asserted floor ([`MIN_SPEEDUP_VS_PRE_M7`]), not a band** — the ratio cancels
/// power but not raw box speed, so a faster box compresses it toward the floor (M7-R18), and every
/// tight band published got undercut by the next clean run. The ~3 s figure lives on where it
/// belongs: a **reported product claim** in the READMEs, with its box and conditions named.
///
/// **Both shapes, because both ship** (M7-R1): the single-session default an operator gets by
/// setting nothing (the personal shape since the 2026-07-17 flip), and the pooled `NER_POOL_SIZE=2`
/// shape a centralizing operator sets for concurrent clients.
///
/// **What this guard does NOT see, stated because an honest guard states its blind spot (M7-R14).**
/// The floor is 1.5 against a worst *observed* ~1.7, so it tolerates a **~13% regression** —
/// materially the same blindness the wall-clock bar had. **The ratio buys regime-independence, not
/// sensitivity**; it answers R9's false *positive* (a red bar because the box is slow) and not the
/// false *negative*. The floor cannot simply be tightened: nearer 1.7 it would start false-firing on
/// a fast box that legitimately compresses the ratio, which is the failure it was built to end.
///
/// **Run it isolated — `--test-threads=1` (M7-R12).** The module doc's command lets cargo run all
/// five perf tests concurrently; measured at **1.50×** on the absolute, at constant power. The
/// ratio survives that (it is what proved the design), but the ms columns do not.
#[test]
#[ignore]
fn m7_s2_the_bar_holds_for_every_shipped_shape() {
    let cores = available_cores();
    // M7-R13 / M7.1. Below 4 cores the derived shapes are too thread-poor to clear the floor
    // reliably (and at 1 core the default's intra=1 IS `PRE_M7_SHAPE`'s, ratio 1.0 by construction).
    // The old version asserted anyway and told the reader it had found "a real regression in the
    // thread work, not a slow box" — a conclusion it had not earned, on a box where M7 simply has
    // nothing to deliver. Say that instead.
    if cores < MIN_CORES_FOR_A_MEANINGFUL_RATIO {
        eprintln!(
            "\nSKIPPED: {cores} cores. Under the pool=1 default the derived shapes here are too \
             thread-poor to clear the {MIN_SPEEDUP_VS_PRE_M7}x floor reliably (and at 1 core the \
             default intra=1 IS the pre-M7 shape {:?}'s thread count — ratio 1.0 by construction). \
             M7's speedup scales with the box and this guard has nothing dependable to say below \
             {MIN_CORES_FOR_A_MEANINGFUL_RATIO} cores — a real property of the derivation, not a \
             failure (M7-R13 / M7.1).",
            PRE_M7_SHAPE
        );
        return;
    }

    let fields = realistic_turn();
    let bytes: usize = fields.iter().map(|f| f.text.len()).sum();
    eprintln!(
        "\n=== S2: the bar, as a ratio vs the pre-M7 shape. {REPS} reps each, {bytes} B turn \
         ({:.1} KiB), {cores} cores ===",
        bytes as f64 / 1024.0
    );

    // The calibration leg first: everything below is read against it, and it is what makes the
    // absolute numbers interpretable rather than merely printed.
    let (base_min, base_median) = measure_shape(PRE_M7_SHAPE.0, PRE_M7_SHAPE.1, &fields);
    eprintln!(
        "{:<32} pool={} intra={:<3} min {base_min:>7.0} ms   median {base_median:>7.0} ms   \
         <- calibration leg (pre-M7)",
        "pre-M7 (what shipped before)", PRE_M7_SHAPE.0, PRE_M7_SHAPE.1
    );
    // **Compare the calibration leg to something, or it is half a calibration (M7-R12).** Printing
    // it told a reader nothing they could act on; measured against the reference box it says, in
    // the harness's own voice, how much of any surprise below is *this box* before they go looking
    // for it in the code. It is a report, never an assert — a slow box is not a defect.
    let drift = base_min / REFERENCE_PRE_M7_MS;
    eprintln!(
        "   ^ the reference box measured {REFERENCE_PRE_M7_MS:.0} ms here, so this box is running \
         **{drift:.2}x** that. Read every ms below through that factor; the `x vs pre-M7` column \
         already has it divided out."
    );

    // **Measure and print every row BEFORE asserting any (M7-R9).** The first cut asserted inside
    // the loop, so a failure on the default meant the personal shape never ran and never printed —
    // on a test whose whole purpose is the two-row comparison. A guard must not destroy the
    // evidence you need to interpret it.
    let measured: Vec<_> = bar_shapes()
        .into_iter()
        .map(|(label, pool, intra)| {
            let (min, median) = measure_shape(pool, intra, &fields);
            (label, pool, intra, min, median)
        })
        .collect();

    for (label, pool, intra, min, median) in &measured {
        eprintln!(
            "{label:<32} pool={pool} intra={intra:<3} min {min:>7.0} ms   median {median:>7.0} ms   \
             {:.2}x vs pre-M7",
            base_min / min
        );
    }
    eprintln!(
        "\nThe ms columns are this box, right now, and are NOT comparable across runs: the same \
         default has measured 2,462 / 3,943 / 4,724 / 4,757 / 4,841 / 4,933 / 7,142 ms here. The \
         `x vs pre-M7` column is FAR more stable — both legs ran in this run, so whatever the box is \
         doing (power state, background load) divides out. It has held ~1.7-2.3x across every one of \
         those. The floor the guard enforces is >=1.5x; a faster box compresses the ratio toward it \
         (M7-R18), so quote the floor, not the day's number.\n\
         \n\
         **Do not reach for a power-state explanation first — this file has been wrong about that \
         twice (M7-R12/R17).** The runs once labelled 'battery' and 'AC' were the SAME \
         energy-efficiency plan (charger attached or not), so that label ordered nothing. The \
         variables that ARE measured: test concurrency (1.50x — run with `--test-threads=1`), and \
         the calibration line above, which tells you how this box compares to the one the READMEs \
         quote *before* you go looking in the code.\n"
    );

    for (label, pool, intra, min, _) in &measured {
        let speedup = base_min / min;
        assert!(
            speedup > MIN_SPEEDUP_VS_PRE_M7,
            "{label} (pool={pool}, intra={intra}) is only {speedup:.2}x the pre-M7 shape \
             ({base_min:.0} ms -> {min:.0} ms), under the {MIN_SPEEDUP_VS_PRE_M7}x floor. Both legs \
             ran in THIS run, so how fast the box is running cancels out — a slow box cannot cause \
             this. A SMALL box can: the speedup scales with the core count (this one reports \
             {cores}), which is why the guard skips below {MIN_CORES_FOR_A_MEANINGFUL_RATIO} \
             cores. Otherwise, suspect the thread work (M7-R13)."
        );
        assert!(
            *min < ABSOLUTE_SANITY_CEILING_MS,
            "{label} (pool={pool}, intra={intra}) masks a realistic turn in {min:.0} ms, over the \
             {ABSOLUTE_SANITY_CEILING_MS:.0} ms sanity ceiling. **Before suspecting the code, check \
             (a) that you ran this isolated — `--test-threads=1`; the documented command lets cargo \
             run all five perf tests CONCURRENTLY, measured at 1.5x on the absolute — and (b) your \
             box's power state.** This ceiling is deliberately order-of-magnitude and exists only to \
             catch the 27 s -> ~5 s win being undone. The ~3 s product claim is NOT asserted here; \
             see the READMEs and M7-R9/M7-R12/M7-R14."
        );
    }
}

/// **S1 — the thread sweep.** How much of the box can a single request actually use?
///
/// The plan says two things here are to be **measured, not reasoned about**, and this is where:
///
/// 1. **SMT.** `available_parallelism()` reports *logical* cores (12 = 6 physical × HT). Dense
///    math often prefers the physical count, so 6 may beat 12.
/// 2. **Sublinear scaling.** Expect ~3x from 6 threads, not 6x — and less on an **int8** model
///    whose kernels are memory-bandwidth-bound rather than ALU-bound.
///
/// It also measures the claim that reordered this milestone (S1a): a single request occupies **one
/// session**, so growing the *pool* buys a lone request nothing, while growing *intra* does.
/// `pool=2, intra=1` is the pre-M7 baseline every row reads against; the shipped default is now
/// `pool=1` → `intra=cores` (the `1×12` row on this box), one session over the whole machine.
#[test]
#[ignore]
fn m7_s1_how_much_of_the_box_can_one_request_use() {
    let cores = available_cores();
    let fields = realistic_turn();
    let bytes: usize = fields.iter().map(|f| f.text.len()).sum();
    eprintln!(
        "\n=== S1 thread sweep: {cores} logical cores, {bytes} B turn, {REPS} reps per shape ==="
    );
    eprintln!(
        "{:>5} {:>6} {:>9} {:>9} {:>9} {:>8} {:>8}",
        "pool", "intra", "min ms", "med ms", "spread", "ms/KiB", "vs 2x1"
    );

    let mut baseline: Option<f64> = None;
    // (pool, intra). `2 x 1` first: it is what shipped before M7, and every other row reads
    // against it.
    for (pool, intra) in [
        (2, 1),
        (1, 1),
        (1, 2),
        (1, 4),
        (1, 6),
        (1, 12),
        (2, 6),
        (4, 3),
    ] {
        if pool * intra > cores * 2 {
            continue; // don't bother measuring absurd oversubscription on a small box
        }
        let detector = build_hybrid_with(pool, intra);
        // One warm-up turn: the first inference pays lazy allocator/arena setup, and charging that
        // to whichever row happens to run first would be a measurement artifact, not a finding.
        let _ = mask_a_turn(&detector, &fields);
        let samples: Vec<f64> = (0..REPS)
            .map(|_| mask_a_turn(&detector, &fields).as_secs_f64() * 1000.0)
            .collect();
        let worst = samples.iter().cloned().fold(f64::MIN, f64::max);
        let (min, median) = min_and_median(samples);
        let base = *baseline.get_or_insert(min);
        eprintln!(
            "{pool:>5} {intra:>6} {min:>9.0} {median:>9.0} {:>8.0}% {:>8.0} {:>7.2}x",
            (worst - min) / min * 100.0,
            min / (bytes as f64 / 1024.0),
            base / min
        );
    }
    eprintln!(
        "\n**Read `spread` before believing any row's delta — and know that it UNDERSTATES the \n\
         noise.** `spread` is the within-run range; the same configuration also drifts BETWEEN \n\
         runs (measured on the reference box: `1x12` at 2.1 s / 2.5 s / 3.0 s on different runs, \n\
         a ~40% band). So this harness resolves *large* effects, not small ones.\n\
         \n\
         Believe a row only when a mechanism backs it (M7-R2 — M7's first cut did not, and turned \n\
         an 18% `1x6` vs `1x12` gap into a stated conclusion, \"SMT helps\", that inverts run to \n\
         run):\n\
         - **Sublinear scaling** — 12 threads buy ~2x, never 12x. Large, and it replicates.\n\
         - **The pool is inert at concurrency 1** (`2x1` ~ `1x1`) — believe this from the CODE, \n\
           not this table: one request occupies one session (the field walk holds `&mut Vault`, \n\
           `infer_chunked` loops its windows), so `pool` cannot help it. When these two rows \n\
           differ here, that is the box, not a mechanism.\n\
         - **SMT (`1x6` vs `1x12`)** — UNRESOLVED. The sign flips run to run. Do not read it.\n"
    );
}

/// **S1, the other half: does the shared-proxy case regress?**
///
/// The session pool exists for **concurrent throughput**, and M7 is about **single-request
/// latency**. Those are different goals, and the ROADMAP is explicit that throughput "must not
/// regress silently" — so it gets measured, not argued.
///
/// The question that **decided** the default (flipped to `pool=1` on 2026-07-17): `pool=1,
/// intra=all` serializes concurrent requests at the one session's mutex, but each of them then uses
/// the whole box. `pool=N, intra=cores/N` runs them side by side, each on a slice. **The box is the
/// box, but intra-op scaling is sublinear** — so N sessions × cores/N threads aggregate *more*
/// turns/s than one × cores, and this is not free: `pool=1` measured **~−23%** turns/s under
/// concurrency (two independent measurements; DEVLOG 2026-07-16). So `pool=1` is **not** a
/// throughput win — it wins on RAM (~270 MB less per removed session: measured 563 MB at `pool=1` vs
/// 834 MB at `pool=2`, since each session holds its own copy of the weights) and gives the lone
/// request the whole box (single-request latency a wash). The default targets the
/// personal proxy, which has **no** concurrency to lose that −23% on; a *centralizing* operator
/// serving concurrent clients sets `NER_POOL_SIZE=N` to reclaim it — which is why this is an
/// override, not the default's job.
#[test]
#[ignore]
fn m7_s1_throughput_under_concurrent_load_must_not_regress() {
    const CONCURRENCY: usize = 4;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let fields = realistic_turn();
    let bytes: usize = fields.iter().map(|f| f.text.len()).sum();

    eprintln!(
        "\n=== S1 throughput: {CONCURRENCY} concurrent turns, {cores} logical cores, \
         {bytes} B each ==="
    );
    eprintln!(
        "{:>5} {:>6} {:>10} {:>12} {:>12}",
        "pool", "intra", "total ms", "turns/s", "ms/turn"
    );

    for (pool, intra) in [(2, 1), (2, 6), (1, 12), (4, 3)] {
        let detector = build_hybrid_with(pool, intra);
        let _ = mask_a_turn(&detector, &fields); // warm up the arenas

        let started = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..CONCURRENCY {
                s.spawn(|| {
                    mask_a_turn(&detector, &fields);
                });
            }
        });
        let elapsed = started.elapsed();
        eprintln!(
            "{pool:>5} {intra:>6} {:>10.0} {:>12.3} {:>12.0}",
            elapsed.as_secs_f64() * 1000.0,
            CONCURRENCY as f64 / elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / CONCURRENCY as f64
        );
    }
    eprintln!(
        "\n`1x12` does NOT hold turns/s against `2x6` — it measured ~-23% (sublinear intra scaling; \
         DEVLOG 2026-07-16). So pool=1 (the shipped default since 2026-07-17) is a throughput trade a \
         centralizing operator reclaims with `NER_POOL_SIZE=N`; the personal case it defaults to has \
         no concurrency to lose, and keeps the whole box per request plus ~270 MB less RAM (563 vs \
         834 MB). See DEVLOG 2026-07-17.\n"
    );
}

/// **S0's hypothesis, tested directly: is the boilerplate entity-free?**
///
/// The plan asserts a real turn is "~30 KB of boilerplate with **~zero** PII", and concludes the
/// big field costs **one** fixpoint pass. That conclusion is load-bearing — it is the entire reason
/// the plan demotes the fixpoint lead (S4) and promotes the cache (S3).
///
/// This prints what the hybrid actually finds in text that contains no PII by construction. Every
/// hit here is a **false positive on boilerplate**, and each one silently doubles the cost of the
/// biggest field in the turn.
///
/// (Printing entity text is safe and normal in this harness: the fixture is synthetic, contains no
/// real PII by construction, and `#[ignore]`d perf tests already print entity text — see
/// `ner_perf.rs::m5_r4_…`. The never-log-raw-PII rule governs the **product**, not a fixture.)
#[test]
#[ignore]
fn m7_s0_what_the_ner_finds_in_boilerplate_that_has_no_pii() {
    let hybrid = build_hybrid();
    let mut total = 0usize;

    for f in realistic_turn() {
        if f.part == Part::UserMessage {
            continue; // this one is SUPPOSED to have PII
        }
        let found = hybrid.try_detect(&f.text).expect("NER must not error");
        if found.is_empty() {
            continue;
        }
        total += found.len();
        eprintln!(
            "{:<12} {:<50} → {:?}",
            part_label(f.part),
            f.name,
            found
                .iter()
                .map(|e| (e.kind, e.text.as_str()))
                .collect::<Vec<_>>()
        );
    }

    eprintln!(
        "\n{total} entities found in text that contains no PII by construction.\n\
         Each one costs the field a SECOND fixpoint pass — i.e. a second full NER scan of it."
    );
}

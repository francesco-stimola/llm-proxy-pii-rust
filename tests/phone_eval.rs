//! **M10 · step 8 — the domestic-phone measurement.** `#[ignore]`d: it prints numbers, it
//! does not assert a product bar.
//!
//! The deliverable of M10 is this measurement, not the code change. Widening the candidate
//! set without it would trade a *documented* gap (the `it,us` default masking nothing) for
//! an *undocumented* over-mask, which is strictly worse — the first at least tells you what
//! you are not getting.
//!
//! Three questions, in the order the ROADMAP asks them:
//!
//! 1. **Per region:** recall on that country's real renderings, and the false-positive rate
//!    it alone contributes.
//! 2. **The union:** enabling N regions unions their accepted sets, so the compound FP-rate
//!    is ≥ the worst single one and grows with N. That number is what decides the default,
//!    and no per-region figure predicts it.
//! 3. **Latency per enabled region**, on the deterministic path that is today the *fast* one
//!    (~20 ms for a whole turn) — measured over the same real 22 KiB turn the over-mask
//!    guard uses.
//!
//! ## Running
//!
//! ```text
//! cargo test --release --test phone_eval -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **`--release` and `--test-threads=1` are both part of the contract** (M7-R12): a debug
//! build measures the wrong constant factor, and cargo's default concurrency measures the
//! product against other copies of itself. The precision figures are build-independent; the
//! milliseconds are not.

#[path = "common/m7_turn.rs"]
mod m7_turn;

use std::collections::HashMap;
use std::time::Instant;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::recognizers::{StructuredRecognizers, PHONE_REGIONS};
use llm_proxy_pii_rust::pii::{PiiDetector, PiiKind};

const CORPUS_JSON: &str = include_str!("corpus/pii_cases.json");

#[derive(Deserialize)]
struct Corpus {
    recognizers: HashMap<String, Category>,
}

#[derive(Deserialize)]
struct Category {
    positive: Vec<Case>,
    negative: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    input: String,
    locale: Option<String>,
    #[serde(default)]
    entities: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    kind: PiiKind,
    text: String,
}

fn national_phone_cases() -> Category {
    let corpus: Corpus = serde_json::from_str(CORPUS_JSON).expect("corpus must parse");
    corpus
        .recognizers
        .into_iter()
        .find(|(name, _)| name == "national_phone")
        .expect("the corpus must carry a `national_phone` category")
        .1
}

fn phones(detector: &StructuredRecognizers, input: &str) -> Vec<String> {
    detector
        .detect(input)
        .into_iter()
        .filter(|e| e.kind == PiiKind::Phone)
        .map(|e| e.text)
        .collect()
}

/// Digit-shaped non-phones **this harness generates**, on top of the corpus's curated ones,
/// grouped by the *kind* of text they imitate.
///
/// The corpus negatives are the *guard* — a small hand-chosen set that must stay green.
/// This is the *measurement*, and it is reported **per category on purpose**: one blended
/// FP-rate over a pool whose composition you chose is a number about the pool, not about the
/// product. Knowing that dates cost far more than ports is what lets a reader decide whether
/// the trade fits their traffic. Kept out of the corpus deliberately — a generated pool is
/// the wrong thing to freeze into a regression guard, because every addition to it would
/// look like a product change.
fn generated_negatives() -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();

    // Dates. Note which renderings can collide at all: ISO (`2026-07-29`) and
    // slash-separated (`29/07/2026`) cannot — no family accepts `/`, and a 4-digit leading
    // group is not a candidate. The exposure is specifically space- and dash-separated
    // day-month-year, which is why they are the ones generated here.
    for month in 1..=12u32 {
        for day in [1u32, 9, 15, 28] {
            out.push((
                "dates",
                format!("scadenza {day:02} {month:02} 2026 confermata"),
            ));
            out.push(("dates", format!("dated {day:02}-{month:02}-2026 signed")));
            out.push(("dates", format!("due 2026-{month:02}-{day:02} ok")));
            out.push(("dates", format!("le {day:02}/{month:02}/2026 signe")));
        }
    }
    // Ports, PIDs, byte sizes, offsets — the digit runs a coding agent sees constantly.
    for n in [80u32, 443, 3000, 8080, 8443, 9090, 5432, 6379] {
        out.push(("ports", format!("service on port {n} listening")));
        out.push(("ports", format!("bound {n} and {} together", n + 1)));
    }
    for k in 0..12u32 {
        let size = 512u32 << k;
        out.push(("sizes", format!("chunk {size} bytes read")));
        out.push((
            "sizes",
            format!("sizes {size} {} {} here", size * 2, size * 4),
        ));
    }
    for line in [1u32, 42, 128, 256, 1024, 4096] {
        out.push((
            "offsets",
            format!("line {line} of {} in the diff", line * 3),
        ));
        out.push((
            "offsets",
            format!("offset {line} {} {} bytes", line + 8, line + 16),
        ));
    }
    // Money, quantities, versions, HTTP codes, elapsed times.
    for amount in [1u32, 12, 250, 1500, 99] {
        out.push(("money", format!("budget {amount} 000 euro approvato")));
        out.push(("money", format!("qty {amount} x {} units", amount + 3)));
    }
    for (a, b, c) in [(200u32, 301, 404), (100, 204, 500), (301, 302, 307)] {
        out.push(("codes", format!("http {a} {b} {c} observed")));
    }
    for (a, b, c) in [(1u32, 2, 3), (10, 20, 30), (30, 60, 120), (15, 30, 45)] {
        out.push(("codes", format!("retry after {a} {b} {c} seconds")));
        out.push(("codes", format!("v{a}.{b}.{c} released today")));
    }
    // Identifiers that are emphatically not phone numbers.
    for n in 0..40u32 {
        out.push(("refs", format!("order 2026 {:04} shipped", 1000 + n * 7)));
        out.push(("refs", format!("ticket {} {} closed", 100 + n, 200 + n * 3)));
        out.push(("refs", format!("commit 9e31f36 at line {}", 100 + n)));
    }
    out
}

/// The category names in the generated pool, in report order.
const NEGATIVE_CATEGORIES: &[&str] = &[
    "dates", "ports", "sizes", "offsets", "money", "codes", "refs",
];

/// Score one region set: recall over the positives whose `locale` is in `owned`, plus the
/// false positives it produces on the curated and the generated pools.
fn report(
    label: &str,
    detector: &StructuredRecognizers,
    cases: &Category,
    owned: &[&str],
    generated: &[(&'static str, String)],
) {
    let mut hits = 0;
    let mut total = 0;
    let mut misses = Vec::new();
    for case in &cases.positive {
        if !owned.contains(&case.locale.as_deref().unwrap_or("")) {
            continue;
        }
        total += 1;
        let want: Vec<&str> = case
            .entities
            .iter()
            .filter(|e| e.kind == PiiKind::Phone)
            .map(|e| e.text.as_str())
            .collect();
        let got = phones(detector, &case.input);
        if want.iter().all(|w| got.iter().any(|g| g == w)) {
            hits += 1;
        } else {
            misses.push(format!("[{}] {:?} -> {got:?}", case.id, case.input));
        }
    }

    let fp_curated = cases
        .negative
        .iter()
        .filter(|case| !phones(detector, &case.input).is_empty())
        .count();
    let recall = if total == 0 {
        f64::NAN
    } else {
        hits as f64 / total as f64
    };
    print!(
        "{label:<7} recall {recall:>5.3} ({hits:>2}/{total:<2})  FPcur {:>5.3} ({}/{})  ",
        fp_curated as f64 / cases.negative.len() as f64,
        fp_curated,
        cases.negative.len()
    );

    let mut worst: Option<(String, Vec<String>)> = None;
    for category in NEGATIVE_CATEGORIES {
        let pool: Vec<&String> = generated
            .iter()
            .filter(|(c, _)| c == category)
            .map(|(_, s)| s)
            .collect();
        let bad: Vec<&&String> = pool
            .iter()
            .filter(|s| !phones(detector, s).is_empty())
            .collect();
        print!("{category} {:>5.3} ", bad.len() as f64 / pool.len() as f64);
        if worst.as_ref().is_none_or(|(_, w)| w.len() < bad.len()) {
            worst = Some((
                (*category).to_string(),
                bad.iter().take(3).map(|s| (**s).clone()).collect(),
            ));
        }
    }
    println!();
    for m in &misses {
        println!("        MISS {m}");
    }
    // *What* is over-masked matters as much as how often: the reader has to judge whether
    // these are strings a real payload would carry.
    if let Some((category, examples)) = worst.filter(|(_, e)| !e.is_empty()) {
        for e in examples {
            println!("        fp[{category}] {e:?} -> {:?}", phones(detector, &e));
        }
    }
}

/// **Per region, then the union.** The second is not predicted by the first — enabling N
/// regions unions their accepted sets, so the compound rate is ≥ the worst single one.
#[test]
#[ignore]
fn phone_precision_per_region_and_for_the_union() {
    let cases = national_phone_cases();
    let generated = generated_negatives();
    println!(
        "corpus: {} positives / {} curated negatives; {} generated negatives\n\
         FP columns are per category — one blended rate over a pool whose mix you chose is a \
         number about the pool.\n",
        cases.positive.len(),
        cases.negative.len(),
        generated.len()
    );

    for region in PHONE_REGIONS {
        let detector = StructuredRecognizers::with_regions(&[region.id]);
        report(region.code, &detector, &cases, &[region.code], &generated);
    }

    let all: Vec<&str> = PHONE_REGIONS.iter().map(|r| r.code).collect();
    let detector = StructuredRecognizers::new();
    println!();
    report("UNION", &detector, &cases, &all, &generated);

    // The compound effect, named: what the union masks that no single region does.
    let singles: Vec<StructuredRecognizers> = PHONE_REGIONS
        .iter()
        .map(|r| StructuredRecognizers::with_regions(&[r.id]))
        .collect();
    let union_only: Vec<&String> = generated
        .iter()
        .map(|(_, s)| s)
        .filter(|s| !phones(&detector, s).is_empty())
        .filter(|s| singles.iter().all(|d| phones(d, s).is_empty()))
        .collect();
    println!(
        "\nunion-only false positives: {} — {union_only:?}",
        union_only.len()
    );
}

/// **Latency per enabled region**, over the same real 22 KiB turn.
///
/// The shape that matters is the slope: shape (b) puts the region loop inside the validator,
/// so adding a region costs validations on candidates only — not another O(n·L) scan of
/// every field. A per-region *recognizer* would show a straight line with a much steeper
/// slope; this should be closer to flat.
#[test]
#[ignore]
fn phone_latency_per_enabled_region() {
    const REPS: usize = 7;
    let fields: Vec<String> = m7_turn::realistic_turn()
        .into_iter()
        .map(|f| f.text)
        .collect();
    let bytes: usize = fields.iter().map(|f| f.len()).sum();
    println!("fixture: {} fields, {bytes} bytes\n", fields.len());

    for n in 0..=PHONE_REGIONS.len() {
        let regions: Vec<_> = PHONE_REGIONS.iter().take(n).map(|r| r.id).collect();
        let detector = StructuredRecognizers::with_regions(&regions);
        let mut best = f64::MAX;
        for _ in 0..REPS {
            let start = Instant::now();
            let hits: usize = fields.iter().map(|f| detector.detect(f).len()).sum();
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            std::hint::black_box(hits);
            best = best.min(ms);
        }
        let names: Vec<&str> = PHONE_REGIONS.iter().take(n).map(|r| r.code).collect();
        println!("{n} region(s) {:<40} {best:>7.2} ms/turn", names.join(","));
    }
}

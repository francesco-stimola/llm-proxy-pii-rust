//! **PHONE-EVAL — the domestic-phone precision measurement, and since M11-R55 a *guard*.**
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
//! ## What M11-R55 changed, and why it is the durable half of that finding
//!
//! Until then the precision test was `#[ignore]`d — *"it prints numbers, it does not assert a
//! product bar"*. That sentence was true and the consequence was not survivable: this is the
//! **only** precision harness the domestic-phone tier has, `cargo test` never ran it, and four
//! consecutive widenings of the separator axis (M11-R25 → R48 → R51/R52) each landed against
//! *coverage* assertions alone. `SEPARATOR-01`'s matrix and `RENDER-01`'s span assertion both
//! say a recorded rendering **is** detected; neither can express *"and this is not a phone
//! number"*, so the cost of a widening was invisible to the suite **by construction**. When
//! round 15 finally ran this file by hand, the union's `dates` rate was 0.270 against the 0.180
//! `ARCHITECTURE.md` published, and the slash rendering it reported as a hit was the one both
//! that file and this file's own comment called impossible.
//!
//! So the numbers stopped being printed and started being **asserted, against the document that
//! publishes them**. `docs/ARCHITECTURE.md` carries the measurement between
//! `<!-- PHONE-EVAL:BEGIN -->` and `<!-- PHONE-EVAL:END -->`, and this test renders the same
//! block from a live run and requires them to be equal. That is the chokepoint rather than a
//! list of expected constants: one side is a measurement of the product, the other is what the
//! operator reads, and no third place can drift because there is no third place. **A number in
//! prose is either asserted from the code, or not written.**
//!
//! It costs ~5 s in the debug profile, which is why it can simply run.
//!
//! ## Running
//!
//! `phone_precision_per_region_and_for_the_union` runs in a plain `cargo test`. The **latency**
//! test stays `#[ignore]`d — it is milliseconds, and milliseconds are not build-independent:
//!
//! ```text
//! cargo test --release --test phone_eval -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **`--release` and `--test-threads=1` are both part of that contract** (M7-R12): a debug
//! build measures the wrong constant factor, and cargo's default concurrency measures the
//! product against other copies of itself. The precision figures are build-independent; the
//! milliseconds are not.

#[path = "common/m7_turn.rs"]
mod m7_turn;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde::Deserialize;

use llm_proxy_pii_rust::pii::recognizers::{
    StructuredRecognizers, PHONE_REGIONS, SEPARATOR_RUN_MAX,
};
use llm_proxy_pii_rust::pii::{PiiDetector, PiiKind};

const CORPUS_JSON: &str = include_str!("corpus/pii_cases.json");

/// The markers in `docs/ARCHITECTURE.md` that fence the published measurement.
const BLOCK_BEGIN: &str = "<!-- PHONE-EVAL:BEGIN -->";
const BLOCK_END: &str = "<!-- PHONE-EVAL:END -->";

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

/// Whether `pattern` matches anywhere in `input`. Compiled per call — this runs a few times
/// over a few hundred short strings, and a lazy static would be more machinery than the
/// measurement deserves.
fn regex_lite_contains(input: &str, pattern: &str) -> bool {
    regex::Regex::new(pattern).unwrap().is_match(input)
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

    // Dates, in the four renderings a document actually carries. **The comment that stood
    // here was false at HEAD and is the reason M11-R55 exists:** it said ISO (`2026-07-29`)
    // *and* slash (`29/07/2026`) renderings "cannot collide at all — no family accepts `/`".
    // Round 14 gave the domestic families `/` and `.`, so `28/01/2026` became a candidate in
    // the same commit that made the sentence false, and this harness went on generating the
    // string while its comment explained why the string could not matter. The ISO half still
    // holds — a 4-digit leading group is not a candidate for any family.
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

    // ---------------------------------------------------------------------------------
    // **Shapes the un-anchored families can actually reach (M10-R3).**
    //
    // Everything above was written by imagining plausible text, and it turned out to be
    // structurally unable to test half of what M10 added: an un-anchored candidate needs a
    // **2–3-digit leading token**, and almost nothing above has one — `chunk 8192 bytes`,
    // `order 2026 1042`, `port 8080` all start with four digits, which is not a candidate at
    // all. The second family (`[1-9]\d{2}[ -]\d{6,8}`) had essentially no representative.
    //
    // So the published per-category zeros were reporting on the pool's shape, not the
    // detector's precision — *a corpus has a shape, and that shape is a blind spot*
    // (M4-R13), landing on the milestone's own deliverable measurement. These entries are
    // generated **from the families' own structure** rather than from imagination, which is
    // the only way a pool can honestly claim to cover them.
    // ---------------------------------------------------------------------------------
    for (i, lead) in [12u32, 42, 99, 100, 256, 512, 800, 913]
        .into_iter()
        .enumerate()
    {
        let k = i as u32;
        // 2–3-digit token + a 6–8-digit block — file offsets, byte counts, order numbers,
        // and `YYYYMMDD` as a second field.
        out.push((
            "sizes",
            format!("chunk {lead} {} bytes read", 1_048_576 + k),
        ));
        out.push((
            "offsets",
            format!("offset {lead} {} in file", 1_000_000 + k * 7),
        ));
        out.push(("refs", format!("order {lead} {} shipped", 4_500_000 + k)));
        out.push((
            "dates",
            format!("row {lead} 2026{:02}{:02} inserted", 1 + k % 12, 1 + k % 28),
        ));
        // 2–3-digit tokens in tabular runs — the shape a CSV column or a spreadsheet has.
        out.push((
            "tables",
            format!(
                "cell {lead} {} {} {} of the sheet",
                100 + k,
                200 + k,
                300 + k
            ),
        ));
        out.push(("tables", format!("row {lead} {} {} totals", 10 + k, 20 + k)));
    }

    ip_and_alignment_negatives(&mut out);
    out
}

/// A deterministic 64-bit LCG. **Not `rand`**: the pool has to be byte-identical on every box
/// and every run, because its rates are now asserted against a published document. A seeded
/// generator in the test is the whole reproducibility argument.
fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

/// **The shapes round 14's separator widening admitted** — a dotted-decimal IPv4 address and a
/// column-aligned numeric row (M11-R55).
///
/// Neither existed in this pool, and neither could: before round 14 no domestic family accepted
/// `.` and every gap was exactly one character, so both were structurally impossible candidates.
/// After it, `62.30.40.50` is the digit run `62304050` — a valid Latvian landline — and
/// `170.75.154.131` is `17075154131`, a valid Chinese mobile, which is why the rate is high
/// rather than incidental: CN mobiles are 11 digits beginning `1`, exactly what a dotted quad
/// with a `1xx` first octet spells.
///
/// The private ranges are reported **separately from the public one and from each other**. They
/// are the addresses agent traffic actually carries, their leading octets are fixed, and a
/// blended "IP" rate over a mixture somebody chose would be a number about the mixture — this
/// harness's own first rule.
fn ip_and_alignment_negatives(out: &mut Vec<(&'static str, String)>) {
    let mut state = 0x5EED_1CE5_A11A_B1E5u64;

    // Public: anything that is not loopback, link-local, private, multicast or reserved.
    let mut made = 0;
    while made < 128 {
        let (a, b, c, d) = (
            (next(&mut state) % 224) as u32,
            (next(&mut state) % 256) as u32,
            (next(&mut state) % 256) as u32,
            (next(&mut state) % 256) as u32,
        );
        let private = a == 0
            || a == 10
            || a == 127
            || (a == 169 && b == 254)
            || (a == 172 && (16..32).contains(&b))
            || (a == 192 && b == 168)
            || a >= 224;
        if private {
            continue;
        }
        out.push((
            "ips",
            format!("peer {a}.{b}.{c}.{d} timed out after 3 retries"),
        ));
        made += 1;
    }
    for _ in 0..64 {
        let (b, c, d) = (
            next(&mut state) % 256,
            next(&mut state) % 256,
            next(&mut state) % 256,
        );
        out.push((
            "ips10",
            format!("peer 10.{b}.{c}.{d} timed out after 3 retries"),
        ));
    }
    for _ in 0..64 {
        let (c, d) = (next(&mut state) % 256, next(&mut state) % 256);
        out.push((
            "ips192",
            format!("peer 192.168.{c}.{d} timed out after 3 retries"),
        ));
    }
    for _ in 0..64 {
        let (b, c, d) = (
            16 + next(&mut state) % 16,
            next(&mut state) % 256,
            next(&mut state) % 256,
        );
        out.push((
            "ips172",
            format!("peer 172.{b}.{c}.{d} timed out after 3 retries"),
        ));
    }

    // Column alignment: four numeric columns at every gap the separator run **admits**, and one
    // gap **outside** it. Both pools are derived from `SEPARATOR_RUN_MAX` rather than written as
    // literals, which is M11-R61's fix: widening the constant from 4 to 5 used to leave the whole
    // suite green, because no corpus anywhere held a run of five. Now it changes the size of both
    // pools *and* moves `alignedwide` off 0.000, and the published block is asserted byte for byte.
    //
    // `alignedwide`'s **0.000 is the point**: the bound's residue published as a number, the same
    // way the admission is. It is the one category here whose value is expected to stay at zero.
    for gap in 2..=SEPARATOR_RUN_MAX {
        let spaces = " ".repeat(gap);
        for _ in 0..48 {
            let cols: Vec<u64> = (0..4).map(|_| 100 + next(&mut state) % 900).collect();
            out.push((
                "aligned",
                cols.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(&spaces),
            ));
        }
    }
    let wide = " ".repeat(SEPARATOR_RUN_MAX + 1);
    for _ in 0..48 {
        let cols: Vec<u64> = (0..4).map(|_| 100 + next(&mut state) % 900).collect();
        out.push((
            "alignedwide",
            cols.iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(&wide),
        ));
    }
}

/// The category names in the generated pool, in report order.
const NEGATIVE_CATEGORIES: &[&str] = &[
    "dates",
    "ports",
    "sizes",
    "offsets",
    "money",
    "codes",
    "refs",
    "tables",
    "ips",
    "ips10",
    "ips192",
    "ips172",
    "aligned",
    "alignedwide",
];

/// One region set's measurement: recall over the positives it owns, the curated false-positive
/// rate, the per-category generated rate, and **which** generated strings it flagged.
///
/// The last field exists so the dispatch invariant below can be checked without a second pass
/// over the pool — ten more `detect` sweeps was most of this test's runtime, and the test now
/// runs on every `cargo test`.
struct Measured {
    recall: f64,
    fp_curated: f64,
    per_category: Vec<f64>,
    flagged: Vec<bool>,
}

/// Score one region set: recall over the positives whose `locale` is in `owned`, plus the
/// false positives it produces on the curated and the generated pools.
fn report(
    label: &str,
    detector: &StructuredRecognizers,
    cases: &Category,
    owned: &[&str],
    generated: &[(&'static str, String)],
) -> Measured {
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

    // One sweep of the generated pool, kept as a mask: every rate below is counted off it, and
    // so is the dispatch invariant.
    let flagged: Vec<bool> = generated
        .iter()
        .map(|(_, s)| !phones(detector, s).is_empty())
        .collect();

    let mut per_category = Vec::with_capacity(NEGATIVE_CATEGORIES.len());
    let mut worst: Option<(String, Vec<String>)> = None;
    for category in NEGATIVE_CATEGORIES {
        let idx: Vec<usize> = generated
            .iter()
            .enumerate()
            .filter(|(_, (c, _))| c == category)
            .map(|(i, _)| i)
            .collect();
        let bad: Vec<usize> = idx.iter().copied().filter(|i| flagged[*i]).collect();
        per_category.push(bad.len() as f64 / idx.len() as f64);
        if worst.as_ref().is_none_or(|(_, w)| w.len() < bad.len()) {
            worst = Some((
                (*category).to_string(),
                bad.iter()
                    .take(3)
                    .map(|i| generated[*i].1.clone())
                    .collect(),
            ));
        }
    }

    print!(
        "{label:<7} recall {recall:>5.3} ({hits:>2}/{total:<2})  FPcur {:>5.3} ({}/{})  ",
        fp_curated as f64 / cases.negative.len() as f64,
        fp_curated,
        cases.negative.len()
    );
    for (category, rate) in NEGATIVE_CATEGORIES.iter().zip(&per_category) {
        print!("{category} {rate:>5.3} ");
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

    Measured {
        recall,
        fp_curated: fp_curated as f64 / cases.negative.len() as f64,
        per_category,
        flagged,
    }
}

/// Render the measurement exactly as `docs/ARCHITECTURE.md` publishes it.
///
/// **Transposed on purpose** — one row per quantity, one column per region set. Categories are
/// rows because rows are cheap: a new over-mask class can be measured and published without
/// reflowing a table, which is the friction that let `dates 0.180` stand for four rounds.
fn render_block(
    positives: usize,
    curated: usize,
    generated: &[(&'static str, String)],
    columns: &[(String, Measured)],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "pool: {positives} corpus positives · {curated} curated negatives · {} generated\n",
        generated.len()
    ));
    let sizes: Vec<String> = NEGATIVE_CATEGORIES
        .iter()
        .map(|c| format!("{c} {}", generated.iter().filter(|(g, _)| g == c).count()))
        .collect();
    out.push_str(&format!("generated: {}\n", sizes.join(" · ")));
    out.push_str(&format!("{:<11}", "region"));
    for (label, _) in columns {
        out.push_str(&format!("{label:>7}"));
    }
    out.push('\n');
    let mut row = |name: &str, values: Vec<f64>| {
        out.push_str(&format!("{name:<11}"));
        for v in values {
            out.push_str(&format!("{v:>7.3}"));
        }
        out.push('\n');
    };
    row(
        "recall",
        columns.iter().map(|(_, m)| m.recall).collect::<Vec<_>>(),
    );
    row(
        "curatedFP",
        columns
            .iter()
            .map(|(_, m)| m.fp_curated)
            .collect::<Vec<_>>(),
    );
    for (i, category) in NEGATIVE_CATEGORIES.iter().enumerate() {
        row(
            category,
            columns
                .iter()
                .map(|(_, m)| m.per_category[i])
                .collect::<Vec<_>>(),
        );
    }
    out
}

/// The block `docs/ARCHITECTURE.md` publishes, normalised for comparison: the fence lines and
/// any blank edges dropped, every line right-trimmed.
fn normalise(block: &str) -> String {
    block
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim_start().starts_with("```"))
        .skip_while(|l| l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// **Per region, then the union.** The second is not predicted by the first — enabling N
/// regions unions their accepted sets, so the compound rate is ≥ the worst single one.
///
/// **Not `#[ignore]`d since M11-R55**, and that is the finding rather than a note about it: the
/// four widenings this milestone shipped were each measured by nobody, because the only harness
/// that could measure them was one a human had to remember to run.
#[test]
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

    // **The pool must be able to reach every shape family, or its zeros mean "unmeasured"
    // rather than "clean" (M10-R3).** Checked by candidate shape, not by outcome: a family is
    // reached when the pool contains strings its regex proposes, whether or not any region
    // accepts them.
    let reach = |name: &str, sample: &dyn Fn(&str) -> bool| {
        let n = generated.iter().filter(|(_, s)| sample(s)).count();
        assert!(
            n >= 5,
            "the generated pool contains only {n} strings that could reach the {name} shape \
             family — a pool that cannot reach a family cannot report on it, and today it \
             reports 0.000 instead of 'unmeasured'"
        );
        println!("  pool reaches {name}: {n} strings");
    };
    reach("trunk", &|s: &str| {
        regex_lite_contains(s, r"(?-u:\b)0\d{1,4}[ -]\d")
    });
    reach("un-anchored groups", &|s: &str| {
        regex_lite_contains(s, r"(?-u:\b)[1-9]\d{1,2}[ -]\d{2,4}(?-u:\b)")
    });
    reach("un-anchored long block", &|s: &str| {
        regex_lite_contains(s, r"(?-u:\b)[1-9]\d{2}[ -]\d{6,8}(?-u:\b)")
    });
    // **The two shapes M11-R55 admitted have to be reachable too**, and by the same rule: the
    // pool's `ips` and `aligned` zeros would otherwise mean "not in the pool".
    reach("dotted quad", &|s: &str| {
        regex_lite_contains(s, r"(?-u:\b)\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(?-u:\b)")
    });
    reach("column alignment", &|s: &str| {
        regex_lite_contains(s, r"[0-9]{2,3}[ ]{2,4}[0-9]{2,4}")
    });
    // And **one gap outside the bound**, which is the pool M11-R61 was missing: without it,
    // widening `SEPARATOR_RUN_MAX` from 4 to 5 left the entire suite green.
    let wide = format!(
        r"[0-9]{{2,3}}[ ]{{{},{}}}[0-9]{{2,4}}",
        SEPARATOR_RUN_MAX + 1,
        SEPARATOR_RUN_MAX + 1
    );
    reach("column alignment beyond the run bound", &|s: &str| {
        regex_lite_contains(s, &wide)
    });
    println!();

    let mut columns: Vec<(String, Measured)> = Vec::new();
    for region in PHONE_REGIONS {
        let detector = StructuredRecognizers::with_regions(&[region.id]);
        let m = report(region.code, &detector, &cases, &[region.code], &generated);
        columns.push((region.code.to_string(), m));
    }

    let all: Vec<&str> = PHONE_REGIONS.iter().map(|r| r.code).collect();
    let detector = StructuredRecognizers::new();
    println!();
    let union = report("UNION", &detector, &cases, &all, &generated);

    // **A dispatch invariant, asserted — not a result reported (M10-R6).**
    //
    // "Union-only false positives: 0" was published as a discovered fact and cited as the
    // evidence that made all-on-by-default safe. Under shape (b) it cannot be anything else:
    // the candidate regexes are region-independent and the validator is `.any()` over the
    // enabled set, so the union's recognizers accept a **superset** of any single region's and
    // union acceptance implies single-region acceptance. It is pinned at 0 for every possible
    // pool, which is exactly what makes it an assertion rather than a measurement.
    //
    // What it *does* mean is worth keeping: **no emergent false positives** — a candidate the
    // union masks is always one some enabled region masks alone — so a region's measured cost
    // is also its marginal cost. What it does **not** mean is that adding a region is free:
    // the union's FP set still grows by set-union, which is what the per-category table above
    // shows and what decides the default.
    let union_hits = union.flagged.iter().filter(|f| **f).count();
    let any_single_hits = (0..generated.len())
        .filter(|i| columns.iter().any(|(_, m)| m.flagged[*i]))
        .count();
    assert_eq!(
        union_hits, any_single_hits,
        "the dispatch is supposed to make union acceptance equivalent to acceptance by some \
         enabled region alone. It no longer is — a change to `national_phone_recognizers` \
         made the union produce EMERGENT false positives, which the per-region numbers above \
         can no longer predict."
    );
    println!("\ndispatch invariant holds: union hits {union_hits} == ∪ singles {any_single_hits}");

    // ---------------------------------------------------------------------------------
    // **The published table is this run's output, or the test is red** (M11-R55).
    //
    // Not "assert each rate against a constant here, and separately write the same constants
    // into the document" — that is two lists checking each other, which M11-R53 named as
    // bookkeeping. One side of this comparison is a measurement of the product and the other is
    // the sentence an operator reads before upgrading. There is no third copy to drift.
    // ---------------------------------------------------------------------------------
    columns.push(("UNION".to_string(), union));
    let rendered = render_block(
        cases
            .positive
            .iter()
            .filter(|c| all.contains(&c.locale.as_deref().unwrap_or("")))
            .count(),
        cases.negative.len(),
        &generated,
        &columns,
    );

    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/ARCHITECTURE.md");
    let doc = fs::read_to_string(&doc_path).expect("docs/ARCHITECTURE.md is readable");
    let begin = doc.find(BLOCK_BEGIN).unwrap_or_else(|| {
        panic!(
            "docs/ARCHITECTURE.md has lost its {BLOCK_BEGIN} marker — the \
             domestic-phone rates it publishes are asserted from this test, and without the \
             marker nothing checks them"
        )
    });
    let end = doc
        .find(BLOCK_END)
        .unwrap_or_else(|| panic!("docs/ARCHITECTURE.md has lost its {BLOCK_END} marker"));
    assert!(begin < end, "the PHONE-EVAL markers are in the wrong order");
    let published = normalise(&doc[begin + BLOCK_BEGIN.len()..end]);

    assert_eq!(
        published,
        normalise(&rendered),
        "\n\nthe domestic-phone precision measurement no longer matches what \
         `docs/ARCHITECTURE.md` publishes.\n\nIf a recognizer was widened or narrowed on \
         purpose, that is a **product-visible precision change**: say so in CHANGELOG.md's \
         `[Unreleased]`, and paste the block below between the PHONE-EVAL markers.\n\n\
         ------------------------------ measured now ------------------------------\n\
         {rendered}\
         --------------------------------------------------------------------------\n"
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

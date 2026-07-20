//! Execution-provider benchmark (M9) — measure, **on this machine**, which backend is
//! actually fastest for the configured NER model, and say so plainly.
//!
//! **Why this ships in the binary instead of living in a test.** The answer is
//! hardware-specific and we cannot answer it for the operator. M9 measured an AMD DX12
//! **iGPU** where DirectML *loses* to 12 CPU threads at the sequence lengths that matter
//! (0.38× at seq 512) — while a **discrete** GPU would very likely win. Shipping a number
//! measured on our box would be worse than shipping nothing; shipping the *measuring tool*
//! lets each machine answer for itself. Run it with `--bench-providers`.
//!
//! **The trap this deliberately encodes.** The first M9 measurement ran the shipped
//! **int8** model on DirectML, showed the GPU 2–5× *slower*, and looked like a verdict. It
//! was a **false negative**: int8 is a CPU-optimized format whose ops partition badly onto
//! GPU execution providers. Re-measured at **fp16**, the same GPU went from 5× slower to
//! 1.45× faster at short sequences. So the report always warns about quantization — because
//! the naive reading of a bad GPU number is the wrong conclusion, and it is the exact
//! mistake this milestone made before catching it.

use std::time::Instant;

use anyhow::{anyhow, Result};
use ort::session::Session;
use ort::value::Tensor;

use super::onnx::{available_cores, build_session_reporting, ExecutionProvider, MAX_WINDOW_TOKENS};

/// Sequence lengths measured, and **512 is the one that decides.** Fields are chunked to
/// [`MAX_WINDOW_TOKENS`] (480), so a full window runs near 512, and long fields are what
/// make a slow turn. A provider that wins at 128 but loses at 512 has not won: the 128 case
/// is already fast enough that the difference is invisible.
pub const BENCH_SEQS: [usize; 3] = [128, 256, 512];

const WARMUP: usize = 3;
const RUNS: usize = 20;

/// One (model, provider) measurement.
pub struct ProviderResult {
    /// The model file this row measured. The choice of backend and the choice of
    /// **quantization** are coupled — CPU wants int8, a GPU wants fp16 — so a row is only
    /// meaningful together with the model it ran.
    pub model: String,
    /// The provider we asked for.
    pub requested: ExecutionProvider,
    /// What it actually ran on — `Cpu` when the accelerator fell back. When this differs
    /// from `requested`, the timings are **CPU timings** and must not be read as the
    /// accelerator's.
    pub effective: ExecutionProvider,
    /// Mean ms per inference, one per [`BENCH_SEQS`] entry. Empty when `error` is set.
    pub ms: Vec<f64>,
    pub error: Option<String>,
}

impl ProviderResult {
    /// Whether the accelerator we asked for is the one that ran. CPU trivially satisfies this.
    pub fn engaged(&self) -> bool {
        self.requested == self.effective
    }
}

/// The providers this binary can actually use: CPU plus every accelerator the **linked ONNX
/// Runtime distribution was compiled with**.
///
/// **Asked of the runtime, not inferred from cargo features — and the difference is not
/// academic.** `download-binaries` fetches ONE ONNX Runtime distribution, and *that* decides
/// which execution providers exist; a cargo feature only decides whether our `register()` is
/// compiled. The two can disagree in both directions:
///
/// - Enabling six `ep-*` features on Windows still links the DirectML distribution —
///   **measured**: cuda / tensorrt / coreml / rocm / openvino then report unavailable and fall
///   back to CPU. A feature-derived list would have shown five GPU rows that were CPU.
/// - The platform accelerator is set per-target in `Cargo.toml` (Windows x64 → DirectML), so
///   it is present *without* the `ep-directml` feature being named. A cfg-derived list would
///   miss the one accelerator the machine actually has.
///
/// [`ExecutionProvider::is_available`](ort::ep::ExecutionProvider::is_available) answers the
/// only question that matters — is it in the binary — so the list cannot lie in either
/// direction. It is still not a promise the EP will *run* a given model (ORT may reject it at
/// session creation, or partition nodes back to CPU); that is why the per-row fallback
/// reporting stays.
pub fn available_providers() -> Vec<ExecutionProvider> {
    // Import the ort trait anonymously: its name collides with our own enum, and we want only
    // the `is_available` method, not the type.
    use ort::ep::ExecutionProvider as _;

    let all = [
        ExecutionProvider::DirectMl,
        ExecutionProvider::Cuda,
        ExecutionProvider::TensorRt,
        ExecutionProvider::CoreMl,
        ExecutionProvider::Rocm,
        ExecutionProvider::OpenVino,
        ExecutionProvider::WebGpu,
    ];

    let mut providers = vec![ExecutionProvider::Cpu]; // always present, always the baseline
    for provider in all {
        let available = match provider {
            ExecutionProvider::DirectMl => ort::ep::DirectML::default().is_available(),
            ExecutionProvider::Cuda => ort::ep::CUDA::default().is_available(),
            ExecutionProvider::TensorRt => ort::ep::TensorRT::default().is_available(),
            ExecutionProvider::CoreMl => ort::ep::CoreML::default().is_available(),
            ExecutionProvider::Rocm => ort::ep::ROCm::default().is_available(),
            ExecutionProvider::OpenVino => ort::ep::OpenVINO::default().is_available(),
            ExecutionProvider::WebGpu => ort::ep::WebGPU::default().is_available(),
            ExecutionProvider::Cpu => Ok(true),
        };
        // An error here means ORT itself is unhealthy; treat it as "not available" rather than
        // failing the benchmark — the CPU row still gives the operator something.
        if available.unwrap_or(false) {
            providers.push(provider);
        }
    }
    providers
}

/// What to tell an operator whose build measured **no** accelerator.
///
/// The right answer is OS-specific — there is no single GPU execution provider that spans
/// platforms (DirectML is Windows-only, CoreML is Apple-only, Linux is vendor-split *and* has no
/// prebuilt for arm64), so this names the situation on the machine actually running it rather
/// than dumping a generic list the operator then has to filter.
///
/// **This must not advise a rebuild that would change nothing (M9-R14).** The platform's
/// accelerator is wired per-target in `Cargo.toml`, so on every platform where one exists it is
/// *already compiled in* — telling that operator to `cargo build --features ep-cuda` sends them
/// to rebuild something they already have, and the likeliest place the message fires is arm64
/// Linux, i.e. exactly where that advice cannot help. So the guidance is the platform's real
/// situation, not a feature flag.
fn no_accelerator_guidance() -> &'static str {
    guidance_for(platform_from(std::env::consts::OS, std::env::consts::ARCH))
}

/// Classify a target from `(os, arch)` strings — **the selection half, kept testable (M9-R27).**
///
/// M9-R25 moved the *messages* onto the testable side of `cfg!` and left the *selection* on the
/// untestable side, so nothing on any machine asserted that macOS maps to [`Platform::MacOs`], or
/// that the `x86_64` Linux case is matched **before** bare Linux — invert that order and every
/// x86_64 Linux operator is told no accelerator is wired for their architecture, on a build that
/// has CUDA. Taking `std::env::consts::{OS, ARCH}` instead of `cfg!` is behaviour-identical (both
/// are fixed for the compiled target) and makes the mapping a pure, testable function.
fn platform_from(os: &str, arch: &str) -> Platform {
    match os {
        "windows" => Platform::Windows,
        "macos" => Platform::MacOs,
        // Order matters, and it is the one thing the `cfg!` chain hid from tests.
        "linux" if arch == "x86_64" => Platform::LinuxX86,
        "linux" => Platform::LinuxOther,
        _ => Platform::Unknown,
    }
}

/// The platforms [`no_accelerator_guidance`] distinguishes. Separate from the target constants so
/// every arm is testable everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Windows,
    MacOs,
    LinuxX86,
    LinuxOther,
    Unknown,
}

/// The message for each platform — pure, total, and exhaustively tested.
fn guidance_for(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => {
            "DirectML is compiled into every Windows build of this proxy, so a missing row means \
             the GPU/driver did not present a DX12 device to ONNX Runtime — check the vendor driver."
        }
        Platform::MacOs => {
            "CoreML is compiled into every macOS build of this proxy, so a missing row means ONNX \
             Runtime did not initialize it on this machine."
        }
        Platform::LinuxX86 => {
            "CUDA is compiled into x86_64 Linux builds of this proxy, so a missing row usually \
             means the CUDA runtime is not installed or no NVIDIA device is visible."
        }
        Platform::LinuxOther => {
            "On this architecture (non-x86_64 Linux) no accelerator is wired: ONNX Runtime ships \
             no CUDA prebuilt for it, so `cpu` is the supported path. A from-source ONNX Runtime \
             build would be required to change that."
        }
        Platform::Unknown => "No accelerator is known for this platform; `cpu` is the supported path.",
    }
}

/// One forward pass at `seq` tokens. Token *values* don't change the graph work, so
/// synthetic ids of the right shape measure exactly what real text would — and, unlike real
/// text, they cannot put PII in a benchmark harness.
///
/// **The input set must match the detector's, or the benchmark cannot run the model at all
/// (M9-R8).** A BERT-family model (`NER_TOKEN_TYPE_IDS=1`, e.g. Piiranha) declares a third
/// required graph input, and ORT rejects a `run()` that omits it — so every row would fail with
/// a raw ORT error that never names the real cause. `resolve_ner_model` was extracted so the
/// model *path* could not drift between the server and this tool; the model's *input contract*
/// has to travel with it.
fn run_once(session: &mut Session, seq: usize, needs_token_type_ids: bool) -> Result<()> {
    let ids: Vec<i64> = (0..seq).map(|i| (i % 250_000) as i64).collect();
    let mask: Vec<i64> = vec![1; seq];
    let input_ids =
        Tensor::from_array(([1, seq], ids)).map_err(|e| anyhow!("input_ids tensor: {e}"))?;
    let attention_mask =
        Tensor::from_array(([1, seq], mask)).map_err(|e| anyhow!("attention_mask tensor: {e}"))?;
    if needs_token_type_ids {
        // Zeros, exactly as `run_and_decode` sends for a single-segment sequence.
        let type_ids: Vec<i64> = vec![0; seq];
        let token_type_ids = Tensor::from_array(([1, seq], type_ids))
            .map_err(|e| anyhow!("token_type_ids tensor: {e}"))?;
        session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask,
                "token_type_ids" => token_type_ids,
            ])
            .map_err(|e| anyhow!("ONNX run: {e}"))?;
    } else {
        session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask,
            ])
            .map_err(|e| anyhow!("ONNX run: {e}"))?;
    }
    Ok(())
}

/// Build a session on `provider` and time it across [`BENCH_SEQS`].
fn benchmark_one(
    model_path: &str,
    intra_threads: usize,
    provider: ExecutionProvider,
    needs_token_type_ids: bool,
) -> ProviderResult {
    let failed = |effective, e: String| ProviderResult {
        model: model_path.to_string(),
        requested: provider,
        effective,
        ms: Vec::new(),
        error: Some(e),
    };

    let (mut session, effective) =
        match build_session_reporting(model_path, intra_threads, provider) {
            Ok(pair) => pair,
            Err(e) => return failed(provider, e.to_string()),
        };

    let mut ms = Vec::with_capacity(BENCH_SEQS.len());
    for &seq in &BENCH_SEQS {
        for _ in 0..WARMUP {
            if let Err(e) = run_once(&mut session, seq, needs_token_type_ids) {
                return failed(effective, e.to_string());
            }
        }
        let start = Instant::now();
        for _ in 0..RUNS {
            if let Err(e) = run_once(&mut session, seq, needs_token_type_ids) {
                return failed(effective, e.to_string());
            }
        }
        ms.push(start.elapsed().as_secs_f64() * 1000.0 / RUNS as f64);
    }

    ProviderResult {
        model: model_path.to_string(),
        requested: provider,
        effective,
        ms,
        error: None,
    }
}

/// Measure the full **model × provider** matrix.
///
/// Both axes matter and neither alone answers the question: the shipped CPU model is int8
/// and the format a GPU wants is fp16, so "is the GPU worth it?" is a comparison *across*
/// quantizations (CPU-int8 vs GPU-fp16), not within one. Benchmarking a single model across
/// providers — the first version of this — can only say "for this file, which backend wins",
/// which is exactly how int8-on-GPU produced a confident wrong answer during M9.
/// `intra_threads` is the **server's** resolved per-session thread count, not the core count
/// (M9-R3). Measuring the CPU with more threads than production gives it makes the baseline
/// optimistic and biases the recommendation *against* the accelerator — on exactly the tuned,
/// centralized deployment where an accelerator is most likely to be worth having. This is
/// [M7-R1](../../docs/reviews/M7.md)'s failure class, and `resolve_pool_and_intra` exists to
/// make it impossible, so the caller resolves through it.
pub fn benchmark_matrix(
    model_paths: &[String],
    intra_threads: usize,
    needs_token_type_ids: bool,
) -> Vec<ProviderResult> {
    let providers = available_providers();
    let mut results = Vec::with_capacity(model_paths.len() * providers.len());
    for model in model_paths {
        for &provider in &providers {
            results.push(benchmark_one(
                model,
                intra_threads,
                provider,
                needs_token_type_ids,
            ));
        }
    }
    results
}

/// The index into [`BENCH_SEQS`] that the recommendation is made on — the largest, i.e.
/// the chunked-field operating point (see [`BENCH_SEQS`]).
fn decision_index() -> usize {
    BENCH_SEQS.len() - 1
}

/// Short label for a model file — the file stem, which is what distinguishes
/// `model_quantized` from `model_fp16` in a matrix report.
fn model_label(path: &str) -> &str {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.strip_suffix(".onnx").unwrap_or(file)
}

/// Render the human-facing report: one table section per model, what actually engaged, the
/// **overall** recommendation across the model × provider matrix, and the caveats that keep
/// a bad number from being misread.
pub fn format_report(
    model_paths: &[String],
    results: &[ProviderResult],
    pool_size: usize,
    intra_threads: usize,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let _ = writeln!(out, "Execution-provider benchmark (M9)");
    // The EFFECTIVE server shape, resolved through `resolve_pool_and_intra` — not the raw core
    // count, which is only the same thing at the shipped default (M9-R3).
    let _ = writeln!(
        out,
        "  threads: {intra_threads} intra-op per session, pool {pool_size} (of {} cores)",
        available_cores()
    );
    let _ = writeln!(
        out,
        "  models: {}\n",
        model_paths
            .iter()
            .map(|m| model_label(m))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // One section per model — the two axes are independent and mixing them in one flat table
    // hides which quantization a row belongs to.
    for model in model_paths {
        let _ = writeln!(out, "{}", model);
        let _ = write!(out, "  {:<12}", "provider");
        for seq in BENCH_SEQS {
            let _ = write!(out, " | {:>10}", format!("seq {seq}"));
        }
        let _ = writeln!(out, " | status");
        let _ = writeln!(out, "  {}", "-".repeat(12 + BENCH_SEQS.len() * 13 + 30));

        for r in results.iter().filter(|r| &r.model == model) {
            let _ = write!(out, "  {:<12}", r.requested.as_str());
            for i in 0..BENCH_SEQS.len() {
                match r.ms.get(i) {
                    Some(v) => {
                        let _ = write!(out, " | {v:>9.1}ms");
                    }
                    None => {
                        let _ = write!(out, " | {:>10}", "—");
                    }
                }
            }
            let status = if let Some(e) = &r.error {
                format!("FAILED: {e}")
            } else if !r.engaged() {
                format!(
                    "unavailable — fell back to {} (these are {} timings)",
                    r.effective.as_str(),
                    r.effective.as_str()
                )
            } else {
                "ok".to_string()
            };
            let _ = writeln!(out, " | {status}");
        }
        let _ = writeln!(out);
    }

    // Recommendation over the WHOLE matrix, at the decisive sequence length, and only over
    // rows that genuinely engaged (a fallback row is a duplicate of CPU wearing a GPU's name).
    let idx = decision_index();
    let best = results
        .iter()
        .filter(|r| r.error.is_none() && r.engaged())
        .filter_map(|r| r.ms.get(idx).map(|&v| (r.model.as_str(), r.requested, v)))
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let _ = writeln!(
        out,
        "Decision is made at seq {} — fields are chunked to {} tokens, so a full window\n\
         runs near there, and long fields are what make a slow turn.",
        BENCH_SEQS[idx], MAX_WINDOW_TOKENS
    );

    match best {
        Some((model, ExecutionProvider::Cpu, ms)) => {
            let _ = writeln!(
                out,
                "\n=> FASTEST: cpu on {} ({ms:.1}ms).\n\
                 \x20  Keep the default — leave NER_EXECUTION_PROVIDER unset, and point\n\
                 \x20  NER_MODEL_PATH at that model.",
                model_label(model)
            );
        }
        Some((model, provider, ms)) => {
            let _ = writeln!(
                out,
                "\n=> FASTEST: {} on {} ({ms:.1}ms).\n\
                 \x20  To use it:  NER_EXECUTION_PROVIDER={}  (with NER_MODEL_PATH set to that model)",
                provider.as_str(),
                model_label(model),
                provider.as_str()
            );
        }
        None => {
            let _ = writeln!(
                out,
                "\n=> No (model, provider) pair completed the benchmark."
            );
        }
    }

    // Measurement hygiene — learned the hard way: the same box measured ~3x slower right
    // after a long compile than when idle. The ranking held, but absolute numbers did not.
    let _ = writeln!(
        out,
        "\nMEASUREMENT: run this on an otherwise-idle machine and on AC power. A busy or\n\
         thermally-throttled box inflates every row (measured: ~3x right after a heavy build).\n\
         The ranking is far more robust than the absolute milliseconds."
    );

    // The caveat that stops a false negative from becoming a wrong decision.
    let looks_int8 = model_paths.iter().any(|p| {
        let p = p.to_ascii_lowercase();
        p.contains("quant") || p.contains("int8")
    });
    let has_fp16 = model_paths
        .iter()
        .any(|p| p.to_ascii_lowercase().contains("fp16"));
    let _ = writeln!(
        out,
        "\nNOTE ON QUANTIZATION — read this before concluding a GPU is slow.\n\
         int8 is a CPU-optimized format: its ops partition badly onto GPU providers, so an\n\
         int8 model can make a perfectly good GPU look 2-5x SLOWER than the CPU. fp16 is the\n\
         GPU format (on CPU it is up-cast to fp32, so it only pays off on a GPU). For a fair\n\
         GPU comparison, re-run this against an fp16 export of the same model."
    );
    // Read the shape off the results rather than re-querying ORT: the rows ARE the record of
    // what this run could measure.
    let measured_an_accelerator = results
        .iter()
        .any(|r| r.requested != ExecutionProvider::Cpu);
    if looks_int8 && !has_fp16 && measured_an_accelerator {
        let _ = writeln!(
            out,
            "\n  !! An int8/quantized model is in this run and NO fp16 model is, so every GPU\n\
             \x20    row here is very likely an UNDERESTIMATE — this comparison cannot answer\n\
             \x20    'is the GPU worth it?'. Add an fp16 export via NER_BENCH_MODELS to get the\n\
             \x20    honest matrix (CPU-int8 vs GPU-fp16)."
        );
    }
    // Which accelerators exist is decided by the ONNX Runtime distribution this binary linked.
    // When none is present the answer is NOT "rebuild with a feature" — the platform's
    // accelerator is already wired per-target — so the guidance describes what is actually true
    // of this machine (M9-R14/M9-R16).
    if !measured_an_accelerator {
        let _ = writeln!(
            out,
            "\nNo accelerator is present in this build, so only `cpu` could be measured.\n\
             Which providers exist is decided by the ONNX Runtime distribution linked at build\n\
             time: one distribution, one set of providers — enabling several `ep-*` features does\n\
             not combine them.\n\n\
             \x20 {}",
            no_accelerator_guidance()
        );
    }

    out
}

#[cfg(test)]
mod report_tests {
    use super::*;

    /// **BENCH-01 (M9-R20).** The "no accelerator" guidance must never name an `ep-*` feature.
    ///
    /// `CLI-03` spawns the real binary, but with no `NER_*` configured it bails at model
    /// resolution and never reaches `format_report` — and on a box whose platform accelerator IS
    /// present, `!measured_an_accelerator` is false anyway. So the branch that actually prints
    /// the guidance was unreachable from that test in the `onnx` build: a guard written so "only
    /// the branch a finding quoted" could not recur, itself covering only one branch.
    ///
    /// This drives `format_report` directly with a CPU-only result set, which is the only way to
    /// reach it deterministically on any machine. Asserting the seven concrete names rather than
    /// the string `ep-` matters: the report legitimately says "enabling several `ep-*` features
    /// does not combine them", and that sentence must stay.
    #[test]
    fn the_no_accelerator_guidance_never_names_a_cargo_feature() {
        let results = vec![ProviderResult {
            model: "model_quantized.onnx".to_string(),
            requested: ExecutionProvider::Cpu,
            effective: ExecutionProvider::Cpu,
            ms: vec![1.0; BENCH_SEQS.len()],
            error: None,
        }];
        let report = format_report(&["model_quantized.onnx".to_string()], &results, 1, 4);

        // Non-vacuity: this really is the branch under test.
        assert!(
            report.contains("No accelerator is present in this build"),
            "the CPU-only result set must reach the no-accelerator branch; got:\n{report}"
        );
        for feature in [
            "ep-directml",
            "ep-cuda",
            "ep-coreml",
            "ep-rocm",
            "ep-openvino",
            "ep-tensorrt",
            "ep-webgpu",
        ] {
            assert!(
                !report.contains(feature),
                "the report advised `{feature}`, but the platform accelerator is wired per-target \
                 and needs no such rebuild (M9-R14/R16/R20). Report was:\n{report}"
            );
        }
    }

    /// **BENCH-02 (M9-R25).** *Every* platform's guidance is checked, not just the running one.
    ///
    /// `no_accelerator_guidance` is a five-way `cfg!` chain, so a test that only calls it sees
    /// the single arm compiled for this machine — three of the five ship on platforms CI never
    /// runs. That is exactly the shape of M9-R16: a message that was wrong for macOS and Linux
    /// survived because nothing on Windows could observe it. Splitting the pure `guidance_for`
    /// out of the `cfg!` makes all five reachable here.
    #[test]
    fn every_platforms_guidance_is_sound_not_only_this_ones() {
        let all = [
            Platform::Windows,
            Platform::MacOs,
            Platform::LinuxX86,
            Platform::LinuxOther,
            Platform::Unknown,
        ];
        for platform in all {
            let msg = guidance_for(platform);
            assert!(!msg.is_empty(), "{platform:?}: guidance must not be empty");
            for feature in [
                "ep-directml",
                "ep-cuda",
                "ep-coreml",
                "ep-rocm",
                "ep-openvino",
                "ep-tensorrt",
                "ep-webgpu",
            ] {
                assert!(
                    !msg.contains(feature),
                    "{platform:?}: guidance named `{feature}` — the platform accelerator is wired \
                     per-target, so no rebuild advice is correct (M9-R14/R16/R25). Got: {msg}"
                );
            }
        }
        // **Positive, per-case (M9-R27).** A distinctness check is invariant under PERMUTATION:
        // swap the Windows and macOS bodies and a set-size assertion still passes while a Mac user
        // is told about DirectML — the exact failure this test claims to prevent. (The reviewer
        // demonstrated it by mutating the function and watching the old assertion pass; the
        // assertions below were then re-verified against that same mutation, and they fail it.)
        assert!(guidance_for(Platform::Windows).contains("DirectML"));
        assert!(guidance_for(Platform::MacOs).contains("CoreML"));
        assert!(guidance_for(Platform::LinuxX86).contains("CUDA is compiled into x86_64 Linux"));
        assert!(guidance_for(Platform::LinuxOther).contains("no accelerator is wired"));
        assert!(guidance_for(Platform::Unknown).contains("No accelerator is known"));
    }

    /// **BENCH-03 (M9-R27).** The *selection* half, which M9-R25's refactor left untestable.
    ///
    /// Nothing asserted that macOS maps to `MacOs`, nor — the one that bites — that the `x86_64`
    /// Linux case is matched **before** bare Linux. Invert that order and every x86_64 Linux
    /// operator is told no accelerator is wired for their architecture, on a build that has CUDA.
    #[test]
    fn platform_classification_covers_every_target_and_orders_linux_correctly() {
        assert_eq!(platform_from("windows", "x86_64"), Platform::Windows);
        assert_eq!(platform_from("windows", "aarch64"), Platform::Windows);
        assert_eq!(platform_from("macos", "aarch64"), Platform::MacOs);
        assert_eq!(platform_from("macos", "x86_64"), Platform::MacOs);
        // The ordering guard: same OS, different arch, different answer.
        assert_eq!(platform_from("linux", "x86_64"), Platform::LinuxX86);
        assert_eq!(platform_from("linux", "aarch64"), Platform::LinuxOther);
        assert_eq!(platform_from("freebsd", "x86_64"), Platform::Unknown);

        // And the real target classifies to something we have a message for — the seam between
        // this pure function and `std::env::consts` that no table can cover.
        let actual = platform_from(std::env::consts::OS, std::env::consts::ARCH);
        assert!(
            !guidance_for(actual).is_empty(),
            "the running target ({}/{}) must classify to a platform with guidance",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }

    /// The header must echo the **resolved** shape it measured with, not the core count (M9-R3).
    #[test]
    fn the_header_reports_the_resolved_thread_shape() {
        let report = format_report(&["m.onnx".to_string()], &[], 2, 6);
        assert!(
            report.contains("6 intra-op per session, pool 2"),
            "header must state the resolved (pool, intra) the benchmark actually used; got:\n{report}"
        );
    }
}

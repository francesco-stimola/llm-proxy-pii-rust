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

/// The providers this binary can actually use: CPU plus every accelerator whose `ep-*`
/// feature was compiled in.
///
/// Benchmarking an *uncompiled* provider would silently measure the CPU fallback under a
/// GPU's name — a row that looks like evidence and is not. So the list is built from the
/// features, not from the enum.
pub fn compiled_providers() -> Vec<ExecutionProvider> {
    // `mut` goes unused in a build with no `ep-*` feature: every push below is cfg'd out and
    // the list stays `[Cpu]`. That is the *default* shape, not an oversight.
    #[allow(unused_mut)]
    let mut providers = vec![ExecutionProvider::Cpu];
    #[cfg(feature = "ep-directml")]
    providers.push(ExecutionProvider::DirectMl);
    #[cfg(feature = "ep-cuda")]
    providers.push(ExecutionProvider::Cuda);
    #[cfg(feature = "ep-tensorrt")]
    providers.push(ExecutionProvider::TensorRt);
    #[cfg(feature = "ep-coreml")]
    providers.push(ExecutionProvider::CoreMl);
    #[cfg(feature = "ep-rocm")]
    providers.push(ExecutionProvider::Rocm);
    #[cfg(feature = "ep-openvino")]
    providers.push(ExecutionProvider::OpenVino);
    #[cfg(feature = "ep-webgpu")]
    providers.push(ExecutionProvider::WebGpu);
    providers
}

/// The accelerator worth trying on **this** platform, as
/// `(NER_EXECUTION_PROVIDER value, cargo feature)`, plus a one-line reason.
///
/// The right answer is OS-specific — there is no single GPU execution provider that spans
/// platforms (DirectML is Windows-only, CoreML is Apple-only, Linux is vendor-split), so the
/// report names the one that fits the machine actually running it rather than dumping a
/// generic list the operator then has to filter.
pub fn suggested_provider_for_platform() -> Option<(&'static str, &'static str, &'static str)> {
    if cfg!(target_os = "windows") {
        Some((
            "directml",
            "ep-directml",
            "any DX12 GPU (AMD/NVIDIA/Intel, incl. integrated) — no CUDA, no admin",
        ))
    } else if cfg!(target_os = "macos") {
        Some((
            "coreml",
            "ep-coreml",
            "Apple Neural Engine / GPU — built into the OS",
        ))
    } else if cfg!(target_os = "linux") {
        Some((
            "cuda",
            "ep-cuda",
            "NVIDIA (needs the CUDA runtime); for AMD use `ep-rocm`, for Intel `ep-openvino`",
        ))
    } else {
        None
    }
}

/// One forward pass at `seq` tokens. Token *values* don't change the graph work, so
/// synthetic ids of the right shape measure exactly what real text would — and, unlike real
/// text, they cannot put PII in a benchmark harness.
fn run_once(session: &mut Session, seq: usize) -> Result<()> {
    let ids: Vec<i64> = (0..seq).map(|i| (i % 250_000) as i64).collect();
    let mask: Vec<i64> = vec![1; seq];
    let input_ids =
        Tensor::from_array(([1, seq], ids)).map_err(|e| anyhow!("input_ids tensor: {e}"))?;
    let attention_mask =
        Tensor::from_array(([1, seq], mask)).map_err(|e| anyhow!("attention_mask tensor: {e}"))?;
    session
        .run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => attention_mask,
        ])
        .map_err(|e| anyhow!("ONNX run: {e}"))?;
    Ok(())
}

/// Build a session on `provider` and time it across [`BENCH_SEQS`].
fn benchmark_one(
    model_path: &str,
    intra_threads: usize,
    provider: ExecutionProvider,
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
            if let Err(e) = run_once(&mut session, seq) {
                return failed(effective, e.to_string());
            }
        }
        let start = Instant::now();
        for _ in 0..RUNS {
            if let Err(e) = run_once(&mut session, seq) {
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
pub fn benchmark_matrix(model_paths: &[String]) -> Vec<ProviderResult> {
    let intra_threads = available_cores();
    let providers = compiled_providers();
    let mut results = Vec::with_capacity(model_paths.len() * providers.len());
    for model in model_paths {
        for &provider in &providers {
            results.push(benchmark_one(model, intra_threads, provider));
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
pub fn format_report(model_paths: &[String], results: &[ProviderResult]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let _ = writeln!(out, "Execution-provider benchmark (M9)");
    let _ = writeln!(
        out,
        "  cores : {} (CPU intra-op threads)",
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
    if looks_int8 && !has_fp16 && compiled_providers().len() > 1 {
        let _ = writeln!(
            out,
            "\n  !! An int8/quantized model is in this run and NO fp16 model is, so every GPU\n\
             \x20    row here is very likely an UNDERESTIMATE — this comparison cannot answer\n\
             \x20    'is the GPU worth it?'. Add an fp16 export via NER_BENCH_MODELS to get the\n\
             \x20    honest matrix (CPU-int8 vs GPU-fp16)."
        );
    }
    // Which accelerators this binary could even try is fixed at BUILD time, so when none was
    // compiled in, the actionable next step is a rebuild — named for *this* platform, since
    // the right EP differs per OS.
    if compiled_providers().len() == 1 {
        let _ = writeln!(
            out,
            "\nNo accelerator is compiled into this binary, so only `cpu` could be measured.\n\
             Which providers are available is a BUILD-time choice (each `ep-*` feature pulls a\n\
             different ONNX Runtime backend binary)."
        );
        match suggested_provider_for_platform() {
            Some((provider, feature, why)) => {
                let _ = writeln!(
                    out,
                    "\n  On this platform, try:  cargo build --features {feature}\n\
                     \x20   ({why})\n\
                     \x20 then re-run with --bench-providers to compare it against cpu,\n\
                     \x20 and enable it at runtime with NER_EXECUTION_PROVIDER={provider}."
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "\n  No accelerator is known for this platform; `cpu` is the supported path."
                );
            }
        }
    }

    out
}

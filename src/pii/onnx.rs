//! ONNX NER detector for UNSTRUCTURED entities (names, organizations,
//! locations) — milestone M2. Enabled by the `onnx` feature.
//!
//! **Execution providers (M9).** CPU is the default and the reference
//! implementation; an accelerator is opt-in via `NER_EXECUTION_PROVIDER` and is
//! **not** automatic — whether it is faster depends on the model, its
//! quantization, and the hardware (on this project's AMD iGPU the CPU wins; see
//! `ARCHITECTURE.md` → *Execution providers*, and `--bench-providers` for
//! measuring it on a given box). [`ExecutionProvider`] is the knob;
//! [`build_session_pool`] is the single home of the selection + CPU-fallback
//! policy, shared with [`gliner`](super::gliner); [`bench`](super::bench) is the
//! measurement tool built on top of it.
//!
//! This module owns only the model I/O: tokenize (with a HF fast tokenizer),
//! run the ONNX session, and turn per-token logits into label ids. The actual
//! label→[`PiiKind`](super::PiiKind) mapping and BIO→span merge live in the
//! model-independent [`ner_decode`](super::ner_decode), which is unit-tested
//! without a model.
//!
//! **Model contract:** a token-classification model with input `input_ids` +
//! `attention_mask` (and `token_type_ids` when `NER_TOKEN_TYPE_IDS` is set, e.g.
//! BERT-family models such as Piiranha) and a single output named `logits` of
//! shape `[1, seq, num_labels]`. `NER_LABELS` must list exactly `num_labels`
//! labels in class-id order — a mismatch is rejected (never silently degraded).
//!
//! **Chunking (M5, PERF-01).** A field longer than [`MAX_WINDOW_TOKENS`] is
//! split into overlapping windows rather than fed to the model whole. This
//! isn't a latency optimization: RoBERTa-family absolute position embeddings
//! top out at `max_position_embeddings` (514 for the picked XLM-R int8's
//! `config.json`), and a sequence past that limit makes the ONNX graph's
//! position-embedding lookup go **out of range** — measured
//! (`tests/ner_perf.rs`) as an outright `Expand` op failure, not a graceful
//! slowdown. Without chunking, any field over roughly 2 KB of prose (~500
//! tokens) fails NER outright — silently swallowed by the default fail-*open*
//! wrapper, but a hard **block** under `NER_REQUIRED` (every such request would
//! 400). See [`infer_chunked`](OnnxNerDetector::infer_chunked).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{Encoding, Tokenizer};

use super::ner_decode::{decode_entities, validate_label_count, TokenTag};
use super::overlap::widen_to_char_boundaries;
use super::{Budget, DetectError, PiiDetector, PiiEntity};

/// **The hard ceiling: the longest sequence the model can actually be handed.**
///
/// XLM-R declares `max_position_embeddings: 514`, but RoBERTa-family position ids start at
/// `pad_token_id + 1 = 2`, so the *usable* length is **512**. Past it the position-embedding
/// lookup goes out of range and the ONNX graph fails outright (the PERF-01 `Expand` error).
///
/// This is enforced where it matters — in [`run_and_decode`](OnnxNerDetector::run_and_decode),
/// the single choke point every path into the session goes through (M5-R2). It is *not* enough
/// to plan windows under a budget and hope: [`infer_chunked`](OnnxNerDetector::infer_chunked)
/// **re-tokenizes** each window from its own text, which adds the two special tokens and drifts
/// at the cut edges — measured at consistently **481–483** tokens for a 480-token window, i.e.
/// always *over* the planning bound. The drift is small for this tokenizer, but nothing
/// structurally bounds it, and the cost of being wrong is exactly the failure chunking exists to
/// prevent. So the bound is *checked*, not assumed.
pub const MODEL_MAX_TOKENS: usize = 512;

/// **The planning window** used to lay out chunks — deliberately *under* [`MODEL_MAX_TOKENS`] to
/// absorb the re-tokenization drift described there (measured +1…+3; 32 tokens of headroom here).
///
/// Note the distinction the name now carries and the old one didn't (M5-R2): this bounds the
/// **window**, not the **sequence**. The sequence is `window + specials + edge drift`, and only
/// [`MODEL_MAX_TOKENS`] bounds *that*.
pub const MAX_WINDOW_TOKENS: usize = 480;

/// Overlap (in tokens) between consecutive chunks, so an entity that would
/// otherwise land right at a chunk boundary is still whole in at least one
/// chunk. Generous relative to a Person/Org/Location span (rarely more than a
/// handful of tokens).
///
/// `pub` for the same reason [`chunk_char_ranges`] is (M5-R9): the live guard in
/// `tests/ner_perf.rs` drives the real chunker, and a hand-copied `32` there would be a second
/// home for this fact — free to drift from the one that matters.
pub const CHUNK_OVERLAP_TOKENS: usize = 32;

/// The re-tokenization headroom a window must leave below [`MODEL_MAX_TOKENS`]: the two special
/// tokens a re-tokenized chunk re-adds, plus the cut-edge drift (measured **+1…+3** for XLM-R —
/// but nothing *structurally* bounds it, so the margin itself is the guard). A tokenizer swap
/// re-opens this number.
///
/// **This — not the mere ordering `window < ceiling` — is the invariant the chunker relies on
/// (M5-R10).** `479 < 512` and `511 < 512` satisfy `<` identically, but a 511-token window
/// re-tokenizes to ~514 and the PERF-01 `Expand` error is back. The drift has "nowhere to go" the
/// moment the headroom drops below it, not the moment the window reaches the ceiling — so the
/// headroom is what must be pinned. It is deliberately larger than the measured +1…+3: this is the
/// *only* guard in this area that a modelless CI can run, so it holds the constraint the code
/// actually depends on, with room to spare.
pub const MIN_DRIFT_HEADROOM_TOKENS: usize = 16;

// The relationships between the constants above are **compile-time** invariants (M5-R2, M5-R10),
// not runtime tests — get one wrong and the crate must not build at all.
//
// 1. The planning window must leave at least MIN_DRIFT_HEADROOM_TOKENS below the model ceiling, or
//    re-tokenization drift overflows it and the PERF-01 `Expand` error comes straight back. The
//    subtraction is const-evaluated, so a window *over* the ceiling underflows and is a compile
//    error too — this subsumes the weaker `MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS` rather than
//    sitting beside it.
// 2. A window must advance by at least one token, or chunking would not terminate.
const _: () = assert!(
    MODEL_MAX_TOKENS - MAX_WINDOW_TOKENS >= MIN_DRIFT_HEADROOM_TOKENS,
    "the planning window must leave room for re-tokenization drift (specials + cut-edge drift)"
);
const _: () = assert!(
    CHUNK_OVERLAP_TOKENS < MAX_WINDOW_TOKENS,
    "a chunk window must advance, or chunking would not terminate"
);

/// Derive the default intra-op thread count for **one** session, given the pool size — the
/// `NER_INTRA_THREADS` default (M7), divided out of the **thread base** (M11 Track B).
///
/// **The two knobs multiply, and that is the trap.** `NER_POOL_SIZE × intra_threads` is the
/// process's NER thread count under saturated load; the invariant is that the **product** fits the
/// machine, not that either factor saturates it. A fixed `intra = 6` with `pool = 2` puts 12 ONNX
/// threads on a 6-core box — oversubscription, plausibly *slower* than one thread. So the default
/// is derived from the box rather than picked: a constant is wrong on a 2-core VM and on a 64-core
/// server alike.
///
/// **The divisor is the base, not the logical core count (M11 Track B).** It was
/// `available_parallelism()` through M10; it is now [`derive_thread_base`] — physical cores, capped
/// by the parallelism the platform grants. On an SMT box that halves the derived `intra` at every
/// pool size, which is the whole point: read that function for the reasoning, it is not repeated
/// here.
///
/// **What this does NOT buy, and the measurement that says so (M7/S1a).** The product is the
/// *saturated-load* count. A **single** request is sequential at three nested levels — the field
/// walk holds `&mut Vault`, [`infer_chunked`](OnnxNerDetector::infer_chunked) loops its windows,
/// and only then does the session run — so one request can reach `intra`, never `pool × intra`; the
/// pool only wakes for a *second* concurrent request. At the shipped default `pool = 1` this returns
/// the whole base (6 on a 6-core / 12-thread box), so a lone Claude Code request gets every physical
/// core — the personal-proxy shape, which is why it is the default. An operator *centralizing* for
/// concurrent clients sets `NER_POOL_SIZE=N`: at `pool = 2` this returns 3 and a lone request then
/// leaves 3 cores idle — the right trade when a *second* request is there to use them, the wrong one
/// when it never comes.
///
/// **The bound this derivation actually provides, and its domain (M7-R4).** While `pool ≤ base` it
/// holds unconditionally: `intra = floor(base/pool)`, so `pool × intra ≤ base`. **Beyond that it
/// cannot.** `intra` floors at 1 and nothing clamps `NER_POOL_SIZE`, so `pool > base` oversubscribes
/// by `pool` alone — `NER_POOL_SIZE=8` on a 2-core box is 8 threads on 2 cores, and no choice of
/// `intra` fixes it. That is an operator error the proxy does not defend against (it hits the
/// ~270 MB-per-session RAM wall long before the thread wall). The invariant is stated with its
/// domain rather than absolutely, because an invariant that is false in a reachable regime is worse
/// than no invariant.
///
/// **Prefer [`resolve_pool_and_intra`], which is the entry point the server and the latency harness
/// both use.** This wrapper is kept for callers that have already resolved a pool and only need the
/// derivation — it takes `pool` as an argument, so it cannot reintroduce a second *default* (the
/// M7-R1 failure); the default itself has exactly one home, [`DEFAULT_POOL_SIZE`].
pub fn default_intra_threads(pool_size: usize) -> usize {
    derive_intra_threads(pool_size, CoreCounts::detect().thread_base())
}

/// **LOGICAL threads**, or 1 if the platform won't say — SMT siblings included, and honouring
/// whatever the platform actually grants this process (cgroup quota, CPU affinity mask, Windows
/// job object).
///
/// **This is deliberately not the thread base any more (M11 Track B).** It was through M7–M10,
/// which is how `pool = 1` came to put both siblings of every core on the same int8 GEMM. The base
/// is now [`CoreCounts::thread_base`]; this stays the *logical* count because it is one of the two
/// inputs that base is a `min` of — and because `tests/ner_perf.rs`'s NER-THREAD-01 still sweeps
/// intra up to it on purpose (more partitions is strictly more coverage for a detection-inertness
/// guard, whatever the shipped default happens to be).
pub fn available_cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// **PHYSICAL cores**, or `None` if the platform won't say.
///
/// The only count `std` cannot answer, which is the whole reason `num_cpus` is a dependency —
/// behind the `onnx` feature only, beside this function, so the default build's footprint is
/// untouched (`tests/dependency_footprint.rs` asserts the *default* tree).
///
/// **Honest about its own fallback:** `num_cpus::get_physical()` does *not* report "unknown" on a
/// platform it cannot probe — it silently returns the logical count. So the `None` arm is not
/// reachable through that path today (only a nonsensical `0` maps to it). It exists because the
/// *contract* needs it: [`derive_thread_base`] must be able to answer "the platform won't say",
/// and it answers with `logical` — which is byte-for-byte what num_cpus' own fallback would have
/// produced anyway. A platform that cannot answer loses nothing and behaves exactly as before M11.
pub fn physical_cores() -> Option<usize> {
    // `filter(> 0)` rather than a bare `Some`: a zero base would divide down to `intra = 1` on a
    // machine with cores to spare — the same class of silent nonsense `resolve_pool_and_intra`
    // refuses from a `0` env var (M7-R5).
    Some(num_cpus::get_physical()).filter(|n| *n > 0)
}

/// Which of the two counts decided the thread base — logged beside it, because a derived value the
/// logged inputs cannot explain is what [M7-R5](../../docs/reviews/M7.md#m7-r5) rejected, and the
/// base is no longer the core count an operator's task manager shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadBaseSource {
    /// The physical core count, at or below what the platform grants — the normal case, and the
    /// point of the change: one thread per core, no SMT sibling doubling.
    PhysicalCores,
    /// `available_parallelism()` was the *smaller* of the two, so it capped the silicon — a cgroup
    /// quota, a CPU affinity mask, a Windows job object. This is the case the `min` exists for: a
    /// physical-core count reports the hardware and knows nothing about any of them, so taking it
    /// bare would derive `intra = 32` for a proxy pinned to 2 CPUs on a 32-core host.
    ParallelismCap,
    /// The platform would not report a physical count; the base is the logical one — exactly the
    /// pre-M11 behaviour.
    PhysicalUnknown,
}

impl ThreadBaseSource {
    /// Short, stable token for the startup log.
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadBaseSource::PhysicalCores => "physical",
            ThreadBaseSource::ParallelismCap => "parallelism-cap",
            ThreadBaseSource::PhysicalUnknown => "physical-unknown",
        }
    }
}

/// The two core counts this machine reports, so the base and its provenance come from **one** pair
/// of readings rather than from a re-detection per call site.
///
/// Deliberately holds just the two *inputs*: [`CoreCounts::thread_base`] and
/// [`CoreCounts::thread_base_source`] delegate to the pure functions, so the number that is used
/// and the number that is logged cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreCounts {
    /// `available_parallelism()` — logical threads, and what the platform grants.
    pub logical: usize,
    /// Physical cores, or `None` if the platform won't say.
    pub physical: Option<usize>,
}

impl CoreCounts {
    /// Read both counts off this machine — the impure companion of [`derive_thread_base`].
    pub fn detect() -> Self {
        Self {
            logical: available_cores(),
            physical: physical_cores(),
        }
    }

    /// The base [`resolve_pool_and_intra`] divides.
    pub fn thread_base(self) -> usize {
        derive_thread_base(self.logical, self.physical)
    }

    /// Where that base came from, for the log.
    pub fn thread_base_source(self) -> ThreadBaseSource {
        thread_base_source(self.logical, self.physical)
    }

    /// The physical count as a loggable number — `0` meaning "the platform would not say", the
    /// same sentinel the env knobs use for "unset". `tracing` fields are typed, so an `Option`
    /// cannot be recorded as one.
    pub fn physical_for_log(self) -> usize {
        self.physical.unwrap_or(0)
    }
}

/// **The intra-op thread base: physical cores, capped by the parallelism the platform grants**
/// (M11 Track B, decided 2026-09-02).
///
/// ```text
/// base  = min(physical_cores, available_parallelism())
/// intra = max(1, base / NER_POOL_SIZE)
/// ```
///
/// **Why physical and not logical.** Through M10 the base was the logical count, so on the
/// reference box (6 cores / 12 threads) the shipped `NER_POOL_SIZE=1` derived `intra = 12`: both
/// SMT siblings of every core running the same int8 GEMM, contending for one core's L1d/L2 and one
/// set of vector units. Physical cores is the conventional intra-op base for GEMM-bound inference,
/// and it is **the same rule at every pool size** — the sibling contention that motivates the cap
/// does not weaken when a second session appears, it applies to both of them. Dividing the logical
/// count from `pool = 2` up would put `2 × 6 = 12` ONNX threads back on 6 physical cores,
/// reintroducing at `pool = 2` exactly what this removes at `pool = 1`, and making the process's
/// total NER thread count *double* between `pool = 1` and `pool = 2` under a formula whose whole
/// purpose is that the product fits the box. The cost is named and accepted: at `pool = 2` on a
/// 6/12 box the default falls from 6 threads per session to 3, leaving the siblings to the
/// runtime's own work (tokio, TLS, JSON) — which is latency-bound and *does* profit from SMT,
/// unlike the GEMM.
///
/// **`min`, never `physical` alone — this is the trap.** `available_parallelism()` honours cgroup
/// quota, CPU affinity masks and Windows job objects; a physical-core count does not, it reports
/// the silicon. Take it bare and a proxy in a 2-CPU container on a 32-core host derives
/// `intra = 32` — the oversubscription this derivation exists to prevent, arriving through the fix
/// for it. The `min` also settles CPUs whose thread count is no longer 2× the core count: on a
/// hybrid P+E part (14 cores / 20 threads) it returns 14, every core once; with SMT off in firmware
/// the two counts are equal and it is a no-op.
///
/// **`None` falls back to `logical`** — today's behaviour exactly, so a platform that cannot report
/// physical cores loses nothing.
///
/// **Pure in both arguments on purpose** — the standing rule [`derive_intra_threads`] already
/// follows: the runner's own box must not be able to decide whether this is correct. THREAD-01
/// pins the whole `(logical, physical)` grid while owning none of that hardware.
///
/// **Settled by decision and mechanism, not by this box's timings.**
/// [M7-R2](../../docs/reviews/M7.md#m7-r2) left the SMT question *unresolved* after four runs whose
/// sign flipped under a ~40% same-configuration spread; M11 does not claim to have resolved it. It
/// adopts the conventional base and stops paying for a knob no measurement on this hardware can
/// read. `NER_INTRA_THREADS` remains the explicit override, and still wins.
pub fn derive_thread_base(logical: usize, physical: Option<usize>) -> usize {
    // `max(1)` for the same reason `derive_intra_threads` clamps: a base of 0 is not a shape any
    // caller can use, and it must never reach ONNX Runtime as "pick for me".
    logical.min(physical.unwrap_or(logical)).max(1)
}

/// Where [`derive_thread_base`] took its answer from — pure, so the log can name the provenance
/// without recomputing the arithmetic itself.
pub fn thread_base_source(logical: usize, physical: Option<usize>) -> ThreadBaseSource {
    match physical {
        None => ThreadBaseSource::PhysicalUnknown,
        // Equal counts (SMT off in firmware) report `PhysicalCores`: the base *is* the physical
        // count, and nothing capped it.
        Some(p) if p <= logical => ThreadBaseSource::PhysicalCores,
        Some(_) => ThreadBaseSource::ParallelismCap,
    }
}

/// **The shipped session-pool default — one session (was 2 through M7; flipped 2026-07-17).** The
/// dominant deployment is a *personal* proxy in front of a single client (Claude Code, concurrency
/// ≈ 1), and a single request only ever occupies one session (S1a), so a second pooled session buys
/// a lone request nothing while adding a second ~270 MB copy of the model. So the lean default is one
/// session: the whole base for the one in-flight request, and less RAM — measured **563 MB at
/// `pool=1` vs 834 MB at `pool=2`** (a ~290 MB shared base + ~270 MB per session). This is the
/// `low-RAM` bar in `CLAUDE.md` applied to the case almost everyone runs — and it is **not** a
/// latency claim (`intra = base` vs `base/2` is inside this box's noise, M7-R2/S1). The trade it
/// *does* make is throughput: under **concurrent** load `pool=1` measured **~−23%** turns/s versus
/// the pooled shape (two independent measurements + a mechanism — intra-op scaling is sublinear, so
/// N sessions × base/N threads aggregate better than one × base; DEVLOG 2026-07-16). The personal
/// case has no concurrency to lose, so it pays nothing for the RAM it saves; an operator
/// *centralizing* the proxy for concurrent clients sets `NER_POOL_SIZE=N` to reclaim that
/// throughput. The flip and its measurement live in `ARCHITECTURE.md` → *NER threading* and DEVLOG
/// 2026-07-17.
///
/// **M11 Track B re-measured that −23% on the new base and it inverted** (DEVLOG 2026-09-02): with
/// the base at physical cores, `1×6` reached **0.485** turns/s against `1×12`'s 0.419 and `2×3`
/// reached **0.664** against `2×6`'s 0.558. The pooled shape still leads at concurrency 4, so the
/// direction of the trade stands and this default is unchanged — but both sides of it got faster,
/// and the gap narrowed.
///
/// A `pub const` and not a literal because the M7 latency harness must measure *what the server
/// runs*: when these were two independent `unwrap_or`s they silently disagreed, so M7's executable
/// bar guarded a configuration nobody ships (M7-R1). One home, and the drift is structurally
/// impossible rather than merely noticed.
pub const DEFAULT_POOL_SIZE: usize = 1;

/// Resolve `(pool, intra)` from the two env vars' raw values — **the single home of that policy**,
/// so the server and the latency harness cannot resolve them differently (M7-R1/M7-R5).
///
/// Pure in `base` so it is testable without the host's core count deciding the answer.
///
/// **`base` is [`derive_thread_base`]'s output, not a raw core count (M11 Track B).** It was
/// `available_parallelism()` through M10. This function did not otherwise change: it keeps its
/// single home and its `0`-is-unset symmetry, and **only the base it divides moved** — which is
/// also why `GLINER_POOL_SIZE`/`GLINER_INTRA_THREADS` inherit the new base for free, with no second
/// path to keep in step (M7-R1 is the record of what a second path costs).
///
/// **Both knobs treat `0` as unset, and that symmetry is the point (M7-R5).** M7 shipped the guard
/// on `NER_INTRA_THREADS` only, leaving the older `NER_POOL_SIZE` to parse `0` into a pool of zero.
/// That was *safe* — `derive_intra_threads` does `.max(1)` and `load` clamps the pool — but safe by
/// two independent accidents, and the startup log then printed `pool_size=0, intra_threads=12`,
/// which no arithmetic reconciles. A derived value an operator cannot reproduce from the logged
/// inputs defeats the reason it is logged.
pub fn resolve_pool_and_intra(
    pool_var: Option<&str>,
    intra_var: Option<&str>,
    base: usize,
) -> (usize, usize) {
    let pool = pool_var
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_POOL_SIZE);
    // ONNX Runtime reads intra_threads = 0 as "pick for me" — i.e. every session grabbing every
    // core, which is precisely the oversubscription the derivation exists to prevent. Never let a
    // `0` through as a value; treat it as absent.
    let intra = intra_var
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| derive_intra_threads(pool, base));
    (pool, intra)
}

/// The pure core of [`default_intra_threads`], split out so it is testable without depending on
/// the host's core count (the CI runner's box must not decide whether this is correct).
///
/// `base` is [`derive_thread_base`]'s output — physical cores capped by the granted parallelism
/// (M11 Track B) — where it used to be the logical thread count.
fn derive_intra_threads(pool_size: usize, base: usize) -> usize {
    // `max(1)` on both sides: a zero pool is already clamped to 1 by `load`, and ONNX Runtime
    // treats intra_threads = 0 as "pick for me" — which would silently reintroduce the
    // oversubscription this function exists to prevent.
    (base / pool_size.max(1)).max(1)
}

/// The hardware backend an ONNX session runs on (M9).
///
/// `Cpu` is the universal, always-available, always-trusted default — the reference
/// implementation, and the only provider that has passed the cross-thread determinism
/// guard. Every other variant is an **opt-in accelerator** chosen at runtime by
/// `NER_EXECUTION_PROVIDER`.
///
/// Whether one *exists* in a given binary is a build-time fact decided by the linked ONNX
/// Runtime distribution, not by this enum: each platform's natural accelerator is wired
/// per-target in `Cargo.toml` (DirectML on Windows, CoreML on macOS, CUDA on x86_64 Linux), so
/// `--features onnx` already carries it. Selecting a variant the distribution does not contain
/// is not an error — it falls back to CPU. Ask [`super::bench::available_providers`] what is
/// actually present rather than inferring it from cargo features.
///
/// **Only `DirectMl` has been benchmarked** (on this project's AMD DX12 iGPU, where it lost to
/// the CPU); the rest are reachable but UNVERIFIED, and *none* of them — DirectML included —
/// has been run against the determinism guard. See `ARCHITECTURE.md` → *Execution providers*
/// for the measured/trusted split.
///
/// Whatever is selected, a session that cannot initialize the accelerator
/// **falls back to CPU** (see [`build_session_reporting`]): a privacy proxy must never fail
/// to *start* over a GPU that isn't there, and masking is identical on CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionProvider {
    Cpu,
    DirectMl,
    Cuda,
    TensorRt,
    CoreMl,
    Rocm,
    OpenVino,
    WebGpu,
}

impl ExecutionProvider {
    /// Parse the `NER_EXECUTION_PROVIDER` value (case-insensitive; `""`/absent →
    /// [`Cpu`](Self::Cpu)). An **unknown** name is an `Err`, never a silent CPU
    /// fallback: a typo'd accelerator (`directl`, or a hopeful `vulkan` — not an
    /// ONNX Runtime backend) must surface at startup, the same rule the GLiNER
    /// companion-var check follows (M8-R5). "Failed to initialize" (→ CPU fallback)
    /// and "you named something that doesn't exist" (→ config error) are different
    /// failures and must not be conflated.
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "" | "cpu" => Self::Cpu,
            "directml" | "dml" => Self::DirectMl,
            "cuda" => Self::Cuda,
            "tensorrt" | "trt" => Self::TensorRt,
            "coreml" => Self::CoreMl,
            "rocm" => Self::Rocm,
            "openvino" | "vino" => Self::OpenVino,
            "webgpu" => Self::WebGpu,
            other => {
                return Err(format!(
                    "unknown NER_EXECUTION_PROVIDER {other:?} (expected one of: cpu, \
                     directml, cuda, tensorrt, coreml, rocm, openvino, webgpu)"
                ));
            }
        })
    }

    /// Stable lower-case name, for logs and for round-tripping through [`parse`](Self::parse).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::DirectMl => "directml",
            Self::Cuda => "cuda",
            Self::TensorRt => "tensorrt",
            Self::CoreMl => "coreml",
            Self::Rocm => "rocm",
            Self::OpenVino => "openvino",
            Self::WebGpu => "webgpu",
        }
    }

    /// The `ort` dispatch for an accelerated provider, or `None` for [`Cpu`](Self::Cpu).
    ///
    /// Every EP *type* is always compiled (only its `register()` is feature-gated
    /// inside `ort`), so this needs no `#[cfg]` per arm — a provider whose `ep-*`
    /// feature is off simply fails registration with `MissingFeature`, which
    /// [`build_session_reporting`] turns into a CPU fallback.
    ///
    /// `.error_on_failure()` is deliberate: it makes a failed registration return
    /// `Err` from the session build so the fallback can be **explicit and logged**.
    /// ORT's default (`fail_silently`) would drop to CPU invisibly, hiding which
    /// backend actually ran — unacceptable observability for a tool that logs its
    /// effective config.
    fn dispatch(self) -> Option<ort::ep::ExecutionProviderDispatch> {
        use ort::ep;
        Some(match self {
            Self::Cpu => return None,
            Self::DirectMl => ep::DirectML::default().build().error_on_failure(),
            Self::Cuda => ep::CUDA::default().build().error_on_failure(),
            Self::TensorRt => ep::TensorRT::default().build().error_on_failure(),
            Self::CoreMl => ep::CoreML::default().build().error_on_failure(),
            Self::Rocm => ep::ROCm::default().build().error_on_failure(),
            Self::OpenVino => ep::OpenVINO::default().build().error_on_failure(),
            Self::WebGpu => ep::WebGPU::default().build().error_on_failure(),
        })
    }
}

/// Build one ONNX session on `provider`, **falling back to CPU** if the accelerator can't
/// initialize (feature not compiled, backend binary/driver missing, no device, registration
/// error), and report the **effective** provider — what the session actually runs on, which is
/// [`ExecutionProvider::Cpu`] whenever the requested accelerator fell back.
///
/// The single home of the EP-selection + fallback policy. Both detectors reach it through
/// [`build_session_pool`] and the benchmark ([`super::bench`]) calls it directly, so the
/// fallback behaves identically everywhere (no second, drifting copy).
///
/// **Reporting the effective provider is not optional bookkeeping.** A caller that echoed the
/// *requested* one would log a GPU the process is not using (M9-R1) and would let a benchmark
/// claim a GPU measurement that never touched the GPU — the one way a tool meant to answer
/// "what is fastest here?" could confidently answer wrong.
///
/// **The fallback covers *initialization*, not *numerical correctness*, and not the session's
/// later life.** An accelerator that registers but computes subtly different logits than the
/// CPU is not caught here; neither is one that dies mid-process (M9-R11 — that surfaces as an
/// ordinary inference error and follows the `NER_REQUIRED` posture). The blast radius is bounded
/// to **NER recall** (the best-effort layer): the deterministic structured recognizers run on CPU
/// regex, independent of the EP, so the fail-closed layer is untouched no matter what backend the
/// NER runs on. That bound is what makes an untested EP an acceptable opt-in, and it is *also*
/// why even the benchmarked one (DirectML) still owes the cross-thread-determinism guard before
/// it is trusted (see `ARCHITECTURE.md` → *Execution providers*).
pub(crate) fn build_session_reporting(
    model_path: &str,
    intra_threads: usize,
    provider: ExecutionProvider,
) -> Result<(Session, ExecutionProvider)> {
    let intra_threads = intra_threads.max(1);
    if let Some(dispatch) = provider.dispatch() {
        match build_session_inner(model_path, intra_threads, Some(dispatch)) {
            Ok(session) => {
                tracing::info!(
                    provider = provider.as_str(),
                    "NER session initialized on accelerated execution provider"
                );
                return Ok((session, provider));
            }
            Err(e) => {
                // Never fail to start over an absent accelerator — drop to CPU, loudly.
                tracing::warn!(
                    provider = provider.as_str(),
                    error = %e,
                    "accelerated execution provider failed to initialize; falling back to CPU"
                );
            }
        }
    }
    build_session_inner(model_path, intra_threads, None).map(|s| (s, ExecutionProvider::Cpu))
}

/// Build a pool of `pool_size` sessions that all run on the **same** backend, and report which
/// one that is (M9-R1, M9-R2).
///
/// **Homogeneity is the point, not a nicety.** Each session decides independently whether the
/// accelerator initializes, so a naive loop can produce a pool that is part GPU and part CPU —
/// and this is not exotic on the hardware M9 targets: ORT reports **512 MB** of video memory on
/// this box's iGPU while the fp16 XLM-R export is **555 MB**, so at `NER_POOL_SIZE=2` the second
/// session failing on VRAM is the *expected* outcome. Sessions are then dispatched round-robin,
/// which would make the backend a **per-request variable**: the same text could be scanned by
/// DirectML on one request and the CPU on the next, so a name masked once could be missed next
/// time, inside one process, with no configuration change. That is a strictly stronger condition
/// than `m7_r3_intra_threads_changes_speed_not_detection` was ever run against.
///
/// So the **first** session decides, the rest are built on whatever it actually got, and if a
/// later one cannot match it the **whole pool** is rebuilt on CPU. Mixed is never served.
pub(crate) fn build_session_pool(
    model_path: &str,
    pool_size: usize,
    intra_threads: usize,
    provider: ExecutionProvider,
) -> Result<(Vec<Mutex<Session>>, ExecutionProvider)> {
    let pool_size = pool_size.max(1);
    let (first, effective) = build_session_reporting(model_path, intra_threads, provider)?;

    let mut sessions = Vec::with_capacity(pool_size);
    sessions.push(Mutex::new(first));
    for _ in 1..pool_size {
        // Ask for what the first session GOT, not what the caller wanted: if the accelerator
        // already fell back, there is nothing to retry.
        let (session, got) = build_session_reporting(model_path, intra_threads, effective)?;
        if got != effective {
            tracing::warn!(
                pool_provider = effective.as_str(),
                degraded_to = got.as_str(),
                "a later NER session could not use the pool's execution provider; rebuilding the \
                 whole pool on CPU rather than serving a mixed pool"
            );
            return build_cpu_pool(model_path, pool_size, intra_threads);
        }
        sessions.push(Mutex::new(session));
    }
    Ok((sessions, effective))
}

/// Every session on the CPU — the uniform fallback shape for [`build_session_pool`].
fn build_cpu_pool(
    model_path: &str,
    pool_size: usize,
    intra_threads: usize,
) -> Result<(Vec<Mutex<Session>>, ExecutionProvider)> {
    let mut sessions = Vec::with_capacity(pool_size);
    for _ in 0..pool_size {
        sessions.push(Mutex::new(build_session_inner(
            model_path,
            intra_threads.max(1),
            None,
        )?));
    }
    Ok((sessions, ExecutionProvider::Cpu))
}

/// The actual `ort` builder chain. `dispatch = None` is the plain CPU session;
/// `Some(ep)` registers the accelerator first (with `error_on_failure`, so a bad
/// registration surfaces here for [`build_session_reporting`]'s fallback to catch).
fn build_session_inner(
    model_path: &str,
    intra_threads: usize,
    dispatch: Option<ort::ep::ExecutionProviderDispatch>,
) -> Result<Session> {
    // ort's builder errors aren't Send+Sync, so convert to a string at each step
    // rather than chaining `?` into `anyhow` (the module-wide dance).
    let builder = Session::builder().map_err(|e| anyhow!("session builder: {e}"))?;
    let builder = match dispatch {
        Some(ep) => builder
            .with_execution_providers([ep])
            .map_err(|e| anyhow!("register execution provider: {e}"))?,
        None => builder,
    };
    let builder = builder
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow!("optimization level: {e}"))?;
    let mut builder = builder
        .with_intra_threads(intra_threads)
        .map_err(|e| anyhow!("intra threads: {e}"))?;
    builder
        .commit_from_file(model_path)
        .map_err(|e| anyhow!("load model {model_path}: {e}"))
}

/// NER-based detector backed by an ONNX Runtime session.
///
/// Holds a small **pool** of sessions so inference isn't a single-threaded
/// bottleneck under concurrent load (the sync [`PiiDetector::detect`] is called
/// from many request tasks). Sessions are checked out round-robin.
pub struct OnnxNerDetector {
    sessions: Vec<Mutex<Session>>,
    tokenizer: Tokenizer,
    /// id → label string (e.g. `["O", "B-PER", "I-PER", …]`), from the model's
    /// config. Passed in so this stays model-agnostic.
    id2label: Vec<String>,
    /// Whether the model expects a `token_type_ids` input (BERT-family).
    needs_token_type_ids: bool,
    /// The backend every session in the pool actually runs on — **not** the one that was
    /// requested (M9-R1). The pool is homogeneous by construction ([`build_session_pool`]), so
    /// one value describes it. Exposed via [`provider`](Self::provider) so the caller logs what
    /// is running rather than what was asked for.
    provider: ExecutionProvider,
    next: AtomicUsize,
}

impl OnnxNerDetector {
    /// Load the model + tokenizer from disk and build a CPU session pool.
    ///
    /// `id2label` is the model's label list (index = class id); `pool_size` is
    /// clamped to at least 1; `intra_threads` is the per-session intra-op thread count (see
    /// [`default_intra_threads`] — the caller derives it, this only clamps);
    /// `needs_token_type_ids` threads a zero `token_type_ids` input for BERT-family models;
    /// `provider` selects the execution backend (M9) — [`ExecutionProvider::Cpu`] is the
    /// default, any other value is an opt-in accelerator with CPU fallback (see
    /// [`build_session_reporting`]).
    pub fn load(
        model_path: &str,
        tokenizer_path: &str,
        id2label: Vec<String>,
        pool_size: usize,
        intra_threads: usize,
        needs_token_type_ids: bool,
        provider: ExecutionProvider,
    ) -> Result<Self> {
        let tokenizer =
            Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!("load tokenizer: {e}"))?;

        let pool_size = pool_size.max(1);
        // 0 would mean "ONNX Runtime picks", which on a 12-thread box is 12 *per session* — the
        // oversubscription `default_intra_threads` exists to prevent. `build_session_pool` clamps
        // it, but keep the clamp here too so the value handed to each session is unambiguous.
        let intra_threads = intra_threads.max(1);
        // One backend for the whole pool, and we keep the one it actually got (M9-R1/R2).
        let (sessions, provider) =
            build_session_pool(model_path, pool_size, intra_threads, provider)?;

        Ok(Self {
            sessions,
            tokenizer,
            id2label,
            needs_token_type_ids,
            provider,
            next: AtomicUsize::new(0),
        })
    }

    /// The backend this detector's sessions **actually** run on.
    ///
    /// Differs from the requested provider whenever the accelerator could not initialize. The
    /// caller logs *this*, never the request: a startup line that names a GPU the process is not
    /// using is the failure M7-R5 established the rule against, and M9 reintroduced it (M9-R1).
    pub fn provider(&self) -> ExecutionProvider {
        self.provider
    }

    /// Tokenize `input`; dispatch to one model call when it fits the model's
    /// sequence budget, or to [`infer_chunked`](Self::infer_chunked) when it doesn't.
    fn infer(&self, input: &str) -> Result<Vec<PiiEntity>> {
        // Drop the tokenizer's error detail: it can echo the input text, and this
        // is a "never log raw PII" tool (M2-R8).
        let encoding = self
            .tokenizer
            .encode(input, true)
            .map_err(|_| anyhow!("tokenizer error"))?;

        if encoding.get_ids().len() <= MAX_WINDOW_TOKENS {
            return self.run_and_decode(input, &encoding);
        }
        self.infer_chunked(input, &encoding)
    }

    /// Split `input` into overlapping windows (by byte offset, derived from
    /// `full_encoding`'s per-token offsets) each within [`MAX_WINDOW_TOKENS`] tokens, run each
    /// independently, and merge the results.
    ///
    /// Each window is **re-tokenized from its own text**, not sliced out of
    /// `full_encoding`'s token ids: a middle chunk needs its own `<s>` / `</s>`
    /// framing, which a raw token-id slice would lack. Windows overlap by
    /// [`CHUNK_OVERLAP_TOKENS`], so an entity landing on what would otherwise be
    /// a chunk boundary is still whole in a neighboring window; an exact
    /// duplicate entity from the overlap is deduped by (kind, span, text).
    ///
    /// A window that cuts an entity in half still emits the *truncated* fragment (`Mil` where the
    /// neighbouring window sees `Milan`) — a different span, so `dedup` does **not** remove it.
    /// [`resolve_overlaps`](super::overlap::resolve_overlaps) does: its NER phase tiebreaks on
    /// span length descending, takes the whole entity, and drops the fragment as overlapping. See
    /// `ARCHITECTURE.md` → *NER chunking* — that tiebreak is load-bearing here.
    ///
    /// This is a **recall** mechanism, never a leak-relevant one: structured PII
    /// (the fail-closed layer) is detected independently, over the whole field,
    /// and is never chunked. An entity that falls in the small sliver right at a
    /// window edge without enough overlap to be whole in either window is a
    /// missed name/org/location — the same class of gap `OVL-02`/M2-R7 already
    /// document as accepted for the best-effort NER layer.
    fn infer_chunked(&self, input: &str, full_encoding: &Encoding) -> Result<Vec<PiiEntity>> {
        let ranges = chunk_char_ranges(
            input,
            full_encoding.get_offsets(),
            MAX_WINDOW_TOKENS,
            CHUNK_OVERLAP_TOKENS,
        );

        let mut entities = Vec::new();
        for (char_start, char_end) in ranges {
            // `chunk_char_ranges` widens every range to `char` boundaries, so this is total by
            // construction. We still don't *index* — this is a proxy on attacker-influenced input,
            // and the rest of the masking path (`decode_entities`, `Vault::mask`,
            // `overlap::materialize`) all refuse to panic on a tokenizer-derived range for exactly
            // this reason (M2-R6, M4-R14). Skipping a window costs recall on it; panicking costs
            // the request (M5-R3).
            let Some(chunk) = input.get(char_start..char_end) else {
                debug_assert!(false, "chunk_char_ranges must return sliceable ranges");
                tracing::warn!("NER chunk window fell off a char boundary; skipping the window");
                continue;
            };
            let chunk_encoding = self
                .tokenizer
                .encode(chunk, true)
                .map_err(|_| anyhow!("tokenizer error"))?;
            for mut entity in self.run_and_decode(chunk, &chunk_encoding)? {
                entity.span = (entity.span.start + char_start)..(entity.span.end + char_start);
                entities.push(entity);
            }
        }

        entities.sort_by(|a, b| {
            a.span
                .start
                .cmp(&b.span.start)
                .then(a.span.end.cmp(&b.span.end))
        });
        entities.dedup();
        Ok(entities)
    }

    /// Run the model on an already-tokenized `(input, encoding)` pair and decode
    /// its logits into entity spans. `input` and `encoding` must correspond to
    /// the *same* text — this is the one-shot path shared by the direct call and
    /// each chunk of [`infer_chunked`](Self::infer_chunked).
    ///
    /// **This is where [`MODEL_MAX_TOKENS`] is enforced (M5-R2)**, because it is the single choke
    /// point every path into the session goes through — the direct call *and* every re-tokenized
    /// chunk. A sequence longer than the model's usable length is **rejected**, never forwarded:
    /// past it the ONNX graph fails outright (the PERF-01 `Expand` error), so this turns a cryptic
    /// tensor-shape failure into a named one, *before* the session ever sees it.
    ///
    /// **It returns `Err` — it does NOT clamp and continue (M5-R7).** The first version of this
    /// fix truncated the sequence and returned `Ok(partial)`, reasoning that losing a window's tail
    /// beats losing the whole field. That reasoning is fine; **making the call here is not.**
    /// Whether a degraded NER is acceptable is a *posture* decision, and this codebase already has
    /// exactly one place that owns it: the [`FailOpen`](super::composite::FailOpen) wrapper and the
    /// [`try_detect`](PiiDetector::try_detect) error channel (M2-R1/R2). Under the default
    /// (fail-open) posture `FailOpen` swallows this error and the request proceeds structured-only;
    /// under **`NER_REQUIRED`** the detector is *unwrapped*, so the error propagates and the
    /// request is **blocked (400)** — which is precisely what that operator asked for. Clamping
    /// silently returned `Ok` in **both** postures, quietly forwarding a partially-scanned field to
    /// someone who had explicitly demanded a block. That is not a smaller failure; it is the
    /// failure *relocated* — the exact move the M4 retrospective is about.
    ///
    /// This path is **latent by design**: [`MAX_WINDOW_TOKENS`] leaves 32 tokens of headroom for
    /// re-tokenization drift (measured +1…+3 over 17 adversarial scripts), and
    /// `tests/ner_perf.rs::m5_r2_every_retokenized_window_stays_within_the_models_usable_length`
    /// is the guard that keeps it that way. It is the *fail-safe*, not the mechanism.
    fn run_and_decode(&self, input: &str, encoding: &Encoding) -> Result<Vec<PiiEntity>> {
        let seq = encoding.get_ids().len();
        if seq == 0 {
            return Ok(Vec::new());
        }
        if seq > MODEL_MAX_TOKENS {
            // Counts only, never the text (the never-log-raw-PII rule).
            return Err(anyhow!(
                "NER sequence is {seq} tokens, over the model's usable length of \
                 {MODEL_MAX_TOKENS}; refusing to run it (the posture — fail open to \
                 structured-only, or block under NER_REQUIRED — is decided by the caller)"
            ));
        }
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&i| i as i64).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&i| i as i64)
            .collect();
        let offsets = encoding.get_offsets();

        let input_ids =
            Tensor::from_array(([1, seq], ids)).map_err(|e| anyhow!("input_ids tensor: {e}"))?;
        let attention_mask = Tensor::from_array(([1, seq], mask))
            .map_err(|e| anyhow!("attention_mask tensor: {e}"))?;

        // Round-robin a session; recover a poisoned lock rather than permanently
        // disabling a pool slot (M2-R9).
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        let mut session = self.sessions[idx]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let outputs = if self.needs_token_type_ids {
            let type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&i| i as i64).collect();
            let token_type_ids = Tensor::from_array(([1, seq], type_ids))
                .map_err(|e| anyhow!("token_type_ids tensor: {e}"))?;
            session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                    "token_type_ids" => token_type_ids,
                ])
                .map_err(|e| anyhow!("ONNX run: {e}"))?
        } else {
            session
                .run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                ])
                .map_err(|e| anyhow!("ONNX run: {e}"))?
        };

        // logits: [1, seq, num_labels], row-major. Look the output up by name —
        // never panic on a differently-shaped model (M2-R4).
        let logits_value = outputs
            .get("logits")
            .ok_or_else(|| anyhow!("model has no `logits` output"))?;
        let (_shape, logits) = logits_value
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("extract logits: {e}"))?;

        let num_labels = logits.len() / seq;
        if num_labels == 0 {
            return Ok(Vec::new());
        }
        // A mismatched label list would silently drop entities (M2-R3).
        validate_label_count(self.id2label.len(), num_labels).map_err(|m| anyhow!(m))?;

        let mut tags: Vec<TokenTag> = Vec::with_capacity(seq);
        for (token, &(start, end)) in offsets.iter().enumerate().take(seq) {
            let row = &logits[token * num_labels..(token + 1) * num_labels];
            let best = argmax(row);
            let label = self.id2label.get(best).map(String::as_str).unwrap_or("O");
            tags.push(TokenTag { label, start, end });
        }

        Ok(decode_entities(input, &tags))
    }
}

/// Index of the largest value in `row` (0 if empty / all-NaN).
fn argmax(row: &[f32]) -> usize {
    row.iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

impl PiiDetector for OnnxNerDetector {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        // Infallible view: fail open (the caller decides whether to require it).
        self.try_detect(input, &Budget::per_call())
            .unwrap_or_default()
    }

    fn try_detect(&self, input: &str, _budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        // `infer` never embeds input text in its errors (see M2-R8).
        self.infer(input)
            .map_err(|err| DetectError::unavailable("onnx-ner", err.to_string()))
    }

    /// The NER runs **once**, on the fixpoint's pass 0 (S4). Masking a name to `[PERSON_1]` never
    /// reveals a *new* name, so re-running here buys no recall — measured: **0** losses across the
    /// labelled corpus (DEVLOG 2026-07-18). It would only re-tag the sub-word fragments it emits
    /// (`"lack"` of `"Slack"`), the mechanism that pushed masking past `MAX_MASK_PASSES` and 400'd
    /// real Claude Code system prompts (CC-05/CC-08). So it is idempotent after pass 0 — and this
    /// is also the latency win M4-R21 priced (the field's second full NER scan). See
    /// [`redetect`](PiiDetector::redetect) for the invariant this rests on.
    fn redetect(&self, _input: &str, _budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        Ok(Vec::new())
    }
}

/// Compute the byte ranges of the overlapping windows that cover a tokenized `input`, from its
/// **full** per-token `offsets` (as produced by tokenizing the whole text once).
///
/// Pure and model-independent — no tokenizer or session needed — so, unlike the
/// rest of this module, it is unit-tested without a real ONNX model.
///
/// **Every returned range is guaranteed sliceable** — in-bounds and on `char` boundaries — because
/// each is widened through [`widen_to_char_boundaries`] before being emitted (M5-R3). The offsets
/// come from the *tokenizer*, so treating them as `char`-aligned is an assumption about a
/// third-party library's output on attacker-influenced input, and this module already refuses to
/// make that assumption anywhere else (`decode_entities` guards the identical class of offset —
/// that *was* M2-R6). Widening only ever *adds* bytes, so it cannot shrink a window's coverage.
///
/// **The one thing this exists to get right:** the token at `offsets.len() - 1`
/// is always the closing `</s>` added by `encode(_, true)`, whose offset is the
/// sentinel `(0, 0)` — not the real text end. A window reaching the sequence
/// end must use `input.len()` for its char end, not that sentinel (measured,
/// `tests/ner_perf.rs`: using the sentinel silently dropped the final window,
/// losing a third of the entities on a large field — an outright bug, not a
/// recall nuance, caught only by testing at a size that exercised the last
/// window's boundary).
///
/// `pub` so the live guard in `tests/ner_perf.rs` (M5-R2) can drive the **real** chunker with a
/// real tokenizer and assert each re-tokenized window lands under [`MODEL_MAX_TOKENS`]. A copy of
/// this logic in the test would drift from the code it is supposed to guard.
pub fn chunk_char_ranges(
    input: &str,
    offsets: &[(usize, usize)],
    window: usize,
    overlap: usize,
) -> Vec<(usize, usize)> {
    let seq = offsets.len();
    let stride = window.saturating_sub(overlap).max(1);

    let mut ranges = Vec::new();
    let mut token_start = 0usize;
    loop {
        let token_end = (token_start + window).min(seq);
        let char_start = offsets[token_start].0;
        let char_end = if token_end == seq {
            input.len()
        } else {
            offsets[token_end - 1].1.max(char_start)
        };
        let widened = widen_to_char_boundaries(input, char_start..char_end);
        ranges.push((widened.start, widened.end));

        if token_end == seq {
            break;
        }
        token_start += stride;
    }
    ranges
}

#[cfg(test)]
mod thread_tests {
    use super::{
        derive_intra_threads, derive_thread_base, resolve_pool_and_intra, thread_base_source,
        ThreadBaseSource, DEFAULT_POOL_SIZE,
    };

    /// **The `(logical, physical)` grid — M11 Track B's half of THREAD-01.**
    ///
    /// Pure in both arguments precisely so this table can be written without owning a hybrid P+E
    /// part, a cgroup-limited container or an SMT-disabled box. The runner's own machine decides
    /// nothing here — the same standing rule that made `derive_intra_threads` take `base`.
    #[test]
    fn the_thread_base_is_physical_cores_capped_by_the_granted_parallelism() {
        // The reference box: 6 cores / 12 threads. This is the change — both SMT siblings of every
        // core no longer land on the same int8 GEMM.
        assert_eq!(derive_thread_base(12, Some(6)), 6);
        assert_eq!(
            thread_base_source(12, Some(6)),
            ThreadBaseSource::PhysicalCores
        );

        // SMT off in firmware: the two counts are equal and the `min` is a no-op. Reported as
        // `PhysicalCores` — the base *is* the physical count, and nothing capped it.
        assert_eq!(derive_thread_base(6, Some(6)), 6);
        assert_eq!(
            thread_base_source(6, Some(6)),
            ThreadBaseSource::PhysicalCores
        );

        // Hybrid P+E (14 cores / 20 threads) — the case that raised the question. Thread count is
        // no longer 2x the core count, and the rule still means "every core once".
        assert_eq!(derive_thread_base(20, Some(14)), 14);
        assert_eq!(
            thread_base_source(20, Some(14)),
            ThreadBaseSource::PhysicalCores
        );

        // **The `min` case, and the trap the whole formula exists to avoid.** A proxy granted 2
        // CPUs on a 32-core host: `available_parallelism()` honours the cgroup quota / affinity
        // mask / job object, a physical-core count does not — it reports the silicon. Taking
        // `physical` bare here would derive `intra = 32`, i.e. the oversubscription this
        // derivation exists to prevent, arriving through the fix for it.
        assert_eq!(derive_thread_base(2, Some(32)), 2);
        assert_eq!(
            thread_base_source(2, Some(32)),
            ThreadBaseSource::ParallelismCap
        );

        // Detection unavailable -> the logical count, which is today's behaviour *exactly*. A
        // platform that cannot answer loses nothing.
        assert_eq!(derive_thread_base(12, None), 12);
        assert_eq!(
            thread_base_source(12, None),
            ThreadBaseSource::PhysicalUnknown
        );
    }

    #[test]
    fn the_thread_base_never_returns_zero() {
        // A base of 0 is not a shape any caller can use, and it must never reach ONNX Runtime as
        // "pick for me" — the same clamp, for the same reason, as `derive_intra_threads`.
        // `physical_cores()` already maps a nonsensical `0` to `None`; this is the second guard.
        assert_eq!(derive_thread_base(0, None), 1);
        assert_eq!(derive_thread_base(0, Some(8)), 1);
        assert_eq!(derive_thread_base(8, Some(0)), 1);
    }

    #[test]
    fn the_base_and_the_source_it_is_logged_with_cannot_disagree() {
        // Two pure functions feed one log line (M7-R5): the base an operator reads and the reason
        // beside it must describe the same arithmetic, or the line is worse than no line. Pinning
        // the relation is cheaper than merging them into one return type the callers must destructure.
        for logical in [1usize, 2, 6, 12, 20, 64] {
            for physical in [None, Some(1usize), Some(2), Some(6), Some(14), Some(32)] {
                let base = derive_thread_base(logical, physical);
                match thread_base_source(logical, physical) {
                    // The base came from the silicon: it must equal the physical count.
                    ThreadBaseSource::PhysicalCores => assert_eq!(
                        base,
                        physical.expect("PhysicalCores implies a physical count"),
                        "logical={logical} physical={physical:?}"
                    ),
                    // The platform capped the silicon, or would not report it: either way the base
                    // is the logical count.
                    ThreadBaseSource::ParallelismCap | ThreadBaseSource::PhysicalUnknown => {
                        assert_eq!(base, logical, "logical={logical} physical={physical:?}")
                    }
                }
            }
        }
    }

    #[test]
    fn both_knobs_treat_zero_and_garbage_as_unset() {
        // M7-R5. `NER_INTRA_THREADS=0` shipped with a guard; `NER_POOL_SIZE=0` did not, and was
        // safe only because `derive_intra_threads` and `load` each independently clamp — while the
        // startup log printed `pool_size=0, intra_threads=12`, which no arithmetic reconciles. A
        // derived value the operator cannot reproduce from the logged inputs defeats the reason it
        // is logged, so both knobs must resolve `0` identically to unset.
        // The third argument is the *base* since M11 Track B, not the logical core count — pure in
        // it either way, which is why this table did not have to move.
        let unset = resolve_pool_and_intra(None, None, 12);
        // Default pool is 1, so an unset box derives intra = the whole base (M7.1 flip).
        assert_eq!(unset, (DEFAULT_POOL_SIZE, 12));
        assert_eq!(resolve_pool_and_intra(Some("0"), None, 12), unset);
        assert_eq!(resolve_pool_and_intra(None, Some("0"), 12), unset);
        assert_eq!(resolve_pool_and_intra(Some("0"), Some("0"), 12), unset);
        // Unparseable is unset too — a typo must not silently become a different shape.
        assert_eq!(resolve_pool_and_intra(Some("two"), Some(""), 12), unset);
    }

    #[test]
    fn an_explicit_value_wins_over_the_derivation() {
        // The override is the whole point of the knob: the deployment shape is a question the
        // proxy cannot answer for itself.
        assert_eq!(resolve_pool_and_intra(Some("1"), None, 12), (1, 12));
        assert_eq!(resolve_pool_and_intra(Some("1"), Some("4"), 12), (1, 4));
        // Deliberate oversubscription stays possible — an operator who says 12 gets 12. We refuse
        // to *default* into it, not to permit it.
        assert_eq!(resolve_pool_and_intra(Some("2"), Some("12"), 12), (2, 12));
    }

    #[test]
    fn the_default_gives_one_session_the_whole_base() {
        // M7-R13, after the 2026-07-17 default flip (pool 2 -> 1). The default is now `pool=1`, so
        // the derivation is `intra = base`: one session, every core the base counts — which since
        // M11 Track B is every *physical* core, not every SMT sibling. The pre-M7 shape is
        // `(2, 1)` (intra=1), and a single request only ever reaches `intra` (the pool is inert at
        // concurrency 1), so the default matches the pre-M7 *thread count* — the case where M7
        // delivers nothing — **only on a single-core box**. From two cores up it already adds
        // threads a lone request can use.
        assert_eq!(
            resolve_pool_and_intra(None, None, 1),
            (DEFAULT_POOL_SIZE, 1)
        ); // the only no-op
        assert_eq!(
            resolve_pool_and_intra(None, None, 2),
            (DEFAULT_POOL_SIZE, 2)
        );
        assert_eq!(
            resolve_pool_and_intra(None, None, 3),
            (DEFAULT_POOL_SIZE, 3)
        );
        assert_eq!(
            resolve_pool_and_intra(None, None, 4),
            (DEFAULT_POOL_SIZE, 4)
        );
        // The corollary worth pinning: the thread count — and so the speedup — **scales with the
        // box**, so a claim like "~2x faster" is a claim about a 12-thread machine, not a universal
        // one. `tests/m7_latency.rs` still skips its ratio guard below 4 cores, but for a *different*
        // reason now (M7.1): not "ratio 1.0 by construction" (true only at 1 core here), but that
        // the few-thread shapes below that are too thread-poor to clear the 1.5x floor reliably.
        assert_eq!(
            resolve_pool_and_intra(None, None, 12),
            (DEFAULT_POOL_SIZE, 12)
        );
    }

    #[test]
    fn the_harness_and_the_server_cannot_disagree_about_the_default() {
        // M7-R1. The latency harness resolved its own pool default (1) while the server used 2, so
        // M7's executable bar measured a configuration nobody ships. Both now route through this
        // function and this constant, so the drift is impossible rather than merely fixed.
        assert_eq!(resolve_pool_and_intra(None, None, 12).0, DEFAULT_POOL_SIZE);
    }

    #[test]
    fn the_two_knobs_multiply_to_at_most_the_base_while_the_pool_fits_it() {
        // The invariant M7 rests on, stated with the domain it actually holds in (M7-R4) and
        // restated on M11's base. The first version asserted `pool * intra <= cores.max(pool)`
        // across both regimes — which passes for `pool > cores` by *widening the bound to the pool
        // itself*: it reads `pool <= pool` and green-lights 8 threads on a 2-core box, under a name
        // claiming the opposite. The regimes stay split so the exception is documented, never
        // hidden in a `max` — that is the shape M7-R4 rejected and it must not come back through
        // the rename.
        //
        // **The bases are DERIVED, not a list of plausible integers.** The composition is the new
        // risk surface: `derive_thread_base` feeding `derive_intra_threads` is what actually runs,
        // and a base of 0 or one above the granted parallelism would break the bound precisely
        // where nobody was looking. So the grid is `(logical, physical)` pairs — SMT box, SMT off,
        // hybrid P+E, the container cap, detection unavailable, single core.
        let machines = [
            (12usize, Some(6usize)), // the reference box: 6 cores / 12 threads
            (6, Some(6)),            // SMT off in firmware
            (20, Some(14)),          // hybrid P+E
            (2, Some(32)),           // 2 CPUs granted on a 32-core host — the `min` case
            (12, None),              // the platform won't say
            (1, Some(1)),            // single core
            (64, Some(32)),          // a big server
        ];
        for (logical, physical) in machines {
            let base = derive_thread_base(logical, physical);
            assert!(
                base >= 1,
                "logical={logical} physical={physical:?}: the base must never be 0"
            );
            assert!(
                base <= logical,
                "logical={logical} physical={physical:?}: base={base} exceeds the parallelism the \
                 platform grants — the `min` is what keeps a container from deriving the host's \
                 core count"
            );

            for pool in [1usize, 2, 3, 4, 8] {
                let intra = derive_intra_threads(pool, base);
                assert!(intra >= 1, "base={base} pool={pool}: intra must never be 0");

                if pool <= base {
                    // The real invariant, in the regime where the derivation can honour it.
                    assert!(
                        pool * intra <= base,
                        "base={base} pool={pool}: derived intra={intra} → {} threads, over the \
                         box — the oversubscription this derivation exists to prevent",
                        pool * intra
                    );
                } else {
                    // Beyond the box the derivation is out of moves: `intra` floors at 1 and
                    // nothing clamps NER_POOL_SIZE, so `pool` alone oversubscribes and no choice
                    // of `intra` fixes it. The best available is to add nothing — pin that.
                    assert_eq!(
                        intra, 1,
                        "base={base} pool={pool}: an over-large pool already oversubscribes; \
                         intra must not multiply it further"
                    );
                }
            }
        }
    }

    #[test]
    fn derives_the_documented_shapes() {
        // The reference box END TO END, which is the number the docs quote: 6 cores / 12 threads
        // -> base 6, not 12. Before M11 Track B the first of these was `(1, 12) -> 12`.
        let base = derive_thread_base(12, Some(6));
        assert_eq!(base, 6);
        // The shipped default (M7.1): the personal proxy (Claude Code, concurrency ~1) — one
        // session, every PHYSICAL core. This is the shape that matters for M7's latency bar.
        assert_eq!(derive_intra_threads(1, base), 6);
        // The centralized shape an operator sets with `NER_POOL_SIZE=2` on this box: 3 each — the
        // named, accepted cost of one rule at every pool size. Note what this means and why it is
        // documented rather than hidden — a LONE request reaches 3, not 6, because one request only
        // ever occupies one session (M7/S1a); the other 3 cores are there for a *second* concurrent
        // request, and the SMT siblings are left to the runtime's own latency-bound work.
        assert_eq!(derive_intra_threads(2, base), 3);
        assert_eq!(derive_intra_threads(4, base), 1);

        // The derivation itself is unchanged — same arithmetic, different divisor. Pinned on a bare
        // base so a future change to `derive_thread_base` cannot quietly rewrite these too.
        assert_eq!(derive_intra_threads(1, 12), 12);
        assert_eq!(derive_intra_threads(2, 12), 6);
        assert_eq!(derive_intra_threads(4, 12), 3);
    }

    #[test]
    fn never_returns_zero_on_a_small_box_or_a_silly_pool() {
        // A 2-core VM with the default pool, and the degenerate cases. Zero would mean "ONNX
        // Runtime picks for me" — i.e. every session grabbing every core, which is exactly the
        // oversubscription we are avoiding. It must clamp to 1 instead.
        assert_eq!(derive_intra_threads(2, 1), 1);
        assert_eq!(derive_intra_threads(8, 2), 1);
        // A zero pool is clamped by `load`, but the arithmetic must not divide by zero here.
        assert_eq!(derive_intra_threads(0, 12), 12);
    }
}

#[cfg(test)]
mod chunk_tests {
    use super::chunk_char_ranges;

    /// Build a fake offsets table shaped like a real `encode(_, true)` output:
    /// a leading `(0, 0)` for `<s>`, one `(i, i+1)` per content token, and a
    /// trailing `(0, 0)` for `</s>` — the exact shape that hid the M5 chunking
    /// bug (the closing special token's sentinel offset, mistaken for the real
    /// text end).
    fn fake_offsets(content_tokens: usize) -> Vec<(usize, usize)> {
        let mut offsets = vec![(0, 0)]; // <s>
        offsets.extend((0..content_tokens).map(|i| (i, i + 1)));
        offsets.push((0, 0)); // </s>
        offsets
    }

    /// An all-ASCII input of `n` bytes, so the `(i, i+1)` offsets of
    /// [`fake_offsets`] line up one-to-one with its bytes.
    fn ascii(n: usize) -> String {
        "a".repeat(n)
    }

    #[test]
    fn a_single_window_covers_the_whole_input_when_it_fits() {
        let input = ascii(10);
        let offsets = fake_offsets(10); // seq = 12 (incl. <s>/</s>)
        let ranges = chunk_char_ranges(&input, &offsets, 480, 32);
        assert_eq!(ranges, vec![(0, 10)]);
    }

    #[test]
    fn the_last_window_reaches_the_true_text_end_not_the_closing_token_sentinel() {
        // seq = 22 (<s> + 20 content + </s>); window=10, overlap=2 → stride=8.
        // Windows (token space): [0,10) [8,18) [16,22). The last one's final
        // token index is 21 = seq-1, the `</s>` sentinel — this is exactly the
        // case that must NOT collapse to a zero-length range.
        let input = ascii(20);
        let offsets = fake_offsets(20);
        let ranges = chunk_char_ranges(&input, &offsets, 10, 2);

        let last = *ranges.last().unwrap();
        assert_eq!(
            last.1, 20,
            "must reach the real text end, not the sentinel (0,0)"
        );
        assert!(last.1 > last.0, "the last window must not be empty");
    }

    #[test]
    fn windows_overlap_and_jointly_cover_every_char() {
        let input = ascii(50);
        let offsets = fake_offsets(50);
        let ranges = chunk_char_ranges(&input, &offsets, 10, 2);

        assert!(
            ranges.len() > 1,
            "50 content tokens must need more than one window"
        );
        assert_eq!(ranges.first().unwrap().0, 0, "coverage must start at 0");
        assert_eq!(
            ranges.last().unwrap().1,
            50,
            "coverage must reach the true end"
        );
        // Consecutive windows must overlap (never leave a gap).
        for pair in ranges.windows(2) {
            let (_, prev_end) = pair[0];
            let (next_start, _) = pair[1];
            assert!(
                next_start < prev_end,
                "windows {:?} -> {:?} leave a gap",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn every_window_is_sliceable_even_when_an_offset_lands_inside_a_multibyte_char() {
        // M5-R3. The window edges come from the **tokenizer**; treating them as `char`-aligned is
        // an assumption about a third-party library's output on attacker-influenced input, and
        // `&input[start..end]` *panics* if it's wrong — the one place on the masking path that
        // could, while `decode_entities` (M2-R6), `Vault::mask` and `overlap::materialize` all
        // refuse to. So feed offsets that deliberately cut a 3-byte `€` and a 4-byte `𝄞` in half
        // and assert the ranges come back sliceable anyway (widened, never narrowed).
        let input = "a€b𝄞c€d"; // 1 + 3 + 1 + 4 + 1 + 3 + 1 = 14 bytes
        assert_eq!(input.len(), 14);

        // Offsets that land *inside* the multi-byte chars (2 is mid-`€`, 6/7 mid-`𝄞`, 11 mid-`€`).
        let offsets = vec![(0, 0), (0, 2), (2, 6), (6, 7), (7, 11), (11, 13), (0, 0)];

        // Non-vacuity: the table really does carry the hazard. Without this the test could pass
        // on offsets that happen to be `char`-aligned and prove nothing — the exact way a guard
        // goes quietly blind (the M4-R13/M4-R24 lesson: ask what the corpus holds *constant*).
        assert!(
            offsets
                .iter()
                .any(|&(s, e)| !input.is_char_boundary(s)
                    || !input.is_char_boundary(e.min(input.len()))),
            "the offsets table must actually cut a multi-byte char, or this guard is vacuous"
        );

        let ranges = chunk_char_ranges(input, &offsets, 3, 1);

        for (start, end) in ranges {
            assert!(
                input.get(start..end).is_some(),
                "window {start}..{end} is not sliceable — it would panic in infer_chunked"
            );
        }
    }

    #[test]
    fn a_window_at_least_as_wide_as_the_sequence_produces_one_range() {
        let input = ascii(5);
        let offsets = fake_offsets(5);
        let ranges = chunk_char_ranges(&input, &offsets, 480, 32);
        assert_eq!(ranges, vec![(0, 5)]);
    }
}

#[cfg(test)]
mod ep_tests {
    use super::ExecutionProvider as Ep;

    #[test]
    fn parses_known_providers_case_insensitively_and_trims() {
        assert_eq!(Ep::parse("").unwrap(), Ep::Cpu);
        assert_eq!(Ep::parse("cpu").unwrap(), Ep::Cpu);
        assert_eq!(Ep::parse("  CPU ").unwrap(), Ep::Cpu);
        assert_eq!(Ep::parse("DirectML").unwrap(), Ep::DirectMl);
        assert_eq!(Ep::parse("dml").unwrap(), Ep::DirectMl);
        assert_eq!(Ep::parse("cuda").unwrap(), Ep::Cuda);
        assert_eq!(Ep::parse("TensorRT").unwrap(), Ep::TensorRt);
        assert_eq!(Ep::parse("trt").unwrap(), Ep::TensorRt);
        assert_eq!(Ep::parse("coreml").unwrap(), Ep::CoreMl);
        assert_eq!(Ep::parse("rocm").unwrap(), Ep::Rocm);
        assert_eq!(Ep::parse("openvino").unwrap(), Ep::OpenVino);
        assert_eq!(Ep::parse("webgpu").unwrap(), Ep::WebGpu);
    }

    #[test]
    fn an_unknown_provider_is_an_error_not_a_silent_cpu() {
        // The fail-closed rule for the accelerator knob (M8-R5, applied here): a typo must
        // surface at startup, not quietly run CPU while the operator believes a GPU is engaged.
        let err = Ep::parse("directl").unwrap_err();
        assert!(
            err.contains("directl"),
            "the error must name the bad value: {err}"
        );
        assert!(Ep::parse("gpu").is_err());
        // Vulkan is NOT an ONNX Runtime backend (WebGPU is the nearest cross-vendor EP) — it must
        // not look valid, or an operator would think they'd enabled an acceleration that isn't wired.
        assert!(Ep::parse("vulkan").is_err());
    }

    #[test]
    fn requesting_an_unavailable_provider_yields_cpu_as_the_effective_one() {
        // M9-R1's seam, without a model: `dispatch()` is what decides whether an accelerated
        // build is even attempted, and `Cpu` must produce `None` so the pool builder takes the
        // plain CPU path and reports `Cpu`. The full "requested vs effective" behaviour needs a
        // real model (see NER-EP-01 in tests/ner_perf.rs); this pins the branch it rests on.
        assert!(
            Ep::Cpu.dispatch().is_none(),
            "cpu must never build an accelerated session"
        );
        for ep in [Ep::DirectMl, Ep::Cuda, Ep::CoreMl, Ep::Rocm] {
            assert!(
                ep.dispatch().is_some(),
                "{ep:?} must produce a dispatch to attempt — its availability is decided by ORT, \
                 not by us skipping the attempt"
            );
        }
    }

    #[test]
    fn as_str_round_trips_through_parse() {
        for ep in [
            Ep::Cpu,
            Ep::DirectMl,
            Ep::Cuda,
            Ep::TensorRt,
            Ep::CoreMl,
            Ep::Rocm,
            Ep::OpenVino,
            Ep::WebGpu,
        ] {
            assert_eq!(
                Ep::parse(ep.as_str()).unwrap(),
                ep,
                "as_str/parse must round-trip for {ep:?}"
            );
        }
    }
}

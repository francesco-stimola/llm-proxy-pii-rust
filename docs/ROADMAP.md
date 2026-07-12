# Roadmap

Development is split into milestones. Each builds on the previous one and is
independently testable. The checkboxes track progress — keep them current as
work lands. **This document is the single source of truth for "what's next".**

## M0 — Project setup ✅
- [x] Repo, license (MIT), README (EN) + README.it.md (IT)
- [x] Port PII test cases from the old proxy → `tests/reference/old-proxy/`
- [x] Tracking docs (roadmap, ARCHITECTURE, TESTING, SETUP, DEVLOG)
- [x] Rust module scaffold; `cargo build` green (177 deps)
- [x] Install Rust toolchain (rustup, per-user, no admin) — cargo/rustc 1.97
- [x] MSVC linker via portable Build Tools (no admin) → `C:\Lavoro\Tools\MSVC`
- [x] Decisions locked: hybrid detection, CPU-first, `[KIND_N]` placeholders, IT + US

## M1 — Structured PII pipeline (CPU, no ML) ✅

Locales: **IT + US**. Placeholder format: **`[KIND_N]`** (e.g. `[EMAIL_1]`).

### Part A — Masking core ✅
Reproduce the old proxy's structured-PII behavior, better, in Rust.
- [x] `PiiDetector` trait + `PiiEntity` / `PiiKind` types (incl. `Secret`)
- [x] Deterministic recognizers: email, phone (IT/US), SSN, credit card (Luhn), IBAN, with priority-based overlap resolution
- [x] SECRET recognizer (API keys / tokens, e.g. `sk-…`, `sk-ant-…`, `AKIA…`) — deterministic; the old ML model missed these
- [x] `Vault`: mask to `[KIND_N]` placeholders + exact-restore demask (deterministic per value)
- [x] Privacy stage wired into the pipeline (per-request `RequestContext` threads the `Vault` from request to response)
- [x] axum server forwarding `/v1/chat/completions` upstream (non-streaming), `reqwest` client, `/healthz`, config from env
- [x] Port the reference tests to Rust; no false positives; multi-PII round-trip exact (corpus-driven + proptest)

### Part B — Prompt augmentation & round-trip  ⭐ primary feature ✅
Make the masked data actually usable by the model — without this it mishandles
placeholders, especially in tool calls. A headline capability, not a nice-to-have.
- [x] Transparent system-prompt injection: teach the model that `[KIND_N]` are typed real values, to be used verbatim (incl. tool-call arguments) and never altered
- [x] Round-trip covers `tool_calls` arguments (de-anon in responses) and tool results (re-anon in requests)
- [x] Deterministic placeholder assignment (same value → same token across turns)
- [x] Integration tests INT-01…06 green (+ E2E-01/03 against a mock upstream)
- [x] Binary smoke test (`tests/binary_smoke.rs`) — boots the real `.exe` end-to-end

## M1.5 — Robustness & fail-closed  🔒 ✅
For a privacy tool the failure mode *is* the product: when anything is unexpected,
it must **fail closed** (block / scrub), never forward raw PII. Hardens M1 before
we broaden detection.
- [x] **Fail-closed policy**: unrecognized payload shapes, endpoints, or internal errors never pass PII through un-masked. The privacy stage sets a `RequestContext.block` on an unreadable `content` shape or missing `messages`; the proxy then returns 400 and never forwards. Unproxied paths → 404 (`fallback`).
- [x] **Full field coverage**: `messages[].content` (string + `text` of array parts), `messages[].name`, `tool_calls[].function.arguments`, legacy `function_call.arguments`, `tools[].function.description`, and every `description` inside `tools[].function.parameters`. All roles scanned (system/developer included).
- [x] **API scope decision**: **chat/completions only** is proxied; `/healthz` is served; everything else 404s and is never forwarded (documented in `ARCHITECTURE.md` and `src/server.rs`).
- [x] **Adversarial / evasion tests** (`tests/adversarial.rs` + corpus): broadened phone shapes ((555) 867-5309, dots, +1, extra IT grouping) and IBAN-before-word now caught; obfuscated emails pinned as a documented recall gap for NER/M2.
- [x] **Body-size limit**: `MAX_BODY_BYTES` (default 16 MiB) via `DefaultBodyLimit`, above axum's 2 MiB default so long-context requests aren't silently 413'd.
- [x] **Demask robustness**: a single tolerant pass restores `[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]` etc.; an unresolved but known-kind placeholder is logged (never silently shipped).

### Code-review findings — all closed ✅
8 findings from the M1 and M1.5 reviews — IBAN over-match, `iban_mod97` wiring,
single `system` message, response-header allowlist, tolerant de-mask; plus the
array-content fail-closed gap, phone over-match, and `Confidence` consumed — all
fixed with tests. Full detail in `docs/DEVLOG.md` (2026-07-11) / commits
`bb68707`, `775945f`.

## M2 — Unstructured entities (ONNX NER, CPU) ✅
Goal: add names / organizations / locations via a local ML model.
Candidate models & evaluation plan: [docs/M2-NER-EVALUATION.md](M2-NER-EVALUATION.md).

**Done:** hybrid detector + `OnnxNerDetector` (`--features onnx`), the model
**measured and picked** (XLM-R int8, head-to-head vs Piiranha — see DEVLOG
2026-07-12), and all 9 review findings closed. The NER runs off-repo model files
via env vars (`NER_MODEL_PATH` / `NER_TOKENIZER_PATH` / `NER_LABELS`); the shipped
default build stays feature-off and native-dep-free.
- [x] `OnnxNerDetector` behind the `onnx` feature (CPU EP) — `src/pii/onnx.rs`: HF tokenizer + `ort` session, per-token argmax → BIO decode; `--features onnx` compiles clean. Runtime verification is gated on a chosen model.
- [x] **Evaluate candidate models & pick one** — DONE (measured, head-to-head, `--features onnx`). **Picked: XLM-R int8** (`jiting/xlm-roberta-base-ner-hrl_onnx` @ `478a2a3`). Numbers + method in `docs/DEVLOG.md` (2026-07-12).
  - [x] **Eval harness** — `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d): scores a live candidate against `ner_cases.json` (recall/precision/F1 per type + timing) **through the hybrid resolver**. Runs with `cargo test --features onnx --test ner_eval -- --ignored --nocapture`.
  - [x] **Candidate A — XLM-R** (drop-in baseline): int8, hybrid **Org 1.00 / Loc 1.00 / Person 0.75** (only misses the single-token "Caia" split + the "Herr" title), ~23 ms/case CPU, 266 MB, multilingual, no `token_type_ids`, no label mapping. **Winner.**
  - [x] **Candidate B — Piiranha** (PII specialist): int8, hybrid **~0.00 recall** on natural-sentence NER (fires only fragments like "Pinc"/"New") and has **no Organization label** — it's a form/structured-PII model, not free-text NER. Rejected. (Its granular labels wired via extended `label_to_kind`; `type_vocab_size:0` → no `token_type_ids` needed.)
  - [x] **Pick + record** → XLM-R int8; numbers in `docs/DEVLOG.md`. GLiNER escalation not needed (XLM-R clears the bar).
- [x] Combine deterministic + ML detectors behind one `PiiDetector` — `CompositeDetector` fans out to N detectors and merges via the shared `overlap::resolve_overlaps` (structured PII outranks NER via `PiiKind::priority`)
- [x] Extend the corpus with unstructured-entity cases — `tests/corpus/ner_cases.json` (Person/Org/Location, IT+EN, single-word names, REG-03 negatives, a DE multilingual preview)
- [x] **NER concurrency**: `OnnxNerDetector` holds a round-robin **pool of sessions** (`NER_POOL_SIZE`) so inference isn't a single-threaded bottleneck.
- [x] *(micro, from M1.5 review)* **Symmetric response de-masking** — `demask_content` now mirrors `mask_content` (bare-string array elements restored too).

**Fail-closed for NER — RESOLVED.** A `NER_REQUIRED` switch makes the NER fail
closed: a required detector's load *or* inference error is fatal / blocks the
request via the `PiiDetector::try_detect` error channel; when unset (the default),
a `FailOpen` wrapper keeps the explicit fail-open posture for names.

### M2 review findings — ALL 9 closed ✅
R1–R9 (fail-closed NER via `NER_REQUIRED`/`try_detect`, decode/onnx robustness, the
eval harness) are all closed with tests. Full detail + the measured model pick
(XLM-R int8) in `docs/DEVLOG.md` (2026-07-12); commits `244b49e`, `1e473ce`.

### M2 completion review (2026-07-12) — new findings
Verified independently: **65 tests green (default), no warnings**;
`cargo build --features onnx` + the onnx test targets compile clean. The chosen
model is off-repo and was **not** locally available, so the DEVLOG recall numbers
were **not** independently reproduced — the eval harness and rejection reasoning were
code-reviewed instead and are sound. One minor follow-up:
- [ ] **M2-R10 (minor, harness precision) — eval TP/FP/FN use `Vec::contains`, so a
  case with a duplicate `(kind, text)` entity mis-counts.** `tests/ner_eval.rs:118-130`
  scores membership, not multiset: two identical expected entities both match a single
  detection (recall inflated 0.5→1.0), and a spurious duplicate detection isn't an FP.
  No current corpus case has duplicate `(kind, text)` in one sentence, so the recorded
  numbers are unaffected — but a future case would silently inflate recall in a
  leak-measurement harness. **Fix:** count against a consumed multiset (drain matched
  detections, e.g. a per-`(kind,text)` count map). **Test:** a synthetic case with a
  repeated entity where the model finds it once asserts recall 0.5, not 1.0.

## M2.5 — HuggingFace model management (auto-download via `hf-hub`)
Ergonomics + reproducibility, **not a correctness gap** (M2 works). Today the NER
model is downloaded by hand and lives in a plain folder; the standard HF cache
(`~/.cache/huggingface/hub`) is a **library-managed** content-addressed tree
(`refs`/`blobs`/`snapshots`) you must not hand-populate. Use the official **`hf-hub`
Rust crate** (behind the `onnx` feature) to fetch the **revision-pinned** model into
that standard cache — conventional location, dedup, reproducible. Pure Rust: the
default build stays native-dep-free.
- [ ] Add `hf-hub` as an **optional** dep, wired into the `onnx` feature only.
- [ ] Resolve a model file from `(repo, revision, filename)` → a local path in the HF
  cache. **`NER_MODEL_PATH` (explicit local file) keeps priority** — anyone wanting
  zero outbound calls just sets it, exactly as today.
- [ ] **Opt-in** auto-download: when `NER_MODEL_PATH` is unset but `NER_MODEL_REPO`
  (+ pinned `NER_MODEL_REVISION`, default `478a2a3`, + `NER_MODEL_FILE` e.g.
  `onnx/model_quantized.onnx`, + tokenizer file) is set, fetch via `hf-hub`. It's a
  **one-time model fetch (not user data)** — log it; never silent.
- [ ] Derive `NER_LABELS` from the model's `config.json` `id2label` (in class-id
  order) so the hand-typed list is optional (keep an explicit override).
- [ ] Docs: record the env contract + the privacy note (single opt-in outbound model
  fetch) in `ARCHITECTURE.md` / `SETUP.md`; default XLM-R repo/revision pinned.
- [ ] Tests: path-resolution + `id2label`-ordering logic unit-tested **without
  network**; the actual download exercised only by the `#[ignore]`d eval harness.

## M3 — Streaming
Goal: SSE token streaming with incremental de-anonymization.
- [ ] Streaming passthrough of provider responses
- [ ] Hold-back buffer that restores placeholders split across chunks

## M4 — GPU optimization & load
Goal: faster inference once the model is locked, and prove it holds under load.

**Why GPU is safely deferred (decided 2026-07-12):** the M2 model choice is
**execution-provider-agnostic** — every candidate (incl. GLiNER) is standard ONNX
and runs on any `ort` EP, so GPU does *not* constrain model selection; we pick on
CPU recall/latency now and revisit the EP here. On this Windows/no-admin box the
natural EP is **DirectML** (any DX12 GPU, no CUDA/admin). Going to GPU is mostly a
config change: swap the EP and switch the weight file from int8 (CPU) to the
pre-shipped `model_fp16.onnx` (fp16 is the GPU sweet spot; int8 on GPU needs
EP-specific support). No premature GPU work before the CPU baseline is locked.
- [ ] GPU execution provider (CUDA / **DirectML** for no-admin Windows) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline (CPU int8 → GPU fp16)
- [ ] **Load / throughput harness** (concurrent connections, large bodies) — stability under load was the founding motivation; measure it, don't assume it

## M5 — Broad locale & language coverage (future)
Goal: extend PII coverage beyond IT + US to a wide set of locales and languages,
so the proxy protects data regardless of the user's language or the upstream
provider. Likely valuable once we move off the OpenAI model and serve broader
traffic — priority can be pulled earlier if real usage demands it.
- [ ] Locale-parametrized structured recognizers (phone, national IDs, IBAN countries)
- [ ] Multilingual NER model(s) for names / orgs / locations (evaluate vs a multilingual corpus)
- [ ] Extend the test corpus with multi-language / multi-locale cases
- [ ] Provider-agnostic verification (not tied to OpenAI-specific behavior)

## Backlog / later
Auth & rate-limiting stages, **TLS / running behind a TLS terminator**, structured
audit logging (**never log raw PII**), config-file support & container deployment,
additional providers, metrics/observability.

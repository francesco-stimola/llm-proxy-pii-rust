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
- [x] **M2-R10 (minor, harness precision) — eval TP/FP/FN counted membership, not
  multiset.** Fixed: `tests/ner_eval.rs` now scores through a `tally` helper that counts
  each `(kind, text)` as a multiset (`tp = min(expected, detected)`), so duplicate
  expected entities can't both match one detection and a spurious duplicate detection is
  an FP. Test `tally_counts_duplicates_as_multiset` (non-network) pins recall 0.5 (not
  1.0) for a two-expected / one-detected case. The recorded numbers were unaffected (no
  corpus case has a duplicate `(kind, text)` in one sentence), as predicted.

## M2.5 — HuggingFace model management (auto-download via `hf-hub`) ✅
Ergonomics + reproducibility, **not a correctness gap** (M2 works). Before this the
NER model was downloaded by hand into a plain folder; the standard HF cache
(`~/.cache/huggingface/hub`) is a **library-managed** content-addressed tree
(`refs`/`blobs`/`snapshots`) you must not hand-populate. Now the official **`hf-hub`
Rust crate** (behind the `onnx` feature) fetches the **revision-pinned** model into
that standard cache — conventional location, dedup, reproducible. Pure Rust: the
default build stays native-dep-free. Implemented in `src/pii/hf.rs`, wired in
`src/server.rs::load_onnx_ner`; verified end-to-end (`docs/DEVLOG.md` 2026-07-12).
- [x] Added `hf-hub = { version = "1", optional = true }`, wired into the `onnx`
  feature only (async API; **default features**). *Its real dep footprint is heavier
  than first recorded — see **M2.5-R1**.* The **default** build stays native-dep-free.
- [x] Resolve `(repo, revision, filename)` → a local cache path (`HfModelSpec::resolve`).
  **`NER_MODEL_PATH` (explicit local file) keeps priority** — set it for zero outbound
  calls, exactly as before.
- [x] **Opt-in** auto-download: when `NER_MODEL_PATH` is unset but `NER_MODEL_REPO`
  (`owner/name`) is set, fetch via `hf-hub`. Tunable `NER_MODEL_REVISION` (default
  `478a2a3`), `NER_MODEL_FILE` (default `onnx/model_quantized.onnx`),
  `NER_TOKENIZER_FILE` (default `tokenizer.json`), `NER_CONFIG_FILE` (default
  `config.json`). **One-time model fetch (not user data)** — logged, never silent.
- [x] Derive `NER_LABELS` from `config.json` `id2label` (class-id order,
  `parse_id2label`, contiguity-checked → fail closed); explicit `NER_LABELS` overrides.
- [x] Pin the **standard** cache: `hf-hub` 1.0 falls back to `/tmp/.cache` on Windows
  (no `HOME`), so `build_client` sets `cache_dir = <home>/.cache/huggingface/hub` when
  `HF_HOME`/`HF_HUB_CACHE` are unset (otherwise it defers to them). Models now dedupe
  with every other tool on the box.
- [x] Docs: env contract + privacy note in `ARCHITECTURE.md` / `SETUP.md`; default
  XLM-R repo/revision pinned.
- [x] Tests: `parse_id2label` ordering/validation + `standard_hub_cache_dir` unit-tested
  **without network**; the real download is exercised only by the `#[ignore]`d eval
  harness (which grew a `NER_MODEL_REPO` path). Manual model copies removed; the model
  now lives in the standard HF cache.

### M2.5 review findings (2026-07-12)
Independently verified: **65 tests green (default) / 72 + 1 `#[ignore]`d (`--features
onnx`) / no warnings**; `cargo build --features onnx` clean. **Live numbers reproduced**
through the auto-download path (model resolved from the standard HF cache — no
re-download): **Org 1.00 / Loc 1.00 / Person 0.75** on the required corpus (the printed
aggregate 0.60/0.50/0.545 folds in the DE `multilingual_preview` "Herr Müller" case, as
DEVLOG notes) — this closes the prior review's "numbers not independently reproduced"
caveat. The fail-closed paths all hold: required-fetch failure is fatal at startup
(`load_onnx_ner → build_detector → AppState::new → run`); `parse_id2label` fails closed on
missing / non-contiguous / non-integer `id2label`; the outbound fetch is opt-in
(`NER_MODEL_REPO` only), logged, and never fires when `NER_MODEL_PATH` wins or the repo is
unset. Two non-blocking follow-ups:
- [ ] **M2.5-R1 (leanness + docs accuracy) — DECIDED 2026-07-12: keep `hf-hub 1.0` (Option A).**
  Confirmed the footprint with
  `cargo tree --features onnx`: `hf-hub 1.0.0` pulls a **second `reqwest 0.13.4`** (the
  in-tree one is `0.12.28`), **`hf-xet 1.5.3`**, `rustls 0.23` → **`aws-lc-rs` + `aws-lc-sys`**
  (native C/asm crypto), `reqwest-middleware 0.5`, and `ureq 3`. So the "built on the
  `reqwest` already in the tree — no new native deps" claim (`Cargo.toml` comment,
  `docs/DEVLOG.md`, the M2.5 dep bullet above) is **inaccurate**. The **default** build is
  unaffected — hf-hub is `onnx`-gated, so a plain `cargo build`/`cargo test` stays
  native-dep-free.
  - **Why the DECIDED trim (`default-features = false`, minimal features) does NOT work on
    1.0:** verified in `hf-hub 1.0.0`'s own `Cargo.toml` that `hf-xet` and `reqwest 0.13`
    are **non-optional** dependencies — not behind any feature. Its *only* features are
    `blocking`, `rustls-tls`, `socks`, `default = []`. No feature combination can drop
    `hf-xet` / `aws-lc-rs` / the duplicate `reqwest`; `default-features = false` is a no-op
    (default is already empty).
  - **The only way to shed them is a downgrade to a pre-Xet release.** `hf-hub 0.4.3` has
    `reqwest ^0.12` as an **optional** dep (unifies with the in-tree `reqwest 0.12.28`,
    native-tls/schannel) and **no `hf-xet`, no `aws-lc-rs`, no `rustls`, no second reqwest**
    — `hf-hub = { version = "0.4.3", default-features = false, features = ["tokio"] }` would
    reuse the in-tree reqwest and add only small pure-Rust crates (`dirs`, `indicatif`,
    `num_cpus`, `rand`). Its async API differs (`api::tokio::ApiBuilder` + `Repo::with_revision`)
    so `src/pii/hf.rs` would need a rewrite, and its default cache uses `dirs` (correct on
    Windows) so the `/tmp` workaround could be dropped.
  - **Decision (user, 2026-07-12) — Option A: keep `hf-hub 1.0`.** The whole footprint is
    `onnx`-gated, and the `onnx` feature is *already* native (`ort` links ONNX Runtime C++), so a
    maintained/audited crypto lib is no new category — the **default build stays native-dep-free**.
    Downgrading to the **unmaintained** 0.4.3 (no upstream CVE fixes) is the worse security posture
    for a privacy tool; a hand-rolled fetch is the fallback only if the supply-chain surface ever
    becomes a concrete problem. **Remaining tasks (builder):** correct the three inaccurate "no new
    native deps" claims (`Cargo.toml` comment, `docs/DEVLOG.md`, the M2.5 dep bullet) to the real
    onnx-only footprint; add a **footprint guard** test (`cargo tree` with no features contains no
    `hf-hub`).
- [x] **M2.5-R2 (fail-closed hygiene, minor).** Fixed: `parse_id2label` now
  `anyhow::ensure!(!pairs.is_empty(), …)` after the contiguity check, so an **empty**
  `id2label` object fails closed at load instead of returning `Ok(vec![])` and only
  surfacing later at request time. Test `empty_id2label_is_an_error` asserts
  `parse_id2label(r#"{"id2label":{}}"#).is_err()` (`src/pii/hf.rs`).

## M2.6 — Debug & observability modes  🔍 ✅ *(small, pullable — independent of M3)*
Opt-in developer tools to confirm, with your own eyes, that masking holds end-to-end.
**Off by default; neither weakens the fail-closed posture** (request-side masking always
runs). Small and independent of M3 — can be pulled early to eyeball that the system holds.
- [x] **`PII_DEBUG_SKIP_DEMASK`** (opt-in, off by default): skips the response de-mask so
  the client receives the **placeholders** (`[EMAIL_1]`, …) exactly as the provider saw
  them — visual proof the request left masked and the round-trip is wired. On `Config`
  (`debug_skip_demask`), not a bare env read, so it's isolated + testable; a **loud
  `warn!` startup log** fires when enabled. Request-side masking still runs (upstream never
  sees raw PII). **Test:** `e2e_debug_skip_demask_returns_placeholders_to_client` — the
  client gets `[EMAIL_1]` (not the value) while the upstream still saw the masked body.
- [x] **Debug-log the masked upstream body** at **`trace!`** (gated by `RUST_LOG`,
  **orthogonal** to the skip flag): `chat_completions` emits `trace!(masked_request = …)`
  just before forwarding — the body is masked at that point, so it's safe. `debug!` stays
  for the concise kind-only audit lines, so `RUST_LOG=…=debug` is readable and `=trace`
  opts into full-body dumps.
- **Safety boundary (design rule) — upheld:** the masked request → provider and the raw
  provider response (placeholders only) are safe to log; the **final de-masked client
  output (real values) is NEVER logged** (the only body logs are the `trace!` masked
  request and a body-less `debug!` when skipping de-mask). Same bar as the future audit
  logging in Backlog.

### M2.6 + M2.5-R2 review (2026-07-12) — sound, no blockers
Independently verified: **66 tests green (default) / 74 + 1 `#[ignore]`d (`--features onnx`),
no warnings**; `cargo test --features onnx --no-run` compiles the whole tree clean. All
fail-closed paths hold: request-side masking (`on_request`) runs **unconditionally** in
`chat_completions` — `PII_DEBUG_SKIP_DEMASK` guards **only** the `on_response` de-mask loop,
so the upstream never sees raw PII with the flag on; the flag is off by default, emits the
loud startup `warn!`, and the fail-closed block path returns before it is consulted. The
`trace!(masked_request)` fires **after** masking and before forwarding (placeholders only),
and the de-masked client response is never logged (verified by inspection + a codebase-wide
log-site sweep: every log emits kind / placeholder-token / static text only). `parse_id2label`
now `ensure!(!pairs.is_empty())` fails closed on empty `id2label` (`empty_id2label_is_an_error`).
Two **non-blocking** hardening nits (optional — the milestone is sound):
- [ ] **Log-safety regression test (nit, hardens the #1 risk).** The `trace!` masked-body log
  and the never-log-de-masked-output rule are only inspection-verified (DBG-01). Add a
  tracing-capture test that drives a PII request and asserts the emitted `trace!` contains the
  placeholder (`[EMAIL_1]`) and **not** the raw value, and that no log line carries the
  de-masked client response — so a future refactor can't silently turn logging into a leak.
- [ ] **De-duplicate `env_flag` (nit, cleanup).** The boolean-env helper is now defined twice
  with identical semantics (`src/config.rs` + `src/server.rs`); consolidate into one shared
  helper so flag-parsing (`1`/`true`/`yes`/`on`) can't diverge between `PII_DEBUG_SKIP_DEMASK`
  and `NER_REQUIRED`.

## M3 — Streaming & multi-provider routing (Option A)
Goal: SSE token streaming with incremental de-anonymization **and** routing to multiple
providers via their **OpenAI-compatible** endpoints (Option A). The two intertwine
(real Copilot/Anthropic usage is streamed), so they land together.

### Streaming
- [ ] Streaming passthrough of provider responses
- [ ] Hold-back buffer that restores placeholders split across chunks

### Multi-provider routing — Option A (OpenAI-compat normalization)
Route every provider through its **OpenAI-compatible** endpoint so a **single schema**
feeds the PII masker — no new leak surface (a per-schema masker is Option B, Backlog).
Must at least cover **GitHub Copilot** and **Anthropic** (via its OpenAI-compat layer).
- [ ] **Provider selection / routing** — today there is a single static `UPSTREAM_BASE_URL`; add per-provider config/selection.
- [ ] **Path flexibility** — the upstream path is hardcoded `/v1/chat/completions` (`proxy.rs`); Copilot uses `/chat/completions` (no `/v1`), so the path must be per-provider.
- [ ] **Per-provider auth & required headers** — Bearer today; Copilot needs its short-lived token + editor headers; Anthropic uses `Bearer` on its OpenAI-compat endpoint.
- [ ] **Request-header passthrough policy** — today only `Authorization` is forwarded; decide what else (e.g. `anthropic-version`) is safe to pass.
- Anthropic's **native** `/v1/messages` schema is deliberately **out of scope** here — that's Option B (Backlog).

## M4 — Broad locale & language coverage (future)
Goal: extend PII coverage beyond IT + US to a wide set of locales and languages,
so the proxy protects data regardless of the user's language or the upstream
provider. Likely valuable once we move off the OpenAI model and serve broader
traffic — priority can be pulled earlier if real usage demands it.

**Refocused after the M2 model choice (2026-07-12):** the *multilingual NER* half is
already **acquired** — the chosen XLM-R covers 10 languages (incl. IT/EN/FR/ES/DE), so
this milestone's barycenter shifts to the **structured recognizers**, which are still
hard-coded **IT + US** (the NER model does nothing for phone/national-ID/IBAN formats).
- [ ] **Locale-parametrized structured recognizers** (phone, national IDs, IBAN countries) — the meat of this milestone; the deterministic regex layer is still IT + US only.
- [ ] **Validate** the already-multilingual XLM-R against a multilingual corpus — validation, *not* model selection (the NER is already multilingual); pull languages beyond IT/EN/DE-preview into the corpus.
- [ ] Extend the test corpus with multi-language / multi-locale cases
- [ ] Provider-agnostic verification (not tied to OpenAI-specific behavior)

## Backlog / later — documented, not scheduled

### GPU optimization & load  *(was M4 — deferred 2026-07-12; documented, not scheduled)*
Faster inference once the model is locked, and prove it holds under load. **Deferred:**
the M2 model choice is **execution-provider-agnostic** — every candidate is standard
ONNX and runs on any `ort` EP, so GPU constrains nothing upstream; no reason to spend on
it until real latency/load demands it. On this Windows/no-admin box the natural EP is
**DirectML** (any DX12 GPU, no CUDA/admin); going to GPU is mostly a config change (swap
the EP; switch int8 → the pre-shipped `model_fp16.onnx`).
- [ ] GPU execution provider (CUDA / **DirectML** for no-admin Windows) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline (CPU int8 → GPU fp16)
- [ ] **Load / throughput harness** (concurrent connections, large bodies) — stability under load was the founding motivation; measure it when prioritized

### Option B — native provider adapters  *(documented, not scheduled)*
The heavy alternative to M3's Option A: support each provider's **native** API (e.g.
Anthropic's `POST /v1/messages`) instead of its OpenAI-compat endpoint. Needs a
**per-provider, schema-aware masking adapter** (Anthropic uses `system`, content blocks,
`tool_use`/`tool_result`, `tools[].input_schema`), plus native auth (`x-api-key` +
`anthropic-version`) and paths. **Higher leak risk** — a missed schema field is a leak —
so it stays unscheduled until a concrete need outweighs the OpenAI-compat path (M3/Option A).

### Other later items
Auth & rate-limiting stages, **TLS / running behind a TLS terminator**, structured
audit logging (**never log raw PII**), config-file support & container deployment,
additional providers, metrics/observability.

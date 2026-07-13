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
  feature only (async API). **Footprint (M2.5-R1, corrected):** under `--features onnx`
  hf-hub 1.x pulls a **second `reqwest 0.13`** + **`hf-xet`** + **`rustls`/`aws-lc-rs`**
  (native crypto) — *not* "no new native deps". Kept on 1.x deliberately (see M2.5-R1).
  The **default** build stays native-dep-free (hf-hub is `onnx`-gated).
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
- [x] **M2.5-R1 (leanness + docs accuracy) — DECIDED 2026-07-12: keep `hf-hub 1.0` (Option A); builder tasks DONE.**
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
    becomes a concrete problem. **Builder tasks — DONE:** corrected the three inaccurate "no new
    native deps" claims (`Cargo.toml` comment, `docs/DEVLOG.md`, the M2.5 dep bullet) to the real
    onnx-only footprint; added the **footprint guard** `tests/dependency_footprint.rs`
    (`default_build_excludes_the_onnx_and_hf_stack`: `cargo tree` on the default features contains
    no `hf-hub`/`hf-xet`/`aws-lc`/`ort`/`tokenizers`).
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
- [x] **Log-safety regression test (nit, hardens the #1 risk).** Added `tests/log_safety.rs`
  (`crate_logs_carry_placeholders_never_raw_pii`): captures this crate's logs at `trace` while
  driving a real PII request, and asserts the emitted `trace!` contains the placeholder
  (`[EMAIL_1]`) and **not** the raw value — while the reply really did carry the de-masked value
  (so a log leak would be caught). A future refactor can't silently turn logging into a leak.
- [x] **De-duplicate `env_flag` (nit, cleanup).** Consolidated into one `pub(crate)`
  `config::env_flag`; `server.rs` imports it. `PII_DEBUG_SKIP_DEMASK`, `NER_REQUIRED`, and
  `NER_TOKEN_TYPE_IDS` now share the single `1`/`true`/`yes`/`on` parser — no divergence.

## M3 — Streaming & multi-provider routing (Option A) ✅
Goal: SSE token streaming with incremental de-anonymization **and** routing to multiple
providers via their **OpenAI-compatible** endpoints (Option A). The two intertwine
(real Copilot/Anthropic usage is streamed), so they landed together.

**Done:** streaming SSE round-trip with an incremental hold-back de-anonymizer
(`src/stream.rs`), and per-provider config presets (path / passthrough / extra headers)
so one binary fronts OpenAI, Copilot, or Anthropic's OpenAI-compat endpoint. Streaming
never weakens fail-closed: request-side masking runs first, so the provider only ever sees
placeholders. Details in `docs/DEVLOG.md` (2026-07-12).

### Streaming
- [x] Streaming passthrough of provider responses — `stream:true` now forwards with
  `Body::from_stream`; `content-type: text/event-stream`; clean requests (nothing masked)
  stream through untouched.
- [x] Hold-back buffer that restores placeholders split across chunks — `SseDemasker`
  keeps a per-choice buffer, emits up to the last point that could still be an incomplete
  placeholder, and holds the rest until the next delta (or stream end) resolves it. Units +
  an e2e test (`[EMAIL_1]` split across SSE events → client gets the real value).

### Multi-provider routing — Option A (OpenAI-compat normalization)
Route every provider through its **OpenAI-compatible** endpoint so a **single schema**
feeds the PII masker — no new leak surface (a per-schema masker is Option B, Backlog).
Covers **GitHub Copilot** and **Anthropic** (via its OpenAI-compat layer).
- [x] **Provider selection / routing** — `UPSTREAM_PROVIDER` (`openai`/`copilot`/`anthropic`)
  picks a preset; base URL + key stay env-driven. *(Deployment-level: one active provider
  per instance — request-level routing is a follow-up below.)*
- [x] **Path flexibility** — `upstream_chat_path` is per-provider (Copilot `/chat/completions`,
  OpenAI/Anthropic `/v1/chat/completions`); override with `UPSTREAM_CHAT_PATH`.
- [x] **Per-provider auth & required headers** — client `Authorization` wins, else the
  configured key as `Bearer`; `UPSTREAM_EXTRA_HEADERS` adds provider-required static headers.
- [x] **Request-header passthrough policy** — an allowlist (`forward_request_headers`, from the
  preset or `UPSTREAM_FORWARD_HEADERS`) forwards only named client headers (e.g.
  `anthropic-version`, Copilot editor headers) beyond `Authorization`.
- Anthropic's **native** `/v1/messages` schema is deliberately **out of scope** here — Option B (Backlog).

### M3 follow-ups
- [x] **Streaming de-anon of `delta.tool_calls[].function.arguments`.** `SseDemasker` now keys
  its hold-back buffers by field — `Content { choice }` **and** `ToolArg { choice, tool }` — so a
  placeholder split across streamed tool-call argument deltas is reassembled and de-masked too.
  Unit test `tool_call_arguments_split_across_deltas_are_restored`.
- [x] **Upstream streaming error propagation.** A mid-stream upstream error is now turned into a
  **terminal `event: error`** (after flushing any buffered content) and the stream ends cleanly,
  instead of aborting the client connection. `demasking_sse_body` is generic over the stream error
  so it's unit-tested without a network round-trip
  (`mid_stream_upstream_error_becomes_terminal_sse_event`).
- **Request-level provider routing** — **moved out of M3 to Backlog** (2026-07-12): per-instance
  `UPSTREAM_PROVIDER` already covers Copilot + Anthropic (one instance per provider); a clean
  per-request form is non-trivial. See Backlog.

### M3 review (2026-07-12) — sound, no blockers
Independently verified: **73 tests green (default) / 81 + 1 `#[ignore]`d (`--features onnx`), no
warnings**; `cargo test --features onnx` compiles **and** links the `ort` native lib clean (the
`binary_smoke` test boots the real linked binary). Fail-closed holds for streaming: `chat_completions`
runs the `on_request` mask stages **and** the block check *before* the `stream:true` branch
(`server.rs:245-267`), so a streaming request is masked identically and a blocked one returns 400 without
ever forwarding — the provider only ever sees placeholders; streaming de-anon (`src/stream.rs`) is
**client-side only** (safe direction). Confirmed sound: the hold-back tail is length-bounded to 32 chars
(our longest placeholder body, `[ORGANIZATION_NN`, is ~15 — it never over-holds), slices only on the
ASCII `[` boundary (no multi-byte panic), and **no content is dropped** — `demasking_sse_body` calls
`SseDemasker::flush()` exactly once at stream end, and `flush_pending` is idempotent via the `flushed`
guard (also fired pre-`[DONE]`), so the final held tail is always emitted; a mid-stream upstream error is
handled (propagated as an I/O error, no panic). Secrets redacted: `Config`'s manual `Debug` shows
`upstream_api_key` as `<redacted>` and `upstream_extra_headers` as **names only** (values may carry a
Copilot integration id / token), so the startup `info!(?config)` can't leak one; `forward_request_headers`
is an allowlist of header *names*, not secrets. Provider presets (`openai`/`copilot`/`anthropic`) set only
the OpenAI-compat *shape* (path / client-header allowlist / static headers) — masking is schema-based and
provider-independent, so no preset bypasses it. Log sweep: `src/stream.rs` emits **no** logs, and every
streaming/routing log site carries only static text / an error object / the placeholder-only masked body.
Deferred M3 follow-ups (tool-call-arg de-anon, request-level routing, terminal SSE error events) are all
genuine non-leaks. **Non-blocking** follow-ups found:
- [x] **M3-R1 (low, robustness — not a leak).** Fixed: `stream_chat_completions` now branches on the
  upstream response `content-type` — when it isn't `text/event-stream`, it falls back to the buffered
  path (`buffered_fallback`: forward the real status + content-type and run the `on_response` de-mask)
  instead of forcing SSE. So a `stream:true` request the upstream answers with a JSON error (401/429/…)
  reaches the client as that JSON error. Test `e2e_streaming_non_sse_error_falls_back_to_json` (upstream
  429 `application/json` → client sees 429 + `application/json` + the error body).
- [x] **M3-R2 (low-medium, correctness — not a leak; pre-existing).** Fixed: added
  `Vault::demask_json_string`, which JSON-string-escapes the substituted value (via
  `json_string_body`), and wired it into the **`arguments`** fields on both paths — buffered
  (`demask_response` in `src/pipeline/privacy.rs`) and streaming (`SseDemasker::demask_for` picks it for
  `StreamKey::ToolArg`). `content` still uses the plain `demask`. So a de-masked value containing a `"` /
  `\` / control char stays valid inner JSON and the client can parse the tool-call arguments. Tests:
  `demask_json_string_keeps_inner_json_valid` (vault), `tool_call_arguments_demask_stays_valid_json` +
  `content_demask_is_not_json_escaped` (buffered), `tool_call_arguments_deanon_stays_valid_json` (streaming).

## M4 — Broad locale & language coverage (future)
Goal: extend PII coverage beyond IT + US to a wide set of locales and languages,
so the proxy protects data regardless of the user's language or the upstream
provider. Likely valuable once we move off the OpenAI model and serve broader
traffic — priority can be pulled earlier if real usage demands it.

**Scope decided (2026-07-13) — M4 closes the multilingual question within a bounded domain, on two axes:**
- **Language (unstructured / NER):** declared support = the **NER model's languages** (XLM-R HRL:
  ar, de, en, es, fr, it, lv, nl, pt, zh). Beyond these we don't claim to catch names/orgs/locations —
  only structured PII (which is language-independent). If the model changes, the domain moves with it.
- **Structured PII — three tiers** (see M4-R1): **universal** (email, IBAN, card, secret) always on;
  **national IDs** (CF, NINO, SSN, + country packs) always on **regardless of `PII_LOCALES`** (privacy-first;
  each must be specific enough — see M4-R2); **FP-prone** recognizers (national phone formats, …) opt-in
  via `PII_LOCALES`. So `PII_LOCALES` gates *ambiguous* recognizers, **not** "which countries".
- [x] **Locale-parametrized recognizer architecture (first landing).** Split the recognizers into
  **universal** (email, secret, credit card, IBAN — already any-country — and phone US/`+CC`) plus
  per-locale **national-identifier** packs, selectable via `StructuredRecognizers::with_locales` /
  the `PII_LOCALES` env (default `it, us`, on `Config.pii_locales`). Added national IDs: **IT Codice
  Fiscale** and **GB NINO** (new `PiiKind::NationalId`, placeholder `[NATID_N]`). Tests:
  `italian_codice_fiscale_detected_by_default`, `uk_nino_needs_the_gb_locale`, `locale_selection_is_scoped`.
- [x] **Apply the three-tier structure (M4-R1; M4-R2 done first).** National IDs moved to the
  **always-on** tier (`national_id_recognizers()`, off `PII_LOCALES`); the locale-gating seam
  (`fp_prone_recognizers(code)`) kept for **FP-prone** recognizers only; NINO tightened first (M4-R2) so
  always-on can't over-mask. Scoping tests + `PII_LOCALES` docs (SETUP §6, ARCHITECTURE) updated.
- [~] **More national-ID country packs** — **ES DNI/NIE (mod-23) + FR NIR (mod-97) landed** (always-on,
  checksum-specific). **DE Steuer-ID deferred** (needs the ISO 7064 Mod 11,10 check + structural rules to
  hit near-zero FP as an always-on recognizer). More countries welcome on the same seam.
- [ ] **Locale phone national formats** (numbers without a `+CC`, e.g. UK `020 …`, DE `030 …`) — the
  **FP-prone tier**: gated by `PII_LOCALES` (the `fp_prone_recognizers` seam). Needs careful precision
  work (the `+CC` international arm already covers the unambiguous case).
- [ ] **IBAN per-country length validation** — structural + mod-97 already accept every country; add
  per-country length checks to raise precision if needed.
- [x] **Validate the NER across its declared domain** — scored XLM-R int8 on its **10 languages**
  (ar/de/en/es/fr/it/lv/nl/pt/zh) through the hybrid: **Person 0.83 / Org 1.00 / Loc 0.91** aggregate
  (numbers + per-language notes in `docs/DEVLOG.md` 2026-07-13). The 5 added Latin-script European
  languages match cleanly; ar/zh find names/cities with a minor boundary artifact.
- [x] Extend the test corpus with multi-language cases — `tests/corpus/ner_cases.json` `multilingual_preview`
  now covers ar/de/es/fr/lv/nl/pt/zh (en/it in the main set), one Person + Location each.
- [ ] Provider-agnostic verification (not tied to OpenAI-specific behavior).

**M4 is done when** the three-tier structure is in place, the national-ID packs + NER validation cover the
declared domain, and the corpus proves it — i.e. the multilingual question is **closed within the decided
scope**.

### M4 first-landing review (2026-07-12) — sound, no blockers
Independently verified: **83 tests green (default) / 91 + 1 `#[ignore]`d (`--features onnx`),
no warnings**; `cargo build --features onnx` links `ort` clean. The refactor holds — the
universal recognizers (email/secret/card/IBAN/phone) are byte-identical to the pre-M4 set and
US SSN stays active by default (via the `us` locale), so **no M1/M1.5 regression**: the
IBAN-no-trailing-word, phone-no-over-match, Luhn, and mod-97 guards all still pass. The new
national-ID regexes are **word-boundary anchored on both ends**, so a CF/NINO can't fire inside
a longer alphanumeric token (API key / hash / UUID / base64) — no over-mask that mangles legit
text, and no leak from over-anchoring on ordinary punctuation. `NationalId` shares priority
tier 3 with `Ssn`, but the two are structurally disjoint (SSN has dashes; CF/NINO are
contiguous) so they never overlap; `resolve_overlaps` ties fall through to span length
deterministically. No raw PII in logs (kind / placeholder token only); fail-closed posture
unchanged. **M3-R2 confirmed sound:** `demask_json_string` JSON-escapes the substituted value
and is wired **only** to the `arguments` fields (buffered `demask_response` + streaming
`ToolArg`); `content` keeps the plain `demask`; the round-trip is exact and
`demask_json_string_keeps_inner_json_valid` is **non-vacuous** (it asserts the plain path *would*
produce invalid JSON). Non-blocking follow-ups:
- [x] **M4-R1 (DONE 2026-07-13 — Option A: national IDs always-on, privacy-first).** National-ID
  recognizers run **regardless of `PII_LOCALES`** — a national ID that reaches the proxy is masked even
  if its locale isn't configured (recall > precision; "a miss is a leak"). `PII_LOCALES` is narrowed to
  gate only genuinely **FP-prone** recognizers (e.g. national *phone* formats, once added). **Prerequisite:
  M4-R2** — an always-on loose recognizer over-masks globally, so the NINO (and any future loose ID) must
  be tightened first. **Builder tasks:** move the national-ID recognizers out of `PII_LOCALES` gating into
  an always-on set; keep the locale-gating mechanism for FP-prone recognizers; update the scoping tests
  (`uk_nino_needs_the_gb_locale` / `locale_selection_is_scoped` change meaning) and the `PII_LOCALES` docs
  (SETUP §6, ARCHITECTURE) to the new semantics.
- [x] **M4-R2 (DONE 2026-07-13 — precision — GB NINO over-match; PREREQUISITE for M4-R1's always-on).** `\b[A-Za-z]{2}\d{6}[A-Da-d]\b`
  masks any 2-letter + 6-digit + A–D token (e.g. an order/reference code `PO123456A`) as `[NATID_N]` — an
  over-mask on legit text (not a leak). Under M4-R1's always-on policy it fires **globally**, so it must be
  tightened first. Tighten with
  the real NINO prefix rules (invalid 1st letter D/F/I/Q/U/V; 2nd letter D/F/I/O/Q/U/V; disallowed
  pairs BG/GB/KN/NK/NT/TN/ZZ) via a `validate` fn. Test: `PO123456A` (and a disallowed-prefix NINO)
  not masked; a valid NINO still is.
- [ ] **M4-R3 (precision — IT Codice Fiscale; optional).** The CF regex uses `[A-Za-z]` (accepts
  lowercase) and skips the **check character** (the final letter is a checksum), so a wrong-checksum
  or lowercase look-alike is still masked as `Verified`. FP risk is very low (anchored + highly
  specific interleave), but a CF check-char `validate` fn (like the deferred IBAN per-country checks)
  would raise precision and let a structure-only match be tagged `Structural`. Test: a valid CF stays
  `Verified`; a checksum-broken one is `Structural` (or rejected).
- [x] **M4-R4 (docs) — ARCHITECTURE mis-grouped US SSN under `NationalId`/`[NATID_N]`.** Fixed by the
  reviewer: clarified that US SSN keeps `PiiKind::Ssn`/`[SSN_N]`; only IT CF + GB NINO are
  `NationalId`/`[NATID_N]` (`docs/ARCHITECTURE.md`; `mod.rs` and `TESTING.md` were already correct).

## M5 — Integration & performance testing
Goal: prove the whole system holds **end-to-end** and **under load**, then document it. Comes after M4 —
the feature set (structured + NER + streaming + multi-provider) is complete enough to test as a product.
- [ ] **Real integration tests** — end-to-end scenarios beyond today's mock-upstream e2e: full
  mask → forward → (stream) → de-mask round-trips across the provider presets (OpenAI / Copilot /
  Anthropic shapes), tool-call round-trips, multi-turn determinism, and the fail-closed paths. Mock
  upstreams by default; optionally a smoke against a real provider (opt-in, never in CI without a key).
- [ ] **Performance / load harness** (pulled up from Backlog) — concurrent connections, large bodies,
  streaming throughput; measure latency / RAM of the mask → forward → de-mask path (NER on/off).
  Stability under load was the founding motivation — measure it, don't assume it.
- [ ] **Update the root `README.md`** (+ `README.it.md`) to reflect the shipped product — what it does,
  the three-tier detection + NER, streaming, multi-provider (per-instance) usage, config/env, and
  status. The README is intentionally high-level today ("early development"); this is the pass that
  makes it describe a working system.

## Backlog / later — documented, not scheduled

### GPU optimization  *(was M4 — deferred 2026-07-12; documented, not scheduled)*
Faster inference once the model is locked. **Deferred:**
the M2 model choice is **execution-provider-agnostic** — every candidate is standard
ONNX and runs on any `ort` EP, so GPU constrains nothing upstream; no reason to spend on
it until real latency/load demands it. On this Windows/no-admin box the natural EP is
**DirectML** (any DX12 GPU, no CUDA/admin); going to GPU is mostly a config change (swap
the EP; switch int8 → the pre-shipped `model_fp16.onnx`).
- [ ] GPU execution provider (CUDA / **DirectML** for no-admin Windows) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline (CPU int8 → GPU fp16)
- **Load / throughput harness** → moved up to **M5** (integration & performance testing); it's a CPU-throughput concern, independent of the GPU work here.

### Option B — native provider adapters  *(documented, not scheduled)*
The heavy alternative to M3's Option A: support each provider's **native** API (e.g.
Anthropic's `POST /v1/messages`) instead of its OpenAI-compat endpoint. Needs a
**per-provider, schema-aware masking adapter** (Anthropic uses `system`, content blocks,
`tool_use`/`tool_result`, `tools[].input_schema`), plus native auth (`x-api-key` +
`anthropic-version`) and paths. **Higher leak risk** — a missed schema field is a leak —
so it stays unscheduled until a concrete need outweighs the OpenAI-compat path (M3/Option A).

### Request-level provider routing  *(moved out of M3 — 2026-07-12; documented, not scheduled)*
Today provider selection is **per-instance** (`UPSTREAM_PROVIDER`, chosen at startup), which already
covers Copilot + Anthropic: **run one proxy instance per provider** (separate `LISTEN_ADDR`) and point
each client at the right port — no code needed. A *single* instance choosing the provider **per request**
is deferred because a clean, robust form is non-trivial: it needs a provider **map** in `Config` (N base
URLs / keys / presets) + a selection rule, and doing it well across providers likely means understanding
each provider's request conventions (model naming, required fields / headers) — which shades into Option B
territory once native schemas are involved. **If pursued, prefer routing by the request's `model`** (over
the Option A OpenAI-compat normalization): no client changes, no custom headers, composes with the existing
presets; keep it opt-in (one configured provider ⇒ today's behaviour). Not a privacy change — masking runs
before routing, so a mis-route is a wrong-provider error, never a leak.

### Other later items
Auth & rate-limiting stages, **TLS / running behind a TLS terminator**, structured
audit logging (**never log raw PII**), config-file support & container deployment,
additional providers, metrics/observability.

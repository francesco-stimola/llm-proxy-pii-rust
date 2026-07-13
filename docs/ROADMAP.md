# Roadmap

Development is split into milestones. Each builds on the previous one and is
independently testable. The checkboxes track progress — keep them current as
work lands. **This document is the single source of truth for "what's next".**

## M0 — Project setup ✅
- [x] Repo, license (**AGPL-3.0-or-later** — relicensed from MIT 2026-07-13), README (EN) + README.it.md (IT)
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

## M4 — Broad locale & language coverage ✅
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
- [x] **National-ID packs for all XLM-R-aligned countries** (decided 2026-07-13 — "all XLM-R languages").
  All landed, always-on, each checksum-gated: IT CF (check char — M4-R3), GB NINO, US SSN, ES DNI/NIE
  (mod-23), FR NIR (mod-97), **DE** Steuer-ID (ISO 7064 Mod 11,10 + the one-repeated-digit structural
  rule), **NL** BSN (11-proef), **PT** NIF (mod-11), **LV** personal code (classic mod-11 + the post-2017
  `32…` shape-only random form), **zh** China Resident ID (ISO 7064 MOD 11-2, 18 chars). **`ar` gets no
  pack** — the language spans ~20 countries with different ID schemes, so there is no single "Arabic"
  national ID; Arabic names/locations stay covered by NER. *(Overlap note: a numeric national ID that is a
  substring of an email local part is dropped by the **containment gate**, so a numeric email local part is
  never fragmented — see `overlap::drop_spans_contained_in_an_email`. This used to be a `Email`>national-ID
  **priority** rule; since M4-R9 `Email` is the lowest structured priority and the gate carries the case.)*
- **Locale phone national formats** → **moved to Backlog** (user's call 2026-07-13): the FP-prone tier's
  first recognizer. The `+CC` international arm already covers the unambiguous case; the `fp_prone_recognizers`
  seam stays ready. Add a specific national format only on concrete need. See Backlog.
- [x] **IBAN per-country length validation** — `confidence_of` now tags an IBAN `Verified` only when
  **both** mod-97 **and** its country's fixed length (ISO 13616 table, `iban_country_length`) check out;
  a wrong-length IBAN of a known country is still masked but flagged `Structural`. Unknown countries rely
  on mod-97 alone. Test `iban_per_country_length_gates_confidence`.
- [x] **Validate the NER across its declared domain** — scored XLM-R int8 on its **10 languages**
  (ar/de/en/es/fr/it/lv/nl/pt/zh) through the hybrid: **Person 0.83 / Org 1.00 / Loc 0.91** aggregate
  (numbers + per-language notes in `docs/DEVLOG.md` 2026-07-13). The 5 added Latin-script European
  languages match cleanly; ar/zh find names/cities with a minor boundary artifact.
- [x] Extend the test corpus with multi-language cases — `tests/corpus/ner_cases.json` `multilingual_preview`
  now covers ar/de/es/fr/lv/nl/pt/zh (en/it in the main set), one Person + Location each.
- [x] Provider-agnostic verification (not tied to OpenAI-specific behavior) — masking walks the
  OpenAI-shaped JSON and is provider-independent (presets only affect routing). Test
  `e2e_masking_is_provider_agnostic`: the same request via `openai` vs `anthropic` presets yields a
  byte-identical masked body upstream.

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
- [x] **M4-R3 (DONE 2026-07-13 — precision — IT Codice Fiscale).** Added `cf_check_valid` (the odd/even
  table + mod-26 check character); a wrong-checksum CF look-alike is now rejected (consistent with the
  other national IDs). Test `italian_codice_fiscale_checksum_rejects_broken`. *(Original note kept below.)*
  The CF regex uses `[A-Za-z]` (accepts
  lowercase) and skips the **check character** (the final letter is a checksum), so a wrong-checksum
  or lowercase look-alike is still masked as `Verified`. FP risk is very low (anchored + highly
  specific interleave), but a CF check-char `validate` fn (like the deferred IBAN per-country checks)
  would raise precision and let a structure-only match be tagged `Structural`. Test: a valid CF stays
  `Verified`; a checksum-broken one is `Structural` (or rejected).
- [x] **M4-R4 (docs) — ARCHITECTURE mis-grouped US SSN under `NationalId`/`[NATID_N]`.** Fixed by the
  reviewer: clarified that US SSN keeps `PiiKind::Ssn`/`[SSN_N]`; only IT CF + GB NINO are
  `NationalId`/`[NATID_N]` (`docs/ARCHITECTURE.md`; `mod.rs` and `TESTING.md` were already correct).

### M4 continuation review (2026-07-13) — sound, no blockers
Reviewed the always-on national-ID landing (`80da727` recognizers, `63aea5e` corpus, `705bac2` docs).
Independently verified: **87 tests green (default) / 95 + 1 `#[ignore]`d (`--features onnx`), no
warnings**; `cargo test --features onnx` links `ort` clean. *(DEVLOG/commit `80da727` say "86 / 94"
— off by one; the true counts are 87 / 95, no test regressed — a harmless miscount, not a failure.)*
- **Validators audited against the official algorithms — all correct.** GB NINO `nino_prefix_valid`
  matches HMRC (1st ∉ D/F/I/Q/U/V, 2nd ∉ D/F/I/O/Q/U/V, pairs BG/GB/KN/NK/NT/TN/ZZ, suffix A–D):
  `PO123456A`/`GB…`/`DA…` rejected, a valid NINO masks. ES DNI/NIE `es_dni_nie_valid` uses the exact
  mod-23 table `TRWAGMYFPDXBNJZSQVHLCKE` with NIE X/Y/Z→0/1/2 (hand-checked `12345678Z`→Z, `X1234567L`→L;
  wrong-letter look-alike rejected). FR NIR `fr_nir_valid` computes `97 − (body13 mod 97)` (key range
  1..=97, no underflow). No leak / no global over-mask from any validator.
- **Overlap is deterministic.** A 15-digit FR NIR that also passes Luhn is claimed by the higher-priority
  `CreditCard` (tier 4 > `NationalId` tier 3) → masked as `[CARD_N]`, single span, no double-mask, no
  silent drop (the `french_nir_detected` test pins "must never be left in clear"). ES DNI (9 chars, has a
  letter) can't collide with card/phone/SSN. No regression: `universal_recognizers()` is byte-identical;
  US SSN moved from the `us` locale to the **always-on** tier (broader coverage, still on by default).
- **NER numbers reproduced, not fabricated.** Ran the `#[ignore]`d `ner_eval` harness against the cached
  XLM-R int8 (`478a2a3`) over `multilingual_preview`: Person **0.833 / 0.714 / 0.769**, Org
  **1.000 / 1.000 / 1.000**, Location **0.909 / 0.909 / 0.909** — matches DEVLOG to 3 dp. The corpus
  labels are well-formed and the ar/zh boundary artifacts (`ب`, `在北京`) reproduce exactly as documented.
- **`PII_LOCALES` no-op confirmed intentional + wired.** `fp_prone_recognizers` returns empty for every
  code, so `PII_LOCALES` is a documented no-op (SETUP §6, ARCHITECTURE); the seam is still threaded
  end-to-end (`config.pii_locales` → `build_detector` → `with_locales` → `fp_prone_recognizers`), so a
  future FP-prone recognizer would be gated. No raw PII in logs (kind-only); fail-closed unchanged.
- [x] **M4-R5 (DONE 2026-07-13 — FR NIR completeness).** The month alternation now admits the INSEE
  special codes (`20`, `30–42`, `50–99`) so those real NIRs aren't missed on the always-on tier (the
  mod-97 key still gates precision). Test `french_nir_special_month_is_not_missed`. Corsica's `2A`/`2B`
  department (letters in the body) stays a **documented known limitation** (code comment + this item). *(Original note below.)*
  The NIR regex fixes
  the month field to `01–12` (`(?:0[1-9]|1[0-2])`), so a real NIR carrying an INSEE **special month code**
  — `20` (birth month unknown / born abroad) or the provisional `30–42` / `50–99` (SANDIA) ranges — never
  matches and, on the always-on tier, **leaks** (a miss = a leak). Corsica's `2A`/`2B` department form is
  the same class of gap (noted in the code comment but not tracked here). Fix: either broaden the month
  alternation to admit the valid special codes (the mod-97 key still gates precision at ~1/97) **or**
  document both as explicit known limitations like the deferred IBAN per-country work. Test: a NIR with
  month `20` + correct mod-97 key is masked (if broadened), or a doc note if scoped out.

### M4 close-out review (2026-07-13) — M4 genuinely COMPLETE, no blockers
Reviewed the close-out (`a474da8` recognizers + priority reorder + e2e, `2da01c0` docs, `91e67af`
proptest seed). Independently verified: **96 tests green (default), 0 failed, no warnings**;
`cargo build --features onnx` compiles **and** links `ort` clean, no warnings. *(The DEVLOG close-out
entry says only "all tests green" — accurate; the exact default count is 96, up 9 from the 87 of the
continuation review.)*
- **All checksum validators audited against the official algorithms — correct.** Hand-verified with an
  official test number each: **DE Steuer-ID** ISO 7064 Mod 11,10 (`86095742719` → check 9 ✓; the
  structural "exactly one repeated digit in the first 10" gate never rejects a valid ID); **NL BSN**
  11-proef (`111222333` → weighted sum 66, ✓; `sum != 0` excludes all-zeros); **PT NIF** mod-11
  (`123456789` → control 9 ✓); **zh Resident ID** ISO 7064 MOD 11-2 (`11010519491231002X` → 'X' ✓);
  **IT CF** odd/even table + mod-26 (`RSSMRA85T10A562S` → 'S' ✓, the full 26-entry ODD table matches);
  **ES DNI/NIE** mod-23 and **FR NIR** mod-97 re-confirmed. The 9-digit recognizer masks only when BSN
  **or** NIF accepts (a 9-digit that is neither is **not** masked — no over-mask); the 11-digit only when
  DE **or** LV accepts. No validator leaks or globally over-masks.
- **LV `32…` shape-only branch — conscious, documented tradeoff (confirmed).** It's the one always-on
  recognizer with no checksum (returns `true` for any 11-digit starting `32`); flagged in the code comment
  ("randomized form — no checksum to verify") and ROADMAP/ARCHITECTURE. FP surface = 1% of standalone
  11-digit tokens masked unconditionally — acceptable under the privacy-first (over-mask, never leak) rule.
- **Priority reorder is safe.** The only change is Email (2→3) crossing above Ssn/NationalId (3→2); Card/
  Iban/Secret stay above Email, Phone below. Email and Ssn/NationalId only overlap when the ID is a
  *substring of an email local part*, where the email is the complete correct match — so the reorder is a
  strict fix (`123456789@x.com`, and the latent `123-45-6789@x.com` SSN case) with **no new regression**.
  `Ssn ≈ NationalId` (tier 2) never share a span (SSN is dashed; the ID recognizers are letter-interleaved
  or contiguous-digit), so the tie is moot and deterministic. Provider-agnostic e2e genuinely asserts a
  byte-identical masked upstream body across the `openai`/`anthropic` presets. No raw PII in logs
  (kind-only `debug!`); fail-closed unchanged.

Three **non-blocking** precision follow-ups (all fail-safe — over-mask/utility, never a leak) — **all closed 2026-07-13**:
- [x] **M4-R6 (DONE 2026-07-13 — accepted-FP tradeoff, documented + pinned).** The pure-numeric always-on
  recognizers (`\b\d{9}\b`, `\b\d{11}\b`) over-mask a fraction of ordinary numbers (BSN ∪ NIF ≈ 2/11 ≈ 18%
  of arbitrary 9-digit tokens; DE ∪ LV for 11-digit, incl. the unconditional LV `32…` ~1%). **Resolved by
  documenting the accepted magnitude** (code comments on both recognizers, like the LV shape-only note) —
  **not** by context-gating: gating a national ID on a nearby keyword would reintroduce leaks and directly
  contradict the always-on M4-R1 decision ("a miss is a leak"). The clean precision path is the **contextual
  GLiNER detector (Backlog)**, not a regex keyword gate. Test `bare_numeric_national_ids_are_masked_by_design`
  pins the over-mask as intentional (`524287244` — an arbitrary PT-NIF-valid number — is masked) **and** that
  the checksum still filters the majority (`524287245` fails both → left in clear).
- [x] **M4-R7 (DONE 2026-07-13 — generalized to Card/Iban/Secret; *mechanism later corrected by M4-R9*).** An
  email whose local part is exactly a card / IBAN / secret must not be fragmented (`4111111111111111@x.com` →
  `[CARD_1]@x.com` forwards the `@domain` in clear). Originally fixed by making **Email the top structured
  priority** — **that was too broad and M4-R9 replaced it with a containment gate** (a global Email priority also
  wins *partial* overlaps, which leaks grouped forms — see M4-R9). The *behavior* asserted here is unchanged and
  still holds via the gate. Test `email_beats_a_card_iban_or_secret_local_part`: `4111111111111111@x.com`,
  `DE89370400440532013000@x.com`, `sk-abcdef123456@x.com` each mask as a single `Email`.
- [x] **M4-R8 (DONE 2026-07-13 — DE Steuer-ID consecutive-triple exclusion added).** `de_steuerid_valid` now
  enforces the 2016+ rule: a digit that appears three times in the first 10 must **not** sit in three
  *consecutive* positions. Raises precision (rejects a look-alike with a consecutive triple) with no recall
  cost (a valid ID never has one). Self-verifying test `de_steuerid_rejects_a_consecutive_triple`: same digits +
  valid checksum, consecutive → rejected, non-consecutive → accepted.

### M4-R6/R7/R8 precision follow-up review (2026-07-13) — one BLOCKER (priority-reorder leak) — **CLOSED**
Reviewed `3845bfe` (fix) + `4de5d38` (docs). Independently verified: **99 tests green (default), 0 failed, no
warnings**; `cargo check --features onnx` compiles clean (no warnings). M4-R6 and M4-R8 are **sound**:
M4-R6's `bare_numeric_national_ids_are_masked_by_design` genuinely asserts *both* halves (`524287244` masked
by design, `524287245` left in clear), the ~18% / ~1% FP-magnitude comments are accurate, and documenting (not
context-gating) is the correct call. M4-R8's consecutive-triple rule is a correct pure-precision gain with zero
recall cost, and `de_steuerid_rejects_a_consecutive_triple` is a genuine self-verifying test (both numbers carry
a valid Mod 11,10 check digit; only placement differs). But the **M4-R7 Email-top reorder introduces a real
Card/IBAN leak** on partial overlap:
- [x] **M4-R9 (DONE 2026-07-13 — FIXED with the recommended containment gate; leak closed).** Implemented the
  recommended fix, not the minimal revert, so the M4-R7 benefit is kept **and** the leak is closed. Two changes:
  (1) **`resolve_overlaps` gained an Email containment gate** (`drop_spans_contained_in_an_email`, run *before*
  the priority sort): a structured span that lies **entirely inside** an `Email` span is a false decomposition of
  its local part and is dropped, so the email wins — this is the `4111111111111111@x.com` / `123456789@x.com`
  case. A merely **partially** overlapping span (the space-grouped forms) is **not** dropped. (2) **`Email` moved
  to the *lowest* structured priority** (below `Phone`, above NER), so every *partial* overlap now falls through
  to priority and the checksum-backed structured span wins — masking the digits instead of leaving them in clear.
  The two mechanisms are complementary: containment → Email wins; partial overlap → structured wins. Also fixes
  the space-grouped **NINO** variant the reviewer flagged (which was a latent leak even *before* M4-R7, since
  `Email` already outranked `NationalId`). **Tests:** `grouped_forms_attached_to_a_domain_do_not_leak`
  (recognizers — the full grouped card/IBAN/NINO span is the single detected entity);
  `grouped_pii_glued_to_a_domain_leaks_nothing` (`tests/adversarial.rs` — asserts directly on the **masked body**
  that no card/IBAN/NINO group survives in clear, incl. the continuous containment case); plus the two resolver
  units `email_containing_a_structured_span_wins_it` / `email_partially_overlapping_a_structured_span_loses_it`.
  **Verified non-vacuous:** re-raising `Email` above the structured kinds makes them fail. The M4-R7 containment
  test stays green. *(Original finding below.)*
  M4-R7's
  claim ("a card/IBAN/secret can only overlap an email by being a *substring* of its local part") holds only for
  the **continuous** forms the test covers — it **breaks for the space-grouped Card and IBAN forms**, because the
  email local-part class `[A-Za-z0-9._%+-]` (`src/pii/recognizers.rs:104`) excludes the space. When a space-grouped
  card/IBAN is immediately followed by `@domain.tld`, the email match forms from **only the last 4-digit group** and
  *partially* overlaps (does not contain) the card/IBAN; `resolve_overlaps` (`src/pii/overlap.rs:25`, whole-lower-span
  drop) then discards the entire higher-value card/IBAN because `Email` (priority 6, `src/pii/mod.rs:82`) now outranks
  `Iban`/`CreditCard`, leaving the leading groups **in clear**. Reproduced independently (regex + overlap copy):
  `card 4111 1111 1111 1111@example.com` → masked `card 4111 1111 1111 [EMAIL_1]` (**12 Luhn-valid card digits leak**);
  `iban DE89 3704 0044 0532 0130 00@example.com` → masked `iban DE89 3704 0044 0532 0130 [EMAIL_1]` (**IBAN body leaks**).
  This is a **regression**: under the pre-M4-R7 order the card/IBAN won (masked whole → `[CARD_1]@example.com`,
  only the harmless `@domain` in clear). Continuous forms (`4111111111111111@…`), dash-grouped cards
  (`4111-1111-1111-1111@…`, `-` is in the local-part class), and `sk-…`/`AKIA…` secrets (no space) stay safe — the
  gap is exactly the space-grouped Card/IBAN. **Fix (recommended, keeps the M4-R7 benefit):** make the Email-over-
  structured win **containment-gated** — Email suppresses a Card/IBAN/Secret/NationalId only when its span *fully
  contains* the structured span (the `4111…@x.com` case); on a true *partial* overlap the checksum-backed structured
  span must win so no structured PII is left in clear. (This can't be expressed by the flat `priority()` scalar
  alone; it belongs in `resolve_overlaps`, e.g. a containment check before dropping a higher-value structured
  span.) **Minimal fail-safe alternative:** revert M4-R7 (put `Email` back below `Iban`/`CreditCard`/`Secret`) —
  leak-free, at the cosmetic cost of `@domain` appearing in clear next to a `[CARD_1]`/`[IBAN_1]`. **Tests:** add
  adversarial cases asserting that for a space-grouped card/IBAN immediately followed by `@domain`, the masked
  output contains **no** card/IBAN digit group in clear (e.g. `!masked.contains("4111 1111 1111")`,
  `!masked.contains("DE89 3704")`); keep `email_beats_a_card_iban_or_secret_local_part` green for the containment
  case. *(Related, lower severity: the earlier Email>NationalId reorder has the same shape for the space-grouped
  NINO `AB 12 34 56 C@x.com`; a containment-gated fix covers it too.)*

### M4-R9 fix review (2026-07-13) — original leak CLOSED, but **two NEW BLOCKERS** (the leak moved, it didn't go away)
Reviewed `5f31482` (fix) + `7d5668e` (docs). Independently verified: **103 tests green (default), 0 failed, no
warnings**; `cargo test --features onnx` → **111 passed + 1 `#[ignore]`d, no warnings** (both counts match the
builder's claim exactly). The **original M4-R9 leak is genuinely closed** — empirically reproduced through
`Vault::mask` (what the upstream actually receives): `card 4111 1111 1111 1111@example.com` → `card [CARD_1]@example.com`,
`iban DE89 3704 0044 0532 0130 00@example.com` → `iban [IBAN_1]@example.com`, `AB 12 34 56 C@x.com` → `[NATID_1]@x.com`.
No card/IBAN/NINO group survives in clear. The **non-vacuity claim is true** (verified in an isolated clone by
re-raising `Email` to the top structured priority: `email_partially_overlapping_a_structured_span_loses_it`,
`grouped_forms_attached_to_a_domain_do_not_leak` and `grouped_pii_glued_to_a_domain_leaks_nothing` all fail,
the last one reproducing the exact pre-fix output `card 4111 1111 1111 [EMAIL_1]`). Containment cases still hold
(`4111111111111111@x.com`, `123456789@x.com`, `sk-…@x.com` → one `[EMAIL_1]`); `universal_recognizers` is untouched
(the `recognizers.rs` diff is test-only); no raw PII in logs; fail-closed unchanged.

**But the fix did not eliminate the leak class — it swapped which side of a partial overlap leaks.** The root cause
is untouched: `resolve_overlaps` resolves an overlap by **dropping the whole loser span**, so the loser's bytes are
simply *abandoned in clear*. The flat `priority()` scalar can only express "one wins"; it cannot express "both must
be masked". M4-R7 made `Email` win and abandoned the card; M4-R9 makes the structured span win and abandons the
**email**. Two concrete blockers follow. Both reproduced end-to-end through `Vault::mask`.
- [ ] **M4-R10 (BLOCKER — NEW, introduced by the gate).** `drop_spans_contained_in_an_email`
  (`src/pii/overlap.rs:76`) drops a contained structured span **unconditionally, before the priority sort** — but
  the containing `Email` is **not guaranteed to survive** that sort. If a *third* span partially overlaps the email,
  the email loses on priority (it is now the lowest structured tier) and is dropped too — so the contained span,
  already deleted by the gate, **is masked by nothing at all and is forwarded in clear**. The gate's own doc comment
  ("an email is never itself contained in a structured span — so dropping the contained span can't strand a surviving
  email", `src/pii/overlap.rs:74`) checks the wrong direction: the danger is not a *stranded email*, it is a
  **stranded card**. Reproduced:
  `555 867 5309.4111111111111111@x.com` → masked `[PHONE_1].4111111111111111@x.com` (**16 Luhn-valid card digits in clear**);
  `555 867 5309.sk-abcdef123456@x.com` → masked `[PHONE_1].sk-abcdef123456@x.com` (**an API secret in clear** — and `Secret`
  is the *highest* priority kind, deleted by the gate before priority ever ran);
  `555 867 5309.123456789@x.com` → `[PHONE_1].123456789@x.com` (NIF in clear);
  `4111 1111 1111 1111.4111111111111111@example.com` → `[CARD_1].4111111111111111@example.com` (a **second full card** in clear).
  This directly violates the invariant both `src/pii/overlap.rs:41` ("Structured PII is never lost either way") and
  `docs/ARCHITECTURE.md:74` ("Fail-safe in both directions: PII is never left in clear") claim to hold.
- [ ] **M4-R11 (BLOCKER — NEW for `Phone` / `Ssn` / `NationalId`; pre-existing for `Card`/`Iban`/`Secret`).** Demoting
  `Email` to the lowest structured tier (`src/pii/mod.rs:98`) means a structured span that only **partially** overlaps a
  **real email** now wins and drops the *whole* email span — leaving the part of the email outside the structured span
  (its **local part**, i.e. a person's name/handle, plus the domain) **in clear**. The email local-part class includes
  `.` and `-`, which are also phone separators, so this is reachable whenever a space/`+`/`(` sits inside the structured
  span (that is what stops the email from simply *containing* it and being saved by the gate). Reproduced:
  `555 867 5309john.doe@example.com` → masked `[PHONE_1]john.doe@example.com` (**a complete, deliverable email address in clear**);
  `+39 333 1234567.mario.rossi@example.com` → `[PHONE_1].mario.rossi@example.com` (**name + domain in clear**);
  `AB 12 34 56 C.bob@x.com` → `[NATID_1].bob@x.com`; `card 4111 1111 1111 1111.bob@x.com` → `[CARD_1].bob@x.com`.
  Pre-M4-R7 `Email` (priority 3) outranked `Phone` (1) and `Ssn`/`NationalId` (2), so the phone/NatID shapes are a
  **regression** — and note the NINO case is a pure *trade*, not a fix: it used to leak the NINO, it now leaks the email.
  (A bare left-over `@domain.tld` is **not** PII and is acceptable — `[PHONE_1]@x.com`, `[NATID_1]@x.com` are fine. A
  left-over **local part** is.)
- **Fix (one change closes both).** Make the resolver's invariant **"no structured span's bytes are ever abandoned"**
  instead of "the higher priority wins". Concretely: when two **structured** spans *partially* overlap, **merge them
  into a single span covering the union** (labelled with the higher-priority kind) rather than dropping the loser.
  `Vault` masks and restores the union text verbatim, so the round-trip stays exact and nothing is left in clear —
  `555 867 5309john.doe@example.com` → one `[PHONE_1]`; `555 867 5309.4111111111111111@x.com` → one `[PHONE_1]` covering
  the card too. This subsumes the containment gate for the *partial* case and keeps it for true containment, is
  fail-safe in the project's stated direction (over-mask, never leak — cf. M4-R6), and costs nothing in practice since
  these shapes are pathological. Merging must iterate to a fixpoint (a merge can create a new overlap). Keep the
  existing whole-span drop for **NER** spans (M2-R7) — abandoning a `Person` remainder costs recall, never a leak.
  *(Alternative, if the union's over-masking is unwanted: keep the loser's non-overlapping remainder as its own masked
  span — `[PHONE_1][EMAIL_1]` — and make the gate conditional on the containing email actually surviving. More code,
  same guarantee.)*
- **Test (the one that would have caught all of this).** A **property/invariant test** over the resolver, not more
  point cases: for any input, **every byte of every structured candidate span produced by the recognizers must be
  covered by some span in the masked output**. Point cases alone keep missing this — each of the last three reviews
  found a new instance of the same class. Plus adversarial cases pinning the eight shapes above (assert on the masked
  body: `!masked.contains("john.doe")`, `!masked.contains("4111111111111111")`, `!masked.contains("sk-abcdef123456")`).
  Note `grouped_forms_attached_to_a_domain_do_not_leak` / `grouped_pii_glued_to_a_domain_leaks_nothing` will need their
  expected spans widened to the union (`4111 1111 1111 1111@example.com`), which is strictly *better* — the `@domain`
  stops leaking too.
- [ ] **M4-R12 (doc — stale mechanism stated as current).** Three places still describe the *superseded* priority order
  as the live one: `docs/TESTING.md` LOC-12 ("`Email` is the top structured priority" — it is now the **lowest**) and
  LOC-10 + `docs/ROADMAP.md` M5-Arabic note ("`Email` outranks the numeric national IDs" — it no longer does; the
  behavior now holds via the **containment gate**). The asserted *behavior* is unchanged in all three; only the stated
  mechanism is wrong. **Fixed in this review's doc commit** — listed here for traceability.

## M5 — Integration & performance testing
Goal: prove the whole system holds **end-to-end** and **under load**, then document it. Comes after M4 —
the feature set (structured + NER + streaming + multi-provider) is complete enough to test as a product.
- [ ] **Real integration tests.** End-to-end beyond today's mock-upstream e2e: full
  mask → forward → (stream) → de-mask round-trips, tool-call round-trips, multi-turn determinism, and
  the fail-closed paths. **Mock upstreams cover all three preset shapes** (OpenAI / Copilot / Anthropic)
  — no accounts needed. The **real-provider smoke is Anthropic-only for now** (the only provider we have;
  opt-in, needs a key, never in CI without one), via its OpenAI-compat endpoint.
  - [ ] Implement the two cataloged-but-missing e2e cases **E2E-02** (CSV `tool_result`) and **E2E-04**
    (`SELECT … FROM DUAL`) against a mock.
- [ ] **Manual "does the whole structure hold?" procedure (real Anthropic + max logging).** Run the same
  PII prompt twice with `RUST_LOG=llm_proxy_pii_rust=trace`:
  - **Run A** — `PII_DEBUG_SKIP_DEMASK=1`: the client gets the **placeholders** → proof the request left
    masked (Anthropic saw only `[…]`) and the round-trip is wired.
  - **Run B** — normal: the client gets the **restored real values** → proof the full round-trip. Comparing
    A vs B on the same input shows the whole chain holds end-to-end against a real provider.
  Trace-level logging also exercises the never-log-raw-PII rule (DBG-02) on **real** data.
- [ ] **Performance / load harness** (pulled up from Backlog) — concurrent connections, large bodies,
  streaming throughput; measure latency / RAM of the mask → forward → de-mask path (NER on/off).
  Stability under load was the founding motivation — measure it, don't assume it.
- [ ] **Update the root `README.md`** (+ `README.it.md`) to reflect the shipped product — what it does,
  the three-tier detection + NER, streaming, multi-provider (per-instance) usage, config/env, and
  status. The README is intentionally high-level today ("early development"); this is the pass that
  makes it describe a working system.
- [ ] **CI + release binaries (GitHub Actions).** CI on push/PR (`cargo test` + `fmt` + `clippy`; a job
  for the default build **and** one for `--features onnx`). A release workflow on a version tag
  **cross-compiles the full `--features onnx` product** (regex + NER — the complete tool) for
  Linux / macOS / Windows and attaches the binaries to GitHub Releases. **The first tagged release is
  `1.0.0`** (bump `Cargo.toml` from the current `0.4.0`) — cut when M5's integration + performance passes
  are green and the README reflects reality. *(The regex/NER balance shifts again if the GLiNER evolution
  lands — see Backlog.)*

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

### Locale phone national formats  *(moved out of M4 — 2026-07-13; the FP-prone tier)*
National phone numbers without a `+CC` anchor (e.g. UK `020 …`, DE `030 …`) collide with ordinary number
sequences, so they're false-positive-prone. The `fp_prone_recognizers(code)` seam is wired and empty,
gated by `PII_LOCALES`. The universal `+CC` international arm already catches the unambiguous case; add a
specific country's national format here only when a concrete need justifies the precision work. **The clean
way to catch the ambiguous form is *context*, not regex** — the current XLM-R NER only does PER/ORG/LOC
(no phone), so this really belongs to the **GLiNER escalation** below (open-label, context-based).

### Evaluate GLiNER — contextual-PII detector / potential XLM-R successor  *(2026-07-13)*
The path for **ambiguous, anchor-less PII** (a bare national phone, a free-form postal address) that the
deterministic layer can't disambiguate and the current XLM-R (PER/ORG/LOC only) doesn't cover. **GLiNER ≠
Piiranha:** Piiranha (a *fixed-label* mDeBERTa token-classifier) was evaluated at M2 and **rejected** (~0
recall on natural sentences); GLiNER is a *zero-shot, **open-label*** span extractor — you pass labels like
`"phone number"` at inference and it matches by context — and is **not yet evaluated**. It could be a single
more-capable **successor to XLM-R** (named entities *and* contextual PII in one model). Needs a **measured
eval** (score `gliner_multi_pii-v1` int8 through the hybrid on the corpus; CPU latency / RAM vs the lean bar)
and a **separate detector + span decode** (it is *not* token-classification, so not a drop-in). Full detail
in [`docs/M2-NER-EVALUATION.md`](M2-NER-EVALUATION.md) (Escalation path). **Why it's a Backlog item and
not done:** GLiNER was set aside for *first-version scaffolding simplicity* — a separate detector was more
integration than a first pass warranted — **not** for any capability doubt. That is the **opposite** of
Piiranha, which is a *measured* dead-end on prose. **When this is picked up, the evolution to evaluate is
GLiNER, not Piiranha.**

### Other later items
Auth & rate-limiting stages, **TLS / running behind a TLS terminator**, config-file
support & container deployment, additional providers, metrics/observability.

*(The **never-log-raw-PII** rule is **not** a backlog item — it's an enforced quality bar today:
kind/placeholder-only logging, guarded by the `log_safety.rs` regression test (DBG-02). A dedicated
structured **audit-trail** feature could be re-added here only if a compliance need arises.)*

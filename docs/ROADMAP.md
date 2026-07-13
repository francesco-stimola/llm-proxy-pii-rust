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
  substring of an email local part merges **into** the email — their union is exactly the email span, so a
  numeric email local part is never fragmented; `overlap::name_of` keeps the `Email` label. This was a
  `Email`>national-ID **priority** rule (pre-M4-R9), then the M4-R9 containment gate; since M4-R15 both are gone
  — the union-merge produces the span and the naming rule carries the label.)*
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
- [x] **M4-R10 (DONE 2026-07-13 — closed by the union-merge; see the shared fix note below).** `drop_spans_contained_in_an_email`
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
- [x] **M4-R11 (DONE 2026-07-13 — closed by the union-merge; see the shared fix note below).** Demoting
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

#### M4-R10 + M4-R11 — RESOLVED 2026-07-13 (`resolve_overlaps` rewritten around the invariant)
Implemented exactly the recommended fix. The resolver's governing rule is now an **invariant, not a ranking**:
**"no structured span's bytes are ever abandoned."** Three phases (`src/pii/overlap.rs`):
1. **Email containment gate** (kept, and now *provably* safe): a structured span entirely inside an `Email`
   span is a false decomposition of its local part → dropped, email keeps the label. It can no longer strand
   anything, because an email is never *dropped* — only absorbed into a union covering at least its own span.
   This is the coherence M4-R10 demanded between the gate and the new invariant.
2. **Structured union-merge**: overlapping structured spans collapse into their **union**, labelled by the
   highest-priority kind (ties → longest span → incumbent). A single sort-by-start sweep reaches the
   **fixpoint** (a merge can only extend the union rightwards, so it never creates an earlier overlap).
   `resolve_overlaps` now takes `input` so the union's `text` is re-sliced from the source — the `Vault` keys
   on `entity.text` and splices by `span`, so the merged union masks and restores **verbatim** (round-trip
   stays exact). A union wider than its winning candidate is honestly re-tagged `Confidence::Structural`.
3. **NER keeps the whole-span drop** (M2-R7) — a lost `Person` remainder costs recall, never a leak.

`PiiKind::priority` now ranks **labels, not survivors**: a lower priority can no longer cost coverage, it only
decides which kind *names* a union. (It still picks the survivor for NER.)

**Tests.** **PROP-03 `every_structured_candidate_byte_is_covered`** — the property test that encodes the
invariant directly: glue PII values (incl. the grouped shapes) in arbitrary orders/separators, then assert
**every raw structured candidate is fully covered by some resolved span**, and that the mask→demask round-trip
is still exact. Plus the reviewer's exact repros as deterministic cases:
`a_span_deleted_by_the_containment_gate_is_never_stranded` + `a_partially_overlapping_email_is_never_abandoned`
(recognizers), `partially_overlapping_pii_abandons_no_bytes` + `masking_partially_overlapping_pii_still_round_trips`
(`tests/adversarial.rs`, asserted on the **masked body**), and 3 new resolver units (union, fixpoint chain, the
gate-stranding case). **Verified non-vacuous:** with the union-extension disabled, all 7 fail — and PROP-03
*independently rediscovered* the M4-R11 leak, shrinking to `555 867 5309john.doe@example.com` ("Email at 8..32
is left in clear"). A per-byte invariant cannot be satisfied by picking a winner, which is precisely why the two
earlier priority-only fixes (M4-R7, M4-R9) each passed their own case while leaking the other side.
**110 tests green (default) / 118 + 1 `#[ignore]`d (`--features onnx`), no warnings.**
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
- [x] **M4-R12 (doc — stale mechanism stated as current) — DONE.** Three places described the *superseded* priority
  order as the live one: `docs/TESTING.md` LOC-12 ("`Email` is the top structured priority" — it is now the
  **lowest**) and LOC-10 + `docs/ROADMAP.md` M5-Arabic note ("`Email` outranks the numeric national IDs" — it no
  longer does; the behavior now holds via the **containment gate**). The asserted *behavior* was unchanged in all
  three; only the stated mechanism was wrong. Fixed in that review's doc commit — verified corrected.

### M4-R10/R11 union-merge review (2026-07-13) — the overlap leak class is genuinely CLOSED; one **NEW BLOCKER** elsewhere
Reviewed `0ed0c7c` (fix) + `2985022` (docs) — the **fourth** review of this overlap code. Independently verified:
**110 tests green (default) / 118 + 1 `#[ignore]`d (`--features onnx`), 0 failed**; a **from-scratch rebuild**
(`cargo clean -p` + `cargo check --all-targets`, both feature sets) emits **no warnings**. Both counts match the
builder's claim exactly.

**The invariant holds — I tried hard to break it and could not.**
- **All seven previously-leaking inputs are clean**, reproduced through `Vault::mask` (the exact bytes the upstream
  receives): `card 4111 1111 1111 1111@example.com` → `card [CARD_1]`; `iban DE89 3704 0044 0532 0130 00@example.com`
  → `iban [IBAN_1]`; `AB 12 34 56 C@x.com` → `[NATID_1]`; `555 867 5309.4111111111111111@x.com` → `[PHONE_1]`;
  `555 867 5309.sk-abcdef123456@x.com` → `[PHONE_1]`; `555 867 5309john.doe@example.com` → `[PHONE_1]`;
  `+39 333 1234567.mario.rossi@example.com` → `[PHONE_1]`. No card digit, IBAN group, NINO group, secret, or email
  local part survives in clear, and every one round-trips exactly.
- **Fixpoint claim is correct.** `merge_structured` is textbook merge-intervals: sorted by start (ties → longest
  first), a candidate can only extend the running union rightwards, and a new group is opened only when
  `candidate.start >= union.end` — which, since starts are non-decreasing, means no later candidate can reach back
  into a closed group. Chains (A∩B, B∩C, A∩C = ∅) merge transitively; a fully-contained span is absorbed. One pass
  genuinely reaches the fixpoint.
- **No structured byte can be abandoned.** Enumerated every discard site: (1) the **containment gate** only ever
  drops a span contained in an `Email`, and an `Email` is *never* dropped (the gate exempts it; the merge only ever
  absorbs it into a union ⊇ its own span) — so the dropped span's bytes stay covered; (2) the **merge sweep**
  drops nothing, it unions; (3) the **NER drop** only touches non-structured kinds — and `ner_decode::label_to_kind`
  can *only* return `Person`/`Organization`/`Location`, so no NER span can ever enter the structured partition;
  structured spans are all in `kept` **before** NER is considered, so dropping an NER span whole cannot strand one.
  (4) no other dedup/filter exists.
- **Fuzzed the resolver itself**, not just the regexes: **500,000** randomized synthetic candidate sets (1–6 spans,
  arbitrary kinds incl. NER, arbitrary ranges, duplicates, containment, chains) straight into `resolve_overlaps` →
  **zero violations** of: output spans pairwise non-overlapping · every structured candidate fully covered by **one**
  output span · `entity.text == input[span]` · `demask(mask(x)) == x`.
- **Union correctness / memory safety.** Union endpoints are always existing span endpoints, and structured spans
  come only from `regex` matches over the same `input` — always on char boundaries — so the re-slice is safe and
  `text == input[span]` exactly (`Vault` keys on `text` and splices by `span`, so this is load-bearing). The code
  uses `input.get(..)`, not `&input[..]`, so it cannot panic. Verified on multi-byte inputs (CJK / accented / emoji
  neighbours): no panic, exact round-trip, nothing uncovered. PROP-01 and VAULT-02 still pass.
- **Over-masking is provably bounded — no cap needed.** A union is the union of *candidate* intervals, so it can
  only ever cover bytes some recognizer already claimed as PII-shaped: it can **never** swallow ordinary prose
  (300k randomized trials with ordinary sentences interleaved: **0** non-candidate bytes ever absorbed). Widest
  union found in 300k trials of glued real PII: **148 bytes / 8 candidates**, and every byte of it was PII-shaped.
  Accept as-is.
- **PROP-03 is a genuine, non-vacuous invariant.** It asserts each *raw* candidate is fully contained in **one**
  resolved span — a per-byte coverage claim, not "the value is absent". Non-vacuity confirmed independently: a
  reference implementation of the old drop-the-loser resolver leaves 3 uncovered candidates on the three canonical
  inputs, reproducing the historical outputs (`[PHONE_1]john.doe@example.com`, `card [CARD_1]@example.com`). The
  checked-in seed `picks = [1, 0], glues = [0, 0, 0]` decodes to `"555 867 5309" + "" + "john.doe@example.com"` =
  **exactly the M4-R11 leak** — a real case, now green.
- **(E) all clear.** No raw PII in logs (the widened `Structural` tagging only adds kind-only `debug!` lines);
  fail-closed unchanged; `universal_recognizers` semantics unchanged; corpus / adversarial / proptest / containment
  guards all pass. Docs (ROADMAP / ARCHITECTURE / TESTING / DEVLOG) match reality.

**But the review found a leak of a *different* class, outside the overlap code — pre-existing, not a regression:**
- [x] **M4-R13 (DONE 2026-07-13 — ASCII boundaries `(?-u:\b)` + the corpus blind spot closed).** All 12 anchored
  recognizers switched to `(?-u:\b)`. `Email`/`Phone` needed no change (anchored by character classes, not `\b`).
  The anti-FP guarantee is preserved **exactly** — an ID still can't fire inside a longer *ASCII* token
  (`card4111111111111111`, `abc4111…abc`, `PO123456A`) — only a non-ASCII **letter** stops counting as part of a
  number. **The root enabler was the corpus:** `tests/corpus/pii_cases.json` had **zero** non-ASCII characters, so
  a total detection failure in CJK survived four reviews. Added a `non_ascii_scripts` category (CJK-01..05, JA-01,
  CYR-01 positives + 2 ASCII-token negatives) and non-ASCII round-trips (RT-05/RT-06) — the category is picked up
  automatically by the data-driven `pii_corpus.rs`. Plus adversarial cases asserting on the **masked body**
  (`structured_pii_is_detected_in_cjk_and_cyrillic_prose`,
  `ascii_token_anti_false_positive_guarantee_survives_the_ascii_boundary`) and a unit
  (`cjk_prose_does_not_hide_structured_pii`). **Verified non-vacuous:** restoring the Unicode `\b` makes the
  detector return `[]` on `我的信用卡号是4111111111111111`, and corpus CJK-01 + round-trip RT-05 fail with
  "PII must be masked" — i.e. the card really was going upstream in clear. *(Original finding below.)*
  **M4-R13 (BLOCKER — LEAK — Unicode word boundary makes the structured recognizers inert in CJK text).**
  Every `\b`-anchored structured recognizer (`src/pii/recognizers.rs`: Secret `:78`, Iban `:90`, CreditCard `:99`,
  Ssn `:133`, IT CF `:141`, GB NINO `:150`, ES DNI `:159`, FR NIR `:169`, 9-digit `:182`, 11-digit `:192`, LV `:198`,
  **zh Resident ID `:205`**) uses Rust `regex`'s **Unicode-aware `\b`**, in which a Han / Kana / Cyrillic / Greek /
  Arabic letter *is* a word character. There is therefore **no word boundary between a CJK character and a digit** —
  and Chinese/Japanese have **no inter-word spaces**, so the glued form is the *natural* one, not an evasion.
  Reproduced through `Vault::mask` (nothing detected, forwarded verbatim):
  `我的信用卡号是4111111111111111` → **16 Luhn-valid card digits in clear**;
  `密钥sk-abcdef123456` → **an API secret in clear**;
  `我的身份证号是11010519491231002X` → the M4 zh Resident ID recognizer **never fires**;
  `账号DE89370400440532013000` → IBAN in clear; `编号123-45-6789` → SSN in clear;
  `カード番号は4111111111111111です` → card in clear; `Карта4111111111111111` → card in clear.
  The same values **are** masked the moment a space is inserted, which pins the cause precisely. This is squarely
  inside the **declared M4 domain**: `zh` is one of the ten declared languages, ARCHITECTURE states "structured PII
  is language-independent", and M4 shipped a zh Resident ID pack **that is inert in real Chinese prose**. It
  survived four reviews because `tests/corpus/pii_cases.json` contains **zero** non-ASCII characters and the M4
  "validate across the declared domain" pass validated the **NER** on zh — never the structured recognizers.
  **Fix:** switch the anchors to **ASCII** word boundaries — `(?-u:\b)` — on every structured recognizer.
  *Verified independently:* `(?-u:\b)…(?-u:\b)` detects `我的信用卡号是4111111111111111` and `Карта4111111111111111`
  while leaving `card4111111111111111`, `abc4111111111111111abc` and `hash a4111111111111111b` **unmatched** — i.e.
  it preserves the deliberate M4 anti-FP guarantee ("a CF/NINO can't fire inside a longer alphanumeric token —
  API key / hash / UUID / base64") exactly, and only stops treating a *non-ASCII letter* as part of a number.
  **Tests:** adversarial cases per recognizer with a CJK/Cyrillic prefix and suffix, asserting on the **masked body**
  that no card digits / IBAN / secret / national ID survive in clear (`!masked.contains("4111111111111111")`,
  `!masked.contains("sk-abcdef123456")`, `!masked.contains("11010519491231002X")`); plus CJK structured cases added
  to `tests/corpus/pii_cases.json`, which today has none.
- [x] **M4-R14 (DONE 2026-07-13 — the merge fallback now fails *safe*).** `materialize` (was the tail of
  `merge_structured`) no longer falls back to "the winning candidate alone". It first **widens the union out to the
  enclosing `char` boundaries** (clamped to the input) — widening can only *add* bytes, so it can never abandon a
  constituent — which makes the re-slice total. The remaining (now genuinely unreachable) `None` arm returns the
  **whole group unmerged** rather than one span that drops constituents, guarded by a `debug_assert!` and a
  kind-only `tracing::warn!`. No panic: this is a proxy and the input is attacker-influenced. **Test:**
  `a_union_ending_inside_a_multibyte_char_still_covers_every_constituent` — two overlapping structured spans whose
  union endpoint falls *inside* a 3-byte `€`; the span widens to the char boundary, the text is re-sliced from the
  input, and both constituents stay covered. *(Original finding below.)*
  **M4-R14 (low — hardening: the one fallback in the merge degrades *toward* the leak).**
  `merge_structured`'s re-slice (`src/pii/overlap.rs:141-155`) falls back to `None => winner` when
  `input.get(union)` fails — i.e. it silently returns the **winning candidate's span alone, abandoning every other
  constituent's bytes**, which is precisely the leak class this commit closed. It is **unreachable today** (all
  structured spans come from `regex` matches over the same `input`, hence always on char boundaries; `label_to_kind`
  can never yield a structured kind, so no model-derived span can enter the structured partition) — so this is not
  a live leak. But it is a booby trap in the exact code path the invariant rests on, and the **Backlog GLiNER**
  detector would emit spans from a *model*, not a regex. **Fix:** make the fallback fail *safe* — widen the union
  out to the enclosing char boundaries and re-slice, or return the constituents unmerged; never a single span that
  drops constituents. Add a `debug_assert!` and a kind-only `tracing::warn!`. **Test:** a resolver unit feeding two
  overlapping structured spans whose union endpoint falls inside a multi-byte character, asserting both spans'
  bytes are still covered.
- [x] **M4-R15 (DONE 2026-07-13 — the union is named by the highest-priority *raw* candidate; the containment gate
  became a *naming* rule and the deletion is gone).** Implemented as asked, and it let the **containment gate be
  deleted outright** — a strict simplification. Key observation: the gate never affected any *span*. A span enclosed
  by an email, if not deleted, simply **merges into that email**, giving the identical union. The gate only ever
  affected the **label**. So `drop_spans_contained_in_an_email` is removed and enclosure is now expressed in
  `name_of`: the union is named by the highest-priority **raw** candidate it covers (so the enclosed `Secret` names
  it — R15's ask), **except** when the union is *exactly* an `Email` span, in which case the group is a genuine
  email whose local part merely looks like a card/ID (`4111111111111111@x.com`) and the email keeps the label
  (preserving the M4-R7/R9 behaviour every earlier review ratified — a literal "always the highest-priority raw
  candidate" would have relabelled that case `[CARD_1]`). This also **structurally removes the M4-R10 trap**: with
  nothing deleted, no span can be stranded. **Tests:** `an_enclosed_secret_names_the_union_not_the_phone` +
  `a_span_enclosed_by_an_email_is_still_covered_and_names_the_union` (resolver) and the end-to-end assertion in
  `a_span_deleted_by_the_containment_gate_is_never_stranded` — `555 867 5309.sk-abcdef123456@x.com` now resolves to
  a single span labelled **`Secret`**. *(Original finding below.)*
  **M4-R15 (nit — utility/observability: a union is named by the *surviving* candidates, so a `Secret` can be
  masked as `[PHONE_1]`).** In `555 867 5309.sk-abcdef123456@x.com` the containment gate deletes the `Secret`
  (it sits inside the email's local part) *before* priority is consulted, so the union is labelled by the phone —
  `[PHONE_1]`. **No leak** (the secret is masked), but the model is told `[PHONE_1]` stands for a phone when it
  actually stands for a phone + secret + email blob, and the kind-only audit `debug!` under-reports a `Secret` as a
  `Phone`. **Fix:** name the union by the highest-priority **raw** candidate whose span it covers (including ones
  the gate dropped), not by the surviving set. **Test:** the above input resolves to a single span labelled
  `Secret`.

### M4-R13/R14/R15 review (2026-07-13) — the CJK leak is CLOSED, with **no FP regression**; no blockers
The **fifth** review of this area. Reviewed `7388929` (fix) + `3765f71` (docs). Independently verified:
**115 tests green (default) / 123 + 1 `#[ignore]`d (`--features onnx`), 0 failed**; a from-scratch
`cargo check --all-targets` on **both** feature sets emits **no warnings**. Both counts match the builder's
claim exactly.

**(A) M4-R13 — the CJK leak is genuinely closed, and the anti-FP guarantee survives.**
- All seven inputs now mask, reproduced through `Vault::mask` (the exact bytes the upstream receives), each
  round-tripping exactly: `我的信用卡号是4111111111111111` → `我的信用卡号是[CARD_1]`; `密钥sk-abcdef123456` →
  `密钥[SECRET_1]`; `我的身份证号是11010519491231002X` → `我的身份证号是[NATID_1]`; `账号DE89370400440532013000`
  → `账号[IBAN_1]`; `编号123-45-6789` → `编号[SSN_1]`; `カード番号は4111111111111111です` →
  `カード番号は[CARD_1]です`; `Карта4111111111111111` → `Карта[CARD_1]`.
- **All 12 anchored recognizers converted** — verified by grep: no bare `\b` survives in any `Regex::new` in
  `src/`. `Email` (`recognizers.rs:126`) and `Phone` (`:136`) genuinely carry no `\b` (character classes only).
  The `recognizers.rs` diff is *exactly* the 12 boundary swaps + tests + comments: **zero** change to any
  validator or to the recognizer set.
- **Anti-FP guarantee intact:** `card4111111111111111`, `abc4111111111111111abc`, `hash a4111…b`, a SHA-256 hex
  hash, a base64 blob, a UUID and `PO123456A` all still match **nothing**.
- **Non-vacuity confirmed independently** (isolated clone, `(?-u:\b)` → `\b`): the detector returns `[]` on all
  seven inputs, and exactly the claimed tests fail — `cjk_prose_does_not_hide_structured_pii`,
  `structured_pii_is_detected_in_cjk_and_cyrillic_prose`, corpus `[non_ascii_scripts/CJK-01]` and
  `[RT-05] PII must be masked`. The fix is load-bearing.
- **The new FP surface is real but is NOT a precision regression — quantified.** With ASCII boundaries any
  non-ASCII char is a boundary, so a token glued to an *accented Latin* letter can now match
  (`café4111111111111111` → `[CARD_1]`; `Nº123456789` → `[NATID_1]`). It affects the 10 anchored
  ID/card/IBAN/secret recognizers, and it requires **zero separator** between a non-ASCII **letter** and the
  token. Measured over 100k random 9-digit numbers: `nº123456789` (glued — the new surface) masks **18.10%**,
  and `nº 123456789` (separated) masks **18.10%** — *identical*, and the separated form **already matched before
  the fix**. So the boundary change adds *positions*, and in every one of them the FP rate is governed entirely
  by the checksum — i.e. it is the **already-accepted M4-R6 over-mask** (~18% of arbitrary 9-digit tokens),
  reaching a few more places. Nine realistic FR/DE/ES/PT sentences with numbers (`Le café a été payé 12 euros…`,
  `Herr Müller wohnt in der Bahnhofstraße 12…`, `El señor Núñez… desde 1998…`, `Der Preis beträgt 1234567890
  Cent.`) produce **zero** false positives — prose separates numbers from words. Words whose accent is *not*
  final (`Müller123456789`, `Straße123456789`, `señor12345678Z`) are unaffected (the adjacent char is ASCII).
  Verdict: **acceptable over-mask**, squarely inside the project's stated direction; no fix needed. *(The one
  idiomatic zero-separator form, ES/PT `nº`/`Nº` + digits, carries exactly the M4-R6 rate it already had.)*

**(B) The gate removal is sound — containment is preserved and the invariant still holds after the rewrite.**
- **Containment without the gate:** `4111111111111111@x.com`, `123456789@x.com`, `sk-abcdef@x.com` and
  `555.867.5309@x.com` each mask as **one `[EMAIL_1]`** covering the whole email — nothing fragmented, no
  `@domain` in clear, exact round-trip. The union-merge subsumes the gate exactly as claimed: an enclosed span
  merges *into* its email, so the union **is** the email span, and `name_of`'s `union == Email span` exception
  keeps the label. Deleting nothing structurally removes the M4-R10 stranding trap.
- **The whole historical leak set is still clean:** `card 4111 1111 1111 1111@example.com` → `card [CARD_1]`;
  `AB 12 34 56 C@x.com` → `[NATID_1]`; `555 867 5309.4111111111111111@x.com` → `[CARD_1]`;
  `555 867 5309.sk-abcdef123456@x.com` → `[SECRET_1]`; `555 867 5309john.doe@example.com` → `[PHONE_1]`.
- **Re-fuzzed the invariant after the +253-line rewrite — I could not break it.** **400,000** randomized
  synthetic candidate sets (1–6 spans, all 10 kinds incl. NER, duplicates, full containment, chains, 3+ mutual
  overlaps, multi-byte inputs) straight into `resolve_overlaps` → **zero** violations of: output spans pairwise
  non-overlapping · every structured candidate fully covered by **exactly one** output span · `text ==
  input[span]` · exact round-trip. Plus **200,000** end-to-end cases gluing real PII with **non-ASCII** prose
  fragments (CJK / Cyrillic / accented) → same four invariants hold; widest union 99 bytes. The new overlap
  surface M4-R13 opens (recognizers now firing inside CJK, e.g. a card and a zh Resident ID adjacent with no
  spaces) is covered.

**(C) M4-R15 union naming — an improvement, not a regression.** The feared relabelling does **not** happen:
`4111111111111111@x.com` still masks as `[EMAIL_1]`, not `[CARD_1]` — `name_of`'s exception (union *exactly*
an `Email` span ⇒ the email keeps the label) preserves the M4-R7/R9 behaviour every earlier review ratified.
The relabellings that *do* occur are strictly better for both the augmentation prompt and the audit log:
`555 867 5309.sk-abcdef123456@x.com` → `[SECRET_1]` (was `[PHONE_1]`) and `555 867 5309.4111111111111111@x.com`
→ `[CARD_1]` (was `[PHONE_1]`) — the model and the kind-only `debug!` now see the **most sensitive** kind in the
blob. *(Residual, not worth changing: a genuine email whose local part really is a secret — `sk-…@x.com` — is
labelled `Email`, so the audit under-reports it. Masked either way; consistent with M4-R7/R9.)*

**(D) M4-R14 verified.** `widen_to_char_boundaries` (`overlap.rs:202`) is **total** — it clamps to `input.len()`
and walks outward to real char boundaries, so `input.get(span)` in `materialize` always succeeds and the `None`
arm is genuinely unreachable; widening can only *add* bytes, so it cannot abandon a constituent. The fallback
now returns the group **unmerged** (coverage preserved) rather than the winner alone.

**(E) All clear.** The only new log site is `tracing::warn!(?kind, …)` — kind only, never a value; `log_safety`
(DBG-02) passes. `server.rs` / `privacy.rs` / `config.rs` are untouched, so fail-closed is unchanged. Corpus /
adversarial / proptest / PROP-01 / PROP-02 / VAULT-02 and the containment guards all pass. The
`non_ascii_scripts` corpus category (CJK-01…05, JA-01, CYR-01 + 2 ASCII-token negatives) and RT-05/RT-06 are
real and are picked up by the data-driven `pii_corpus.rs` (they fail when the fix is reverted).

Three **non-blocking** follow-ups (none is a leak; the first is a missing *guard*, the second is pre-existing):
- [x] **M4-R16 (DONE 2026-07-13 — PROP-03/PROP-04 now run on multi-byte input).** Added multi-byte entries to the
  property tests' `GLUE` (`我的信用卡号是`, `です`, `、`, `，`, `Карта`, `café`, `—`) and two samples already embedded
  in non-ASCII context (`我的身份证号是11010519491231002X`, `カード番号は4111111111111111です`), so the
  no-abandoned-bytes invariant is exercised on multi-byte input — including the paths the M4-R14 rewrite added
  (`widen_to_char_boundaries`, the union re-slice). The tables carry a standing comment saying why non-ASCII glue
  is mandatory here. *(Original finding below.)*
  **M4-R16 (medium — the ASCII blind spot is *not* fully closed: PROP-03 itself is still ASCII-only).**
  The corpus grew a `non_ascii_scripts` category, but **PROP-03** — the property test that encodes the resolver's
  core invariant (`every_structured_candidate_byte_is_covered`, `src/pii/recognizers.rs:1055-1120`) — still has
  **zero non-ASCII characters** in its `PII_SAMPLES` and `GLUE` tables (verified by counting: 0). That is the
  *exact* blind spot that let M4-R13 survive four reviews, now sitting in the one test the whole
  no-abandoned-bytes guarantee rests on — and `docs/TESTING.md` even states "**Any new recognizer must be
  exercised in a non-ASCII context here**" while the invariant test is not. Not a live leak (my own 200k-case
  fuzz with CJK/Cyrillic/accented glue found no violation), but the **guard is missing**: nothing in the suite
  would catch a future regression in the multi-byte paths the rewrite added (`widen_to_char_boundaries`, the
  union re-slice, a union endpoint landing inside a multi-byte char). **Fix:** add non-ASCII entries to PROP-03's
  `GLUE` (e.g. `"我的信用卡号是"`, `"です"`, `"Карта"`, `"café"`, `"，"`) and at least one non-ASCII-context
  sample, so the invariant is exercised on multi-byte input. **Test:** PROP-03 itself, with the widened tables
  (it must stay green; verify non-vacuity by breaking `widen_to_char_boundaries`).
- [x] **M4-R17 (DONE 2026-07-13 — fixed at the candidate-generation layer, *and* PROP-04 immediately found a
  second, live leak of the same class).** Two changes:
  1. **Overlapping candidate scan.** `Recognizer::push_candidates` replaces `find_iter` and resumes one `char`
     past each match's **start** (not its end), so a value that begins *inside* an earlier match of the same
     recognizer is emitted. Overlapping hits of one recognizer are **coalesced into maximal runs** as they are
     found, so a pathological input (a long row of 4-digit groups → a match at every boundary) can't blow up the
     candidate set; the resolver needs the *coverage*, and would union those spans anyway. Each hit is still
     validated (Luhn/checksum) **before** joining a run. The reviewer's suggested "re-scan the uncovered gaps"
     fix would **not** have closed the reported repro — the gap is ` 1111`, and the hidden card *starts inside the
     covered region* (offset 32, within the 27..46 match), so no amount of gap re-scanning surfaces it. The leak
     is in candidate generation, exactly as the finding says, so that is where it is fixed.
  2. **`Vault::mask_all` — mask to a fixpoint.** PROP-04 (added as asked) failed on its **first run** with a
     genuine, previously-unknown leak: `4111111111111111555 867 5309` → `4111111111111111[PHONE_1]`. The card and
     phone form **one 19-digit run** that is not Luhn-valid, so the card is correctly *not* a candidate (an ID
     never fires inside a longer token) — but masking the phone **splits the run and exposes** a clean, Luhn-valid
     card, which then goes upstream in clear. **Masking rewrites the bytes around what it replaced, and a value is
     only recognizable in context.** So masking now re-detects until the text yields nothing (`mask_all`, wired
     into `PrivacyStage`). It converges because a placeholder is inert (no recognizer can match `[KIND_N]` or span
     across it), with `MAX_MASK_PASSES` as a belt-and-braces bound; the round-trip stays exact (each pass records
     raw → placeholder; `demask` restores all in one tolerant pass).
  **Tests:** `a_value_hidden_behind_an_earlier_match_is_still_a_candidate` + `masking_a_phone_must_not_expose_a_card`
  (recognizers), `a_value_hidden_behind_an_earlier_match_is_not_left_in_clear` (`tests/adversarial.rs`, on the
  masked body), and **PROP-04 `masking_leaves_nothing_detectable`** — re-running the detector on the masked output
  must find nothing. PROP-04 is the right companion to PROP-03: PROP-03 quantifies over the *candidate set* (so a
  value the recognizers never emit satisfies it **vacuously**), PROP-04 quantifies over the **output bytes**, which
  no candidate-generation gap can hide. **Verified non-vacuous:** restoring `find_iter` semantics drops the real
  trailing card from the candidate set (only the shifted `6789-4111 1111 1111` window survives) and the repro
  fails. *(Original finding below.)*
  **M4-R17 (low-medium — PRE-EXISTING, not a regression: `find_iter` can *hide* a real value from the
  candidate set, so the invariant never sees it).** `raw_candidates` (`src/pii/recognizers.rs:258-277`) drives
  each recognizer with `regex::find_iter`, which is **leftmost and non-overlapping per recognizer**. A real PII
  value that *starts inside* an earlier match of the **same** recognizer is therefore **never emitted as a
  candidate at all** — and PROP-03's invariant ("every raw candidate is covered") is then *vacuously* satisfied
  for it. The invariant is only ever as strong as the candidate set. Reproduced through `Vault::mask`:
  `4111 1111 1111 1111@123-45-6789-4111 1111 1111 1111` → masked **`[CARD_1]@[CARD_2] 1111`** — the *shifted*
  16-digit window `6789-4111 1111 1111` happens to be Luhn-valid and is matched first, so the real trailing card
  is never a candidate and its last group **` 1111` is forwarded in clear**. **Verified pre-existing:** it
  reproduces byte-identically at `3e1aa07` (pre-fix), so it is *not* caused by these commits — it is a limit of
  the **candidate-generation** layer, not of the resolver (the resolver's own invariant holds; see the fuzz
  above). Severity is bounded: the input is pathological, and only a trailing 4-digit group escapes. *(A second
  instance of the same class leaves only a bare `@domain.tld`, which the project explicitly accepts as not-PII —
  M4-R11.)* **Fix (suggested):** after resolution, re-run the recognizers over the **uncovered gaps** to a
  fixpoint (gaps are short, so this is cheap), so a value hidden behind an earlier non-overlapping match is still
  caught. **Test:** the input above must leave no `1111` group in clear; plus a strong, cheap invariant that
  would catch this whole class — **re-running the detector on the masked output must yield no structured
  candidates** (add as a proptest sibling of PROP-03).
- [x] **M4-R18 (DONE 2026-07-13).** All five live comments updated to the naming-rule mechanism (`src/pii/mod.rs`
  ×2, `src/pii/overlap.rs`, `src/pii/recognizers.rs`, `tests/adversarial.rs`) and both tests renamed:
  `a_span_deleted_by_the_containment_gate_is_never_stranded` → `a_span_enclosed_by_an_email_is_never_stranded`,
  `email_containing_a_structured_span_wins_it` → `email_enclosing_a_structured_span_names_the_union`. Worth doing
  precisely because describing a deletion that no longer exists is the mis-reasoning that produced M4-R10.
  *(Original finding below.)*
  **M4-R18 (low — stale references to the REMOVED containment gate, in `src/` + `tests/`).** The gate is
  gone, but five live comments still describe it as the current mechanism — which is precisely the mis-reasoning
  that produced M4-R10, so it is worth keeping honest: `src/pii/mod.rs:79` ("handled *ahead* of priority by the
  containment gate") and `:102` ("Used by the containment gate in `overlap::resolve_overlaps`");
  `src/pii/overlap.rs:259` ("// Containment gate: …"); `src/pii/recognizers.rs:864` ("the containment gate
  deletes the card") **and the test name `a_span_deleted_by_the_containment_gate_is_never_stranded`**;
  `tests/adversarial.rs:133` (same). **Fix:** update the five comments to the naming-rule mechanism and rename
  the test (e.g. `a_span_enclosed_by_an_email_is_never_stranded`). *(The four **doc** occurrences —
  `ARCHITECTURE.md` module table and `TESTING.md` LOC-10 / LOC-12 / LOC-15 / LOC-16 — were fixed by the reviewer
  in this review's doc commit; LOC-15 also catalogued `email_partially_overlapping_a_structured_span_loses_it`,
  a test deleted back at `0ed0c7c`.)* **Test:** none needed (comments).

### M4-R16/R17/R18 fix review (2026-07-13) — correctness SOUND, but a DoS regression + a latent fail-open
The M4-R17 fix is real and **non-vacuous** (proven by source mutation): reverting `find_at`+`start()+1` to
`find_iter` semantics reproduces the exact `[CARD_1]@[CARD_2] 1111` leak, and the fixpoint alone does not save it
(the leftover ` 1111` is not independently detectable) — so **both** mechanisms are load-bearing. Round-trips are
exact; **NER does not corrupt them** (H1 falsified: XLM-R int8, run live, never labels or spans a `[KIND_N]`
placeholder, in bare form or in IT/EN/FR/DE/ES/CJK prose → no nested placeholders). No FP regression; `Confidence`
downgrade on a coalesced run is a non-issue (its only consumer is a kind-only `debug!`). Tests: **127 passed / 1
ignored** under `--features onnx`, zero warnings on both feature sets. **But** the new candidate scan is quadratic
and runs inline on the request thread:

- [ ] **M4-R19 (BLOCKER — algorithmic-complexity DoS, a regression introduced by `0c7193b`).**
  `Recognizer::push_candidates` (`src/pii/recognizers.rs`) resumes `regex.find_at` at `m.start()+1` after every
  hit. For the two **unbounded-length** recognizers — Email local part `[A-Za-z0-9._%+-]+@…` and Secret
  `sk-[A-Za-z0-9_-]{6,}` — that is **O(n) start positions × O(n) match length = O(n²)**. Measured (release),
  `"a"*N + "@b.co"`: N=16k → 982 ms, N=50k → 9.5 s, N=100k → 37 s, N=200k → **151 s** on HEAD, vs **~2–8 ms** at
  `6c68150` (~18,000× at 200 KB; doubling N quadruples time — textbook O(n²)). `"sk-"*100k` → 135 s vs 18 ms
  (~7,500×). The default body limit is **16 MiB with no per-field cap** (`DEFAULT_MAX_BODY_BYTES`, `config.rs:50`),
  so a single ~1–2 MB `content` field — trivially under the limit — pegs a core for minutes; masking runs
  **synchronously inline** on the tokio worker (`server.rs:251`, no `spawn_blocking`) and `mask_all` calls
  `detect()` ≥2×, so a handful of concurrent such requests exhaust the executor and the proxy stops serving. This
  is unauthenticated (masking precedes any upstream auth). Bounded-length recognizers (card/phone/IDs) stay linear
  — the blowup is specific to the two unbounded patterns, which is why the overlap rescan buys them **no** coverage
  (a value hidden inside an Email/Secret match merges into the same coalesced run regardless). **Fix (suggested):**
  bound the overlap rescan — either resume at `run.end` for unbounded recognizers and only do the `start()+1`
  rescan within a window of the recognizer's max plausible match length (card ≈19, ID ≈18 chars), or tag
  recognizers `overlap_scan: bool` and leave Email/Secret on plain `find_iter`. Independently, move masking to
  `spawn_blocking` and/or add a per-field size cap. **Test:** `detect()` and `mask_all` on `"a"*1_000_000 +
  "@b.co"` and `"sk-"*350_000` must complete in well under a few hundred ms (a guard that fails on O(n²)).
- [ ] **M4-R20 (low-medium — fail-OPEN on `mask_all` exhaustion; contradicts the fail-closed bar).** After
  `MAX_MASK_PASSES` the loop returns `current` **with no final re-detect** (`src/pii/anonymizer.rs:83-95`), so any
  PII still present is forwarded in clear. The ARCHITECTURE/comment claim *"hitting it can only mean over-masking …
  not a leak"* is **unproven**: "each pass strictly shrinks the un-masked text" guarantees *eventual* convergence,
  not convergence *within 4 passes*. **Not shown reachable** — an exhaustive search over 314,432 glued inputs (plus
  handcrafted 5-value chains) never exceeded **2** passes, because masking *fragments* a digit run rather than
  peeling it, keeping exposure depth tiny — so this is a **latent** fail-open, not a demonstrated leak. **Fix:**
  after the loop, one final `try_detect(&current)`; if non-empty, return `Err(DetectError)` so `PrivacyStage`
  blocks (fail closed). Keep the `warn!`; correct the ARCHITECTURE sentence. **Test:** a synthetic detector whose
  masking always re-exposes one entity → `mask_all` must return `Err`, not forward.
- [ ] **M4-R21 (low — perf note, not a bug).** `mask_all` runs the detector **≥2×** on every PII-bearing request
  (one to mask, one to confirm the fixpoint). Under NER this ~doubles inference: single `detect()` **64 ms** vs full
  `mask_all` **127 ms** (1.99×, 20 iters, XLM-R int8). Inherent to the fixpoint design and acceptable for
  correctness; fold into M5's perf harness. Possible later optimization: skip the confirmation pass when the last
  `mask` changed no bytes adjacent to un-masked digits.
- [ ] **M4-R22 (low — M5 CI blocker-in-waiting).** No `rust-version`/MSRV in `Cargo.toml` (edition 2021), yet
  `Option::is_none_or` (`recognizers.rs:304`) requires **Rust ≥1.82**. The planned M5 CI on an older toolchain
  would fail to compile. **Fix:** add `rust-version = "1.82"` to `[package]`.

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

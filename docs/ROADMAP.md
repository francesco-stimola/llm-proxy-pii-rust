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

### From the M1 code review — each item is fix (+ test where needed)
- [x] **IBAN over-match** (`src/pii/recognizers.rs`) — regex now matches the two canonical IBAN shapes (continuous / 4-groups) so a match can't extend into a trailing ALL-CAPS word. **Test**: corpus `IBAN-04` + `iban_does_not_absorb_a_following_word`.
- [x] **Wire `iban_mod97`** (`src/pii/recognizers.rs`) — now used in the detection path via `confidence_of`: a mod-97-valid IBAN is `Verified`, a structure-only one `Structural` (still masked). `PiiEntity.confidence` carries the signal. **Test**: `structural_iban_is_masked_but_flagged`.
- [x] **Double `system` message** (`src/pipeline/privacy.rs`) — the augmentation now merges into an existing `system`/`developer` message; only one reaches the upstream. **Test**: `int07_augmentation_merges_into_existing_system_message`.
- [x] **`demask` double scan** (`src/pii/anonymizer.rs`) — replaced by a single regex pass (also the demask-robustness fix). Covered by round-trip + tolerance tests.
- [x] **Upstream response headers** (`src/proxy.rs` / `src/server.rs`) — a safe allowlist (`retry-after`, `x-request-id`, `x-ratelimit-*`, `openai-*`, `anthropic-*`) is forwarded; content/hop-by-hop headers are dropped (body is re-serialized). **Test**: `e2e_forwards_safe_response_headers_only`.

### From the M1.5 code review — follow-ups (review before M2) ✅
- [x] **Array-content fail-closed gap** (`src/pipeline/privacy.rs`, `mask_content`) — array `content` now masks bare-string elements, masks the `text` of object parts (skips non-text parts like `image_url`), and **fails closed on any other element** (number/bool/null/nested array). **Test**: `content_array_bare_string_is_masked_not_leaked` + `content_array_scalar_element_fails_closed`.
- [x] **Phone international over-match** (`src/pii/recognizers.rs`) — the international arm is now two canonical shapes (`+CC gg gg gggg` / `+CC gg ggggggg`) instead of `1–4` open groups, so it can't swallow a trailing number (`+39 333 0000001 12345` masks only the phone). **Test**: `phone_international_span_stops_at_the_number`.
- [x] **`Confidence` consumed** (`src/pii/recognizers.rs`) — `detect` now emits a `debug` audit log for every `Structural` match (kind only, never the value), so the field is read, not write-only. Reserved for richer use (audit sink, ML thresholds) in M2.

## M2 — Unstructured entities (ONNX NER, CPU)
Goal: add names / organizations / locations via a local ML model.
Candidate models & evaluation plan: [docs/M2-NER-EVALUATION.md](M2-NER-EVALUATION.md).
- [ ] `OnnxNerDetector` behind the `onnx` feature (CPU execution provider)
- [ ] Evaluate candidate models against the corpus; pick the most reliable one
- [ ] Combine deterministic + ML detectors behind one `PiiDetector`
- [ ] Extend the corpus with unstructured-entity cases
- [ ] **NER concurrency**: model-instance pool / request queue so inference isn't a single-threaded bottleneck under load

## M3 — Streaming
Goal: SSE token streaming with incremental de-anonymization.
- [ ] Streaming passthrough of provider responses
- [ ] Hold-back buffer that restores placeholders split across chunks

## M4 — GPU optimization & load
Goal: faster inference once the model is locked, and prove it holds under load.
- [ ] GPU execution provider (CUDA / DirectML) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline
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

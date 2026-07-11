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

## M1.5 — Robustness & fail-closed  🔒
For a privacy tool the failure mode *is* the product: when anything is unexpected,
it must **fail closed** (block / scrub), never forward raw PII. Hardens M1 before
we broaden detection.
- [ ] **Fail-closed policy**: unrecognized payload shapes, endpoints, or internal errors never pass PII through un-masked (block/scrub, never fail open)
- [ ] **Full field coverage**: audit every text-bearing field of the chat schema (system/developer content, `name`, `tools[].function` description/params, array content parts) — an unscanned field is a leak
- [ ] **API scope decision**: which endpoints are in scope (chat/completions only, or also `/v1/responses`, `/v1/embeddings`?); out-of-scope defaults to fail-closed
- [ ] **Adversarial / evasion tests**: obfuscated emails, exotic phone shapes, PII split across fields — measure *recall* (a miss = a leak)
- [ ] **Body-size limit** tuned for long-context requests (avoid silent 413 / OOM)
- [ ] **Demask robustness**: tolerate or detect model-corrupted placeholders (`[EMAIL 1]`, translated/split tokens) so restore never silently fails

### From the M1 code review — each item is fix (+ test where needed)
- [ ] **IBAN over-match** (`src/pii/recognizers.rs`) — the IBAN regex greedily absorbs a trailing uppercase word: `"IBAN IT60X0542811101000000123456 EUR"` masks `…456 EUR`. **Fix**: tighten the pattern so a match can't extend into a following all-letter token. **Test**: adversarial case in `tests/corpus/pii_cases.json` + a recognizer unit test asserting the span is exactly the IBAN and `EUR` is left untouched.
- [ ] **Wire `iban_mod97`** (`src/pii/recognizers.rs`) — defined but used only in tests. **Fix**: use it as a confidence signal on IBAN hits (valid vs structure-only), or document it as reserved for M2 — remove the dead-code ambiguity. Structure-only IBANs must stay masked (keep corpus `IBAN-03` green). **Test**: only if behaviour changes.
- [ ] **Double `system` message** (`src/pipeline/privacy.rs`) — when the client already sent a `system` message, the augmentation inserts a second one at index 0. **Fix**: merge/append the augmentation into the existing system message (or place it deterministically). **Test**: integration test with a client-supplied `system` message asserting the agreed single-system-message shape reaches the upstream.
- [ ] **`demask` double scan** (`src/pii/anonymizer.rs`) — drops a redundant pass. **Fix**: remove the `contains` guard and call `replace` directly (identical behaviour, one pass instead of two). **Test**: not needed — covered by existing round-trip tests.
- [ ] **Upstream response headers** (`src/proxy.rs` / `src/server.rs`) — currently dropped. **Fix**: forward a safe subset of the upstream response headers (at least `content-type`; consider rate-limit headers). **Test**: e2e assertion that a header set by the mock upstream reaches the client.

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

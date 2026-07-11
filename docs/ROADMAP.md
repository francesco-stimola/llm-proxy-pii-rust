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

## M1 — Structured PII pipeline (CPU, no ML)

Locales: **IT + US**. Placeholder format: **`[KIND_N]`** (e.g. `[EMAIL_1]`).

### Part A — Masking core
Reproduce the old proxy's structured-PII behavior, better, in Rust.
- [x] `PiiDetector` trait + `PiiEntity` / `PiiKind` types (incl. `Secret`)
- [x] Deterministic recognizers: email, phone (IT/US), SSN, credit card (Luhn), IBAN, with priority-based overlap resolution
- [x] SECRET recognizer (API keys / tokens, e.g. `sk-…`, `sk-ant-…`, `AKIA…`) — deterministic; the old ML model missed these
- [x] `Vault`: mask to `[KIND_N]` placeholders + exact-restore demask (deterministic per value)
- [ ] Privacy stage wired into the pipeline
- [ ] axum server forwarding `/v1/chat/completions` upstream (non-streaming)
- [x] Port the reference tests to Rust; no false positives; multi-PII round-trip exact (corpus-driven + proptest, 16 tests green)

### Part B — Prompt augmentation & round-trip  ⭐ primary feature
Make the masked data actually usable by the model — without this it mishandles
placeholders, especially in tool calls. A headline capability, not a nice-to-have.
- [ ] Transparent system-prompt injection: teach the model that `[KIND_N]` are typed real values, to be used verbatim (incl. tool-call arguments) and never altered
- [ ] Round-trip covers `tool_calls` arguments (de-anon in responses) and tool results (re-anon in requests)
- [ ] Deterministic placeholder assignment (same value → same token across turns)
- [ ] Integration tests INT-01…06 green

## M2 — Unstructured entities (ONNX NER, CPU)
Goal: add names / organizations / locations via a local ML model.
Candidate models & evaluation plan: [docs/M2-NER-EVALUATION.md](M2-NER-EVALUATION.md).
- [ ] `OnnxNerDetector` behind the `onnx` feature (CPU execution provider)
- [ ] Evaluate candidate models against the corpus; pick the most reliable one
- [ ] Combine deterministic + ML detectors behind one `PiiDetector`
- [ ] Extend the corpus with unstructured-entity cases

## M3 — Streaming
Goal: SSE token streaming with incremental de-anonymization.
- [ ] Streaming passthrough of provider responses
- [ ] Hold-back buffer that restores placeholders split across chunks

## M4 — GPU optimization
Goal: faster inference once the model is locked.
- [ ] GPU execution provider (CUDA / DirectML) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline

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
Auth & rate-limiting stages, structured audit logging, config-file support,
additional providers, metrics/observability.

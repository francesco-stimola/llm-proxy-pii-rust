# Roadmap

Development is split into milestones. Each builds on the previous one and is
independently testable. The checkboxes track progress — keep them current as
work lands. **This document is the single source of truth for "what's next".**

## M0 — Project setup
- [x] Repo, license (MIT), README (EN) + README.it.md (IT)
- [x] Port PII test cases from the old proxy → `tests/reference/old-proxy/`
- [x] Tracking docs (roadmap, ARCHITECTURE, TESTING, SETUP, DEVLOG)
- [x] Rust module scaffold (uncompiled until the toolchain is installed)
- [x] Install Rust toolchain (rustup, per-user, no admin) — cargo/rustc 1.97
- [ ] MSVC linker via portable Build Tools (no admin) — see [SETUP](SETUP.md)
- [ ] First `cargo build` green; fix scaffold errors; verify dependency versions

## M1 — Structured PII pipeline (CPU, no ML)
Goal: reproduce the old proxy's structured-PII behavior, better, in Rust.
- [ ] `PiiDetector` trait + `PiiEntity` / `PiiKind` types
- [ ] Deterministic recognizers: email, phone, SSN, credit card (Luhn), IBAN
- [ ] SECRET recognizer (API keys / tokens, e.g. `sk-…`, `sk-ant-…`) — deterministic; the old proxy's ML model missed these
- [ ] `Vault`: mask to typed placeholders + exact-restore demask
- [ ] Privacy stage wired into the pipeline
- [ ] Transparent system-prompt injection so the model treats placeholders as typed real values (verbatim, incl. tool calls)
- [ ] Round-trip covers `tool_calls` arguments (de-anon in responses) and tool results (re-anon in requests)
- [ ] Deterministic placeholder assignment (same value → same token across turns)
- [ ] axum server forwarding `/v1/chat/completions` upstream (non-streaming)
- [ ] Port the reference test cases to Rust; all green
- [ ] No false positives on plain text; multi-PII round-trip is exact

## M2 — Unstructured entities (ONNX NER, CPU)
Goal: add names / organizations / locations via a local ML model.
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

## Backlog / later
Auth & rate-limiting stages, structured audit logging, config-file support,
additional providers, metrics/observability.

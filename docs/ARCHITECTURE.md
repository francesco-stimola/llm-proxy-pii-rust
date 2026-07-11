# Architecture

## Overview

`llm-proxy-pii-rust` is a reverse proxy in front of an OpenAI-compatible LLM
provider. It inspects each request, anonymizes PII locally, forwards the
anonymized request upstream, and restores the original values in the response.

## Design principles

- **Local-first** — PII detection runs on-box; nothing leaves for the filtering step.
- **Modular pipeline** — request/response transformations are `Stage`s. Only the
  privacy stage is wired now, but auth / rate-limit / logging can be added later
  without touching the core.
- **Engine-agnostic detection** — everything sits behind the `PiiDetector` trait,
  so we can swap models or add engines without touching the proxy.
- **CPU-first, GPU later** — correctness and reproducibility on CPU first; GPU is
  an optimization milestone (M4) behind a feature flag. GPU behavior isn't
  automatic — it depends on the model and quantization.
- **Textbook & lean** — idiomatic Rust, low RAM/CPU, no over-engineering.

## Hybrid detection (key decision)

Two classes of PII, handled differently:

| Class        | Examples                              | Engine |
|--------------|---------------------------------------|--------|
| Structured   | email, phone, SSN, credit card, IBAN, secret | deterministic regex + validation (Luhn, IBAN checksum) — high precision, no model |
| Unstructured | names, organizations, locations       | ONNX NER model (M2) |

The old proxy's ONNX `openai/privacy-filter` was unreliable on the ML part.
Keeping deterministic recognizers for structured PII removes most of the
reliability risk *and* most of the compute cost — the ML model only carries the
unstructured-entity load.

## Anonymization

Detected spans are replaced with typed placeholders of the form `[KIND_N]` — e.g.
`[EMAIL_1]`, `[PERSON_2]` — ASCII and tokenizer-friendly. A per-request `Vault`
maps placeholder → original so the response can be restored exactly (round-trip).
Text with no PII passes through unchanged.

## Prompt augmentation (helping the model read placeholders)

The downstream model sees only masked values, so it must be told how to read
them — otherwise it can mishandle them, especially in tool calls (treating
`[EMAIL_1]` as literal noise instead of a stand-in for an email). The privacy
stage therefore **transparently injects a system instruction** into the outgoing
request, stating that:

- values like `[EMAIL_1]`, `[PERSON_2]` are placeholders standing in for real
  data of the named type;
- they must be used verbatim — including as tool-call arguments — never altered
  or guessed at;
- they will be restored downstream before anyone real sees them.

This expands the round-trip scope:

- **Tool calls are in scope** — `tool_calls` arguments in the response are
  de-anonymized (so the client runs tools with real values), and tool *result*
  messages coming back are re-anonymized before going upstream. It is not just
  chat message text.
- **Placeholder assignment is deterministic** — the same real value maps to the
  same placeholder every time it appears, so the model can correlate it across a
  multi-turn (stateless) conversation where history is re-sent and re-masked on
  each request.

## Robustness & fail-closed (M1.5)

For a privacy proxy the failure mode *is* the product: anything unexpected must
**fail closed** (block or scrub), never forward raw PII.

- **Fail-closed request handling.** A stage can set `RequestContext.block` when it
  hits something it can't safely mask — an unreadable `content` shape (a bare
  object/scalar) or a missing/!array `messages`. The proxy then returns **400**
  and never forwards. Masking always runs *before* forwarding, so a masked value
  can't leak even on a later error.
- **API scope.** Only `POST /v1/chat/completions` is proxied; `GET /healthz` is
  served for liveness. Every other path/method returns **404** via the router
  `fallback` and is never forwarded — we don't proxy schemas we don't model
  (`/v1/responses`, `/v1/embeddings`, … are out of scope for now).
- **Field coverage.** The masker scans *every* text-bearing field of the chat
  schema — message `content` (string and the `text` of array parts, all roles),
  `name`, `tool_calls[].function.arguments`, legacy `function_call.arguments`,
  `tools[].function.description`, and every `description` inside
  `tools[].function.parameters`. One shared per-request `Vault` means the same
  value gets the same token even when it's split across fields.
- **Body-size limit.** `MAX_BODY_BYTES` (default 16 MiB) is applied via
  `DefaultBodyLimit`, above axum's 2 MiB default, so long-context requests aren't
  silently rejected.
- **Tolerant de-masking.** Restore accepts model-mangled placeholders
  (`[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]`) in one pass; a placeholder that looks
  like ours but isn't in the vault is logged rather than silently shipped.
- **Response headers.** Only a safe allowlist of upstream response headers is
  forwarded (`retry-after`, `x-request-id`, `x-ratelimit-*`, `openai-*`,
  `anthropic-*`); content/hop-by-hop headers are dropped because the body is
  re-serialized after de-masking.
- **Detection confidence.** `PiiEntity` carries a `Confidence` (`Verified` vs
  `Structural`). A structure-only IBAN (mod-97 fails) is still masked but tagged
  `Structural`; the signal is available to audit logging now and ML thresholds in
  M2.
- **Known M2 gap — NER fails *open*.** Structured PII is always fail-closed, but the
  M2 NER layer is not yet: when a configured model fails to *load* it falls back to
  structured-only (`build_detector`), and when a per-request *inference* errors the
  detector yields no NER entities (`OnnxNerDetector::detect`) — in both cases raw
  **unstructured** PII (names/orgs/locations) is forwarded upstream. This is a
  deliberate deferral (the `PiiDetector::detect` signature has no error channel, and
  the right policy needs a real error rate to tune) tracked in ROADMAP M2-R1/R2; the
  structured guarantees above are unaffected.

## Module layout

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | binary entry: tracing, config, run server |
| `src/config.rs` | runtime configuration |
| `src/server.rs` | axum router + handlers |
| `src/proxy.rs` | request/response value objects + the upstream HTTP client (the pipeline is applied in `server.rs`) |
| `src/pipeline/mod.rs` | `Stage` trait |
| `src/pipeline/privacy.rs` | the privacy stage (only one wired) |
| `src/pii/mod.rs` | `PiiDetector` trait, `PiiEntity` / `PiiKind` / `Confidence` |
| `src/pii/recognizers.rs` | deterministic structured-PII recognizers (M1) |
| `src/pii/overlap.rs` | shared span overlap resolution (`PiiKind::priority`) |
| `src/pii/composite.rs` | `CompositeDetector` — combine detectors behind one trait |
| `src/pii/anonymizer.rs` | `Vault`: mask / demask |
| `src/pii/ner_decode.rs` | pure NER decode (label→kind, BIO→spans) — model-independent |
| `src/pii/onnx.rs` | ONNX NER detector (M2, feature `onnx`) — tokenizer + `ort` session pool |

## Stack

tokio (async runtime) · axum + tower (HTTP + modular layers) · reqwest (upstream,
streaming) · serde / serde_json · regex + once_cell (recognizers) · `ort` (ONNX
Runtime, M2, feature `onnx`) · `tokenizers` (M2, feature `onnx`).

**Hybrid detection (M2).** `CompositeDetector` runs the deterministic recognizers
and — when the `onnx` feature is on and the model env vars are set — the
`OnnxNerDetector` over the same text, merging spans through `overlap`. NER config
is env-driven: `NER_MODEL_PATH`, `NER_TOKENIZER_PATH`, `NER_LABELS` (comma-separated
labels in class-id order), optional `NER_POOL_SIZE` (session pool for concurrency).
A missing/failed model logs and falls back to structured-only. Choosing the model
is a *measured* step (`docs/M2-NER-EVALUATION.md`); the code path is ready for it.

**Toolchain:** Rust with the **MSVC** target on Windows. On a machine without
admin rights, install rustup per-user and the MSVC linker via portable Build Tools
— full procedure in `docs/SETUP.md`. MSVC is required to link the `ort` / ONNX
Runtime native library at M2.

## Decisions & open points

- **Placeholder format: `[KIND_N]`** (e.g. `[EMAIL_1]`) — ASCII, tokenizer-friendly.
- **Locales: IT + US** — Italian and US phone numbers; IBAN including Italian; US SSN.
- **Resolved (M1)**: the `Stage` signature threads a per-request `RequestContext`
  (carrying the `Vault`) from request to response.
- **Resolved (M1.5)**: the scanned text fields are fixed — see *Robustness &
  fail-closed → Field coverage* above.

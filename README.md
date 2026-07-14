# llm-proxy-pii-rust

A fast, privacy-preserving reverse proxy for OpenAI-compatible LLM APIs, written in Rust.

`llm-proxy-pii-rust` sits between your application and any OpenAI-compatible provider
(OpenAI, GitHub Copilot, Anthropic's compat endpoint). It detects personally
identifiable information (PII) **locally, before a request ever leaves your
network**, replaces it with typed placeholders, forwards the anonymized request
upstream, and restores the original values in the response — so your application
sees coherent, real data while the provider never does.

Point your existing OpenAI-compatible client at the proxy's URL; nothing else in
your stack has to change.

## Why

Sending prompts to a hosted LLM means trusting a third party with whatever your
users type — names, emails, phone numbers, national IDs, account numbers, API
keys. This proxy keeps detection and redaction under your control, on your own
infrastructure, instead of relying on the provider to handle it for you. It fails
**closed**: on any unexpected input, it blocks or scrubs rather than risk
forwarding something raw.

## What it does

- **Structured PII (deterministic, always on)** — email, phone (US + `+CC`),
  credit card (Luhn), IBAN (mod-97 + per-country length), API keys/secrets
  (`sk-…`, `sk-ant-…`, `AKIA…`), and national identifiers for ten countries (US
  SSN, IT Codice Fiscale, GB NINO, ES DNI/NIE, FR NIR, DE Steuer-ID, NL BSN, PT
  NIF, LV personal code, zh Resident ID) — each checksum- or rule-validated for a
  near-zero false-positive rate, and masked regardless of locale configuration
  (privacy-first: a national ID that reaches the proxy is masked even if its
  country isn't configured).
- **Unstructured entities (optional ML, `onnx` feature)** — people, organizations,
  and locations via a local ONNX NER model (XLM-R int8), covering Arabic, German,
  English, Spanish, French, Italian, Latvian, Dutch, Portuguese, and Chinese.
  CPU-first; off by default so the shipped binary stays native-dependency-free.
- **Reversible, deterministic anonymization** — detected values become typed
  placeholders (`[EMAIL_1]`, `[PERSON_2]`, …); a per-request vault restores the
  exact originals in the response. The same value always gets the same token
  within a request, so a stateless multi-turn conversation (history re-sent and
  re-masked every turn, as OpenAI-style clients do) stays consistent turn to
  turn.
- **Tool-call aware** — placeholders are restored in `tool_calls[].function.arguments`
  before your client runs a tool, and re-masked in `tool` result messages before
  they go back upstream. A transparent system-prompt injection tells the model
  the placeholders are real-data stand-ins to use verbatim, never to alter or
  guess at.
- **Streaming (SSE)** — `stream:true` requests are masked exactly like buffered
  ones and de-anonymized incrementally as tokens arrive, with a hold-back buffer
  so a placeholder split across two chunks (`[EMA` + `IL_1]`) still resolves
  correctly.
- **Multi-provider** — one `UPSTREAM_PROVIDER` setting routes to OpenAI, GitHub
  Copilot, or Anthropic's OpenAI-compatible endpoint. Masking is identical
  regardless of provider: presets only change routing (path, headers), never
  what gets scrubbed.
- **Fail-closed by design** — an unreadable request shape, a required detector
  that errors, or masking that can't reach a stable fixpoint all **block the
  request (400)** rather than forward anything of unknown PII status. Only
  `POST /v1/chat/completions` is proxied; everything else is `404`, never
  forwarded.

## Quick start

Requires Rust (stable, MSRV 1.82). Build and run the default (structured-only,
native-dependency-free) binary:

```sh
cargo build --release
UPSTREAM_API_KEY=sk-... ./target/release/llm-proxy-pii-rust
```

Then point any OpenAI-compatible client at `http://127.0.0.1:8080` instead of the
real provider:

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"email me at jane@example.com"}]}'
```

The upstream provider only ever sees `[EMAIL_1]`; your client gets
`jane@example.com` back.

To add unstructured-entity detection (names, organizations, locations), build with
the `onnx` feature and point it at a model — see [Unstructured entities
(NER)](#unstructured-entities-ner-optional-onnx-feature) below.

## Configuration

Everything is environment-driven. Core:

| Variable | Default | Purpose |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Address the proxy listens on |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | Base URL of the upstream provider |
| `UPSTREAM_API_KEY` | *(unset)* | Injected as `Authorization: Bearer …` when the client sends none of its own |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — selects routing defaults (chat path, forwarded headers) |
| `UPSTREAM_CHAT_PATH` | *(preset default)* | Override the chat-completions path |
| `UPSTREAM_FORWARD_HEADERS` | *(preset default)* | Comma-separated client request headers to pass through (beyond `Authorization`) |
| `UPSTREAM_EXTRA_HEADERS` | *(none)* | `Key=Value;Key2=Value2` static headers added to every upstream request |
| `MAX_BODY_BYTES` | `16777216` (16 MiB) | Request body size limit |
| `PII_LOCALES` | `it,us` | Locales for the opt-in, false-positive-prone recognizer tier (national ID packs are always on regardless — see `docs/ARCHITECTURE.md`) |
| `PII_DEBUG_SKIP_DEMASK` | off | **Debug only.** Skips response de-masking so the client sees the placeholders the provider saw — proof the round-trip is wired. Never enable in production |
| `RUST_LOG` | *(unset)* | Standard `tracing` env-filter, e.g. `llm_proxy_pii_rust=debug` |

### Unstructured entities (NER, optional `onnx` feature)

```sh
cargo build --release --features onnx
```

| Variable | Default | Purpose |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(unset)* | Explicit local model files — zero outbound calls, always wins if set |
| `NER_MODEL_REPO` | *(unset)* | Opt-in auto-download (`owner/name`) of a revision-pinned model into the standard HuggingFace cache; only outbound call in the whole tool, made once at startup |
| `NER_MODEL_REVISION` | `478a2a3` | Pinned revision for auto-download (the evaluated XLM-R int8) |
| `NER_POOL_SIZE` | `2` | Concurrent ONNX session pool size |
| `NER_REQUIRED` | off | Fail **closed** for names: a missing/failing NER blocks the request (400) instead of silently falling back to structured-only |

With neither `NER_MODEL_PATH` nor `NER_MODEL_REPO` set, the `onnx` build simply
runs structured-only, same as the default build. See `docs/ARCHITECTURE.md` and
`docs/SETUP.md` for the full model-management contract.

## Status

**M0–M4 complete**, M5 (integration & performance testing) in progress. Ten
national-ID packs, three-tier locale coverage, streaming, multi-provider routing,
and an algorithmically-linear masking path (measured — see `docs/TESTING.md`)
are all shipped and tested. `v0.4.0`, pre-1.0: the first tagged release
(`1.0.0`) follows once M5's README/CI pass lands.

## Development

Living documents that track the whole build so nothing gets lost between
sessions:

- [Development setup (Windows, no admin)](docs/SETUP.md)
- [Architecture & design decisions](docs/ARCHITECTURE.md)
- [Roadmap & milestones](docs/ROADMAP.md)
- [Testing strategy](docs/TESTING.md)
- [Manual verification runbook](docs/MANUAL_VERIFICATION.md)
- [Development log](docs/DEVLOG.md)

```sh
cargo test                    # default (structured-only) test suite
cargo test --features onnx    # + NER-path tests (model-dependent ones are #[ignore]d)
```

## License

Copyright (C) 2026 Francesco Stimola.

Licensed under the **GNU Affero General Public License v3.0 or later**
(`AGPL-3.0-or-later`) — see [LICENSE](LICENSE). As a network-served privacy proxy,
AGPL ensures anyone who runs a **modified** version as a service shares their
changes; running it unmodified carries no such obligation.

---

🇮🇹 Una traduzione italiana è disponibile in [README.it.md](README.it.md).

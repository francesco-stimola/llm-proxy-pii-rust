<div align="center">

```
██╗     ██╗     ███╗   ███╗      ██████╗ ██████╗  ██████╗ ██╗  ██╗██╗   ██╗      ██████╗ ██╗██╗
██║     ██║     ████╗ ████║      ██╔══██╗██╔══██╗██╔═══██╗╚██╗██╔╝╚██╗ ██╔╝      ██╔══██╗██║██║
██║     ██║     ██╔████╔██║█████╗██████╔╝██████╔╝██║   ██║ ╚███╔╝  ╚████╔╝ █████╗██████╔╝██║██║
██║     ██║     ██║╚██╔╝██║╚════╝██╔═══╝ ██╔══██╗██║   ██║ ██╔██╗   ╚██╔╝  ╚════╝██╔═══╝ ██║██║
███████╗███████╗██║ ╚═╝ ██║      ██║     ██║  ██║╚██████╔╝██╔╝ ██╗   ██║         ██║     ██║██║
╚══════╝╚══════╝╚═╝     ╚═╝      ╚═╝     ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝   ╚═╝         ╚═╝     ╚═╝╚═╝
```

### The PII firewall for your LLM traffic

**Your prompts leave masked. Your app still sees real data.**

Local-first detection · reversible placeholders · fail-closed · streaming · OpenAI-compatible

[![Release build & publish](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/release-build-publish.yml/badge.svg)](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/release-build-publish.yml)
[![Latest release](https://img.shields.io/github/v/release/francesco-stimola/llm-proxy-pii-rust?sort=semver&label=release)](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)
[![Locales](https://img.shields.io/badge/national%20IDs-10%20countries-green.svg)](#what-it-detects)

[Quick start](#quick-start) · [What it detects](#what-it-detects) · [How it works](#how-it-works) · [Configuration](#configuration) · [Architecture](docs/ARCHITECTURE.md)

</div>

---

## The problem

Every prompt you send to a hosted LLM is a copy of your users' data on someone else's
server — names, emails, phone numbers, national IDs, IBANs, API keys. You cannot
un-send it, and "we'll redact it later" is not a control.

`llm-proxy-pii-rust` puts the redaction step **on your side of the wire**, and makes it
reversible so nothing downstream breaks.

```
   your app                    THE PROXY                        provider
  ┌──────────┐          ┌────────────────────┐              ┌────────────┐
  │          │  real    │  detect  ──►  mask │  masked      │            │
  │  client  │─────────►│                    │─────────────►│  OpenAI /  │
  │          │  data    │  [EMAIL_1] [IBAN_1]│  only        │  Copilot / │
  │          │          │                    │              │  Anthropic │
  │          │◄─────────│  restore  ◄── vault│◄─────────────│            │
  └──────────┘  real    └────────────────────┘  placeholders└────────────┘
                data          local, on-box                  never sees raw PII
```

Point your existing OpenAI-compatible client at the proxy. Nothing else in your stack
changes.

---

## Quick start

Requires Rust **1.89+**.

```sh
cargo build --release --features onnx
UPSTREAM_API_KEY=sk-... ./target/release/llm-proxy-pii-rust
```

Then talk to it exactly like the real provider:

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user",
       "content":"email jane@example.com about invoice IT60X0542811101000000123456"}]}'
```

**What the provider actually received:**

```json
{"role":"user","content":"email [EMAIL_1] about invoice [IBAN_1]"}
```

**What your client got back:** the reply, with `jane@example.com` and the IBAN restored.

> The proxy never needs to hold your key: a client `Authorization` header always wins over
> `UPSTREAM_API_KEY`, so you can pass your own token per request and leave the proxy
> credential-free.

---

## What it detects

**Structured PII — deterministic, always on, no model.** Every match is checksum- or
rule-validated, so the false-positive rate stays near zero.

| | |
|---|---|
| **Universal** | email · phone (US + `+CC`) · credit card (Luhn) · IBAN (mod-97 + per-country length) · API keys & secrets (`sk-…`, `sk-ant-…`, `AKIA…`) |
| **National IDs** *(10 countries)* | 🇺🇸 SSN · 🇮🇹 Codice Fiscale · 🇬🇧 NINO · 🇪🇸 DNI/NIE · 🇫🇷 NIR · 🇩🇪 Steuer-ID · 🇳🇱 BSN · 🇵🇹 NIF · 🇱🇻 personal code · 🇨🇳 Resident ID |

National IDs are masked **regardless of locale configuration** — privacy-first: an ID that
reaches the proxy gets masked even if its country isn't the one you configured.

**Unstructured entities — local ONNX NER (XLM-R int8, CPU).** People, organizations and
locations across **ar · de · en · es · fr · it · lv · nl · pt · zh**. Runs on your own
hardware; large fields are chunked so long documents work too.

---

## How it works

1. **Detect** — deterministic recognizers scan every text-bearing field of the chat schema
   (`content`, `name`, tool-call arguments, tool descriptions, nested parameter
   descriptions), plus the NER for names/orgs/places.
2. **Mask** — each value becomes a typed placeholder (`[EMAIL_1]`, `[PERSON_2]`), recorded
   in a per-request vault. Masking runs **to a fixpoint**, because replacing one value can
   expose another.
3. **Teach the model** — a system instruction is injected telling it the placeholders stand
   for real data and must be used verbatim, including in tool-call arguments.
4. **Forward** — the provider sees placeholders and nothing else.
5. **Restore** — the vault puts the real values back, in buffered replies, in
   `tool_calls[].function.arguments`, and **incrementally in SSE streams** (a placeholder
   split across two chunks, `[EMA` + `IL_1]`, still resolves).

### The bar it holds itself to

- **Fail closed.** An unreadable request shape, a required detector that errors, or masking
  that can't reach a stable fixpoint all **block the request (400)** rather than forward
  anything of unknown PII status. Only `POST /v1/chat/completions` is proxied — everything
  else is `404`, never forwarded.
- **Never log raw PII.** Logs carry kinds, counts and placeholders — never values. Enforced
  by a test, not by convention.
- **Linear under load.** The masking path is provably linear in both field *size* and entity
  *count*, and CPU-bound work runs off the async executor — a large body can't stall the
  proxy for everyone. *A proxy that is down protects nothing.*
- **Deterministic.** The same value always maps to the same placeholder within a request, so
  stateless multi-turn conversations stay coherent.

---

## Providers & clients

The proxy speaks **one** wire format — the OpenAI Chat Completions schema
(`/v1/chat/completions`) — on *both* sides. So "does it work with X?" is really two questions:
where it **forwards** (upstream) and what can **talk to it** (client).

**Upstream — where it forwards.** Any OpenAI-compatible endpoint. A `UPSTREAM_PROVIDER` preset picks
the right *envelope* for a known provider (masking is **identical** regardless — the preset changes
only routing, never what gets scrubbed):

| `UPSTREAM_PROVIDER` | Chat path | Client headers forwarded |
|---|---|---|
| `openai` (default) | `/v1/chat/completions` | `openai-organization`, `openai-project` |
| `copilot` | `/chat/completions` *(no `/v1`)* | `editor-version`, `editor-plugin-version`, `copilot-integration-id`, `openai-intent` |
| `anthropic` | `/v1/chat/completions` | `anthropic-version`, `anthropic-beta` |

...or any other OpenAI-compatible endpoint via `UPSTREAM_BASE_URL` (local models behind Ollama /
vLLM / LM Studio, Groq, Mistral, …).

> **Why a preset per provider if they're all OpenAI-compatible?** Because *"OpenAI-compatible"*
> describes the request **body**, not the whole HTTP call: all three share one JSON schema and differ
> only in the **envelope** the table shows — path + headers. A preset just bundles those per-provider
> deltas; it never changes the schema or what gets masked, and each part is overridable
> (`UPSTREAM_CHAT_PATH`, `UPSTREAM_FORWARD_HEADERS`, `UPSTREAM_EXTRA_HEADERS`).

**Clients — what can talk to it.** A client works with the proxy if you can point it at an
**OpenAI-compatible base URL**. Most modern agents can (via BYOK / a custom provider); the
exception is a client wired to a vendor's **native** protocol.

*Client compatibility verified **2026-07-15** — BYOK / custom-endpoint support in these tools moves
fast; check each tool's current docs before relying on it.*

| Client | Through the proxy? | How |
|---|---|---|
| OpenAI SDK · `curl` (OpenAI JSON) · Cline · Continue | ✅ | set the client's `base_url` to the proxy |
| **opencode** | ✅ | a custom `@ai-sdk/openai-compatible` provider, `baseURL: http://127.0.0.1:8080/v1` |
| **pi** (`@earendil-works/pi`) | ✅ | a provider with `"api": "openai-completions"`, `"baseUrl": ".../v1"` |
| **GitHub Copilot CLI** | ✅ | BYOK: `COPILOT_PROVIDER_BASE_URL=http://127.0.0.1:8080/v1` (model needs tools + streaming) |
| **GitHub Copilot Chat** (VS Code) | ✅ | BYOK → an "OpenAI Compatible" provider pointed at the proxy (chat only) |
| **Claude Code** · Anthropic SDK | ❌ not yet | native `/v1/messages` only — no OpenAI-compatible mode → [M6](docs/ROADMAP.md#m6) |

> **The test is the base URL, not the brand.** GitHub Copilot even lands on *both* axes — an
> *upstream* preset (`UPSTREAM_PROVIDER=copilot`) **and**, via BYOK, a *client* (Copilot CLI / Chat):
> same brand, two independent settings. (Copilot Chat's custom-endpoint BYOK is chat-only and was
> still rolling out in VS Code at the time of writing; the CLI's is generally available.)

So **Anthropic works as an *upstream*** (via its OpenAI-compatible endpoint), but a **native client
like Claude Code does *not* work** — same provider, different question. Native `/v1/messages` support
is the next milestone.

---

## Configuration

Everything is environment-driven.

| Variable | Default | Purpose |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Address the proxy listens on |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | Upstream provider base URL |
| `UPSTREAM_API_KEY` | *(unset)* | Injected as `Authorization: Bearer …` **only** when the client sends none of its own |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — routing preset |
| `UPSTREAM_CHAT_PATH` | *(preset)* | Override the chat-completions path |
| `UPSTREAM_FORWARD_HEADERS` | *(preset)* | Comma-separated client headers to pass through |
| `UPSTREAM_EXTRA_HEADERS` | *(none)* | `Key=Value;Key2=Value2` static headers for every upstream request |
| `MAX_BODY_BYTES` | `16777216` | Request body limit (16 MiB) |
| `PII_LOCALES` | `it,us` | Gates only the *false-positive-prone* recognizer tier. **National IDs are always on regardless** |
| `RUST_LOG` | *(unset)* | e.g. `llm_proxy_pii_rust=debug` |

<details>
<summary><b>NER (unstructured entities) — <code>--features onnx</code></b></summary>

<br>

| Variable | Default | Purpose |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(unset)* | Explicit local model files — **zero outbound calls**, always wins if set |
| `NER_MODEL_REPO` | *(unset)* | Opt-in auto-download (`owner/name`) of a revision-pinned model into the standard HuggingFace cache. The only outbound call in the whole tool, made once at startup, and it fetches **model artifacts, not user data** |
| `NER_MODEL_REVISION` | `478a2a3` | Pinned revision for auto-download |
| `NER_POOL_SIZE` | `2` | Concurrent ONNX session pool size |
| `NER_REQUIRED` | off | **Fail closed for names**: a missing or failing NER blocks the request (400) instead of silently degrading to structured-only |

With neither `NER_MODEL_PATH` nor `NER_MODEL_REPO` set, the build simply runs
structured-only.

</details>

<details>
<summary><b>Debug — see the masking with your own eyes</b></summary>

<br>

| Variable | Purpose |
|---|---|
| `PII_DEBUG_SKIP_DEMASK` | Skips the response de-mask, so your client receives **the placeholders the provider saw** — direct proof the round-trip is wired. Fires a loud startup warning. Never enable in production |
| `RUST_LOG=llm_proxy_pii_rust=trace` | Logs the exact (masked) bytes sent upstream. The de-masked reply is **never** logged |

Run the same prompt twice — once with `PII_DEBUG_SKIP_DEMASK=1`, once without — and compare.
Full procedure: [`docs/MANUAL_VERIFICATION.md`](docs/MANUAL_VERIFICATION.md).

</details>

---

## Documentation

| | |
|---|---|
| [Architecture & invariants](docs/ARCHITECTURE.md) | How it works, and **what must never break** |
| [Testing strategy](docs/TESTING.md) | The guards, and why each one exists |
| [Manual verification](docs/MANUAL_VERIFICATION.md) | Prove the chain end-to-end against a real provider |
| [Development setup](docs/SETUP.md) | Toolchain (incl. Windows, no admin) |

```sh
cargo test                    # structured-only suite
cargo test --features onnx    # + the NER path
```

---

## License

Copyright (C) 2026 Francesco Stimola.

**GNU Affero General Public License v3.0 or later** (`AGPL-3.0-or-later`) — see
[LICENSE](LICENSE). As a network-served privacy proxy, AGPL ensures anyone who runs a
**modified** version as a service shares their changes. Running it unmodified carries no
such obligation.

---

<div align="center">

🇮🇹 [Leggi in italiano](README.it.md)

</div>

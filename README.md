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
cargo build-onnx --release
UPSTREAM_API_KEY=sk-... ./target/onnx/release/llm-proxy-pii-rust
```

> `build-onnx` is a repo alias for `build --features onnx --target-dir target/onnx`
> (`.cargo/config.toml`). The hybrid builds into **its own directory** so that a plain
> `cargo build`/`cargo test` — which writes the same filename under `target/` — can never
> overwrite it with a structured-only binary. A default build works too, and drops the NER:
> structured PII only.

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

- **Fail closed.** An unreadable request shape, an unknown content-block type, a required
  detector that errors, or masking that can't reach a stable fixpoint all **block the request
  (400)** rather than forward anything of unknown PII status. Only `POST /v1/chat/completions`
  (and `POST /v1/messages` when `UPSTREAM_PROVIDER=anthropic`) is proxied — everything else is
  `404`, never forwarded.
- **Never log raw PII.** Logs carry kinds, counts and placeholders — never values. Enforced
  by a test, not by convention.
- **Linear under load.** The masking path is provably linear in both field *size* and entity
  *count*, and CPU-bound work runs off the async executor — a large body can't stall the
  proxy for everyone. *A proxy that is down protects nothing.*
- **Deterministic.** The same value always maps to the same placeholder within a request, so
  stateless multi-turn conversations stay coherent.

### Footprint — measured, not claimed

Resident memory, measured 2026-07-16 (Windows, debug build, idle after startup):

| build | private | working set | what dominates |
|---|---|---|---|
| **structured only** (default features) | **~10 MB** | ~36 MB | nothing — it's regex |
| **hybrid** (`--features onnx`, XLM-R int8, `NER_POOL_SIZE=2`) | **~834 MB** | ~862 MB | the NER: **two** ONNX sessions, i.e. the model held twice |

The deterministic layer is essentially free; **the model is the whole cost**, and it is tunable:
`NER_POOL_SIZE` (default `2`) trades concurrency for memory — `1` roughly halves it. Pick the
build to match the threat you actually have: structured-only already covers emails, IBANs, cards,
secrets and 10 national ID schemes, and it is language-independent. The NER buys you names,
organizations and locations — nothing else.

### Latency — also measured, on a realistic payload

Masking one **realistic Claude Code turn** (22.3 KiB: system prompt + 10 tool schemas + a user
message), 2026-07-17, reference box (Ryzen 5 PRO 8540U, 6 cores / 12 threads) on its **balanced /
energy-efficiency power plan** — a normal laptop in its normal state, which is the case this proxy
exists for — debug build, run in isolation, best of 3:

| detection | per turn | per KiB |
|---|---|---|
| **structured only** | **~20 ms** | ~0.9 ms |
| **hybrid** (either shape — see below) | **~4.7 s** | ~210 ms |

**The NER is ~100% of the cost** — the deterministic layer is well over 100× faster on the same
bytes.

**Treat that number as this box's, not as the product's.** Across seven measurements by two people
the same code and fixture ranged **2.5–7.1 s**. Two variables are known: running the benchmark
alongside other tests costs **1.5×** (measured), and the rest we cannot account for. What we can say
is what it is *not*: we spent a while attributing the spread to "AC versus battery" until the machine's
owner pointed out that both sets of runs were on the **same energy-efficiency profile** — the charger
was plugged in, the power plan never changed. That label had been separating nothing, which is why a
"battery" run beat three "AC" ones. So `~4.7 s` is the figure that reproduced twice, independently, in
isolation; the faster numbers we once published were the best of a noisy set, which is not the same
thing as the truth.

> A box on a **performance** power plan should do better than this, and we have not measured one. The
> figure above is deliberately the pessimistic, ordinary-laptop case rather than a best case we could
> stage.

**What is stable is the *improvement*, and that is the part that is about the code:** the shipped
default is **~1.8–2.3× faster** than the pre-threading version, measured across every one of those
regimes. If you want to check this repo's claim on your own box, that ratio is what to check — the
harness prints it, computed against a calibration leg measured seconds away in the same run.

**Which shape should you run?** If you front a single client (a coding agent, an IDE), set
**`NER_POOL_SIZE=1`**: it **halves the RAM** — one ONNX session instead of two, which is arithmetic
rather than a benchmark — and costs you nothing, because a single request only ever occupies one
session anyway. **Latency between the two shapes is a wash** (we have measured each winning by
10–15% on different runs; the difference is inside this box's noise). The pooled default exists for a
**shared** proxy, where it is worth ~30% more throughput — that one *is* a real, measured trade.

> **Measure on your own box before believing any of this:** `cargo test-onnx --test m7_latency --
> --ignored --nocapture --test-threads=1`. **The `--test-threads=1` is load-bearing** — without it
> cargo runs the benchmarks concurrently and they measure each other (1.5×). The harness prints the
> per-field breakdown, a thread sweep with min/median/**spread**, the concurrency figures, and —
> because absolute milliseconds are not comparable across machines — **a speedup ratio against an
> in-run calibration leg**, plus how far your box is from the one quoted here. A *release* build is
> irrelevant (measured: 3%): the cost is inside ONNX Runtime, a prebuilt native library, so
> compiling our Rust harder changes nothing.

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
| **Claude Code** · Anthropic SDK | ✅ new (M6) | point it at the proxy with `UPSTREAM_PROVIDER=anthropic`; the native `/v1/messages` body is masked **in place** (no OpenAI translation). *Live end-to-end verification against real Anthropic is the remaining [`1.0.0` gate](docs/ROADMAP.md#m6).* |

> **The test is the base URL, not the brand.** GitHub Copilot even lands on *both* axes — an
> *upstream* preset (`UPSTREAM_PROVIDER=copilot`) **and**, via BYOK, a *client* (Copilot CLI / Chat):
> same brand, two independent settings. (Copilot Chat's custom-endpoint BYOK is chat-only and was
> still rolling out in VS Code at the time of writing; the CLI's is generally available.)

So **Anthropic works as an *upstream*** (via its OpenAI-compatible endpoint) **and now as a *native
client*** — **M6** serves Anthropic's native `/v1/messages` (content blocks, `tool_use`/`tool_result`,
streaming) so a native client like Claude Code is masked without an OpenAI-compatible mode. The route
is registered only when `UPSTREAM_PROVIDER=anthropic`; on any other upstream `/v1/messages` still 404s.

---

## Configuration

Everything is environment-driven.

| Variable | Default | Purpose |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Address the proxy listens on |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | Upstream provider base URL |
| `UPSTREAM_API_KEY` | *(unset)* | Injected as `Authorization: Bearer …` **only** when the client sends none of its own |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — routing preset (`anthropic` also enables the native `/v1/messages` route, M6) |
| `UPSTREAM_CHAT_PATH` | *(preset)* | Override the chat-completions path |
| `UPSTREAM_MESSAGES_PATH` | `/v1/messages` | Override the native Anthropic Messages path (M6; only used when `UPSTREAM_PROVIDER=anthropic`) |
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
| `NER_POOL_SIZE` | `2` | Concurrent ONNX session pool size. **`1` roughly halves RAM and is the shape to use in front of a single client** (see the latency note above) |
| `NER_INTRA_THREADS` | *derived* | Threads **per session**. Defaults to `max(1, cores / NER_POOL_SIZE)` — the two knobs **multiply**, and the product must fit the box. Set it only if you know why |
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
cargo test         # structured-only suite  (target/)
cargo test-onnx    # + the NER path         (target/onnx/)
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

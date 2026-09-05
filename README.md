<div align="center">

  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.png">
    <img alt="llm-proxy-pii-rust" src="assets/logo-light.png" width="62%">
  </picture>

### The PII firewall for your LLM traffic

**Your prompts leave masked. Your app still sees real data.**

Local-first detection · reversible placeholders · fail-closed · streaming · OpenAI-compatible

[![Release build & publish](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/release-build-publish.yml/badge.svg)](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/release-build-publish.yml)
[![Latest release](https://img.shields.io/github/v/release/francesco-stimola/llm-proxy-pii-rust?sort=semver&label=release)](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)
[![PII kinds](https://img.shields.io/badge/PII%20kinds-11-green.svg)](#what-it-masks)

[Quick start](#quick-start) · [What it masks](#what-it-masks) · [How it works](#how-it-works) · [Configuration](#configuration) · [Architecture](docs/ARCHITECTURE.md)

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

Point your existing OpenAI-compatible client at the proxy. Nothing else in your stack changes.

---

## What it masks

**11 kinds · 10 national ID schemes · 5 VAT schemes · 9 domestic phone plans · 10 languages.** Each
kind becomes a typed, numbered placeholder, and the same value always gets the same token within a
request — so a masked conversation still makes sense to the model.

| kind | placeholder | engine | what makes it a match | coverage |
|---|---|---|---|---|
| **Email** | `[EMAIL_1]` | deterministic · always on | address shape, on ASCII word boundaries | universal |
| **Credit card** | `[CARD_1]` | deterministic · always on | **Luhn** checksum + 13–19 digits, compact or in the groupings issuers print (4-4-4-4, Amex 4-6-5, Diners 4-6-4, 19-digit 4-4-4-4-3) | universal |
| **IBAN** | `[IBAN_1]` | deterministic · always on | **mod-97** + that country's length | universal |
| **Secret / API key** | `[SECRET_1]` | deterministic · always on | issuer prefix — `sk-…`, `sk-ant-…`, `AKIA…` | universal |
| **Phone** | `[PHONE_1]` | deterministic · always on | **9 domestic plans** and every `+CC` rendering — compact E.164, and any grouping separated by a space, `-`, `.` or `/` — checked against the country's real numbering plan. The **US 3-3-4 form is shape-only**: it masks on grouping alone, so `555-867-5309` is masked though no plan assigns it (over-mask, never a miss) | `+CC` in any rendering, US always · **9 domestic plans** |
| **US SSN** | `[SSN_1]` | deterministic · always on | area / group / serial rules | 🇺🇸 |
| **National ID** | `[NATID_1]` | deterministic · always on | that scheme's own checksum — mod-23, mod-97, mod-11, ISO 7064, NINO rules | **10 countries** |
| **VAT / tax ID** | `[TAXID_1]` | deterministic · always on | that country's VAT checksum — mod-10, ISO 7064, mod-97, mod-11 (🇳🇱 by format: its scheme has none) | **5 countries** |
| **Person** | `[PERSON_1]` | **ML — needs a model** | the model reads the sentence: a name has no verifiable form | 10 languages · recall **0.83** |
| **Organization** | `[ORG_1]` | **ML — needs a model** | ″ | 10 languages |
| **Location** | `[LOCATION_1]` | **ML — needs a model** | ″ | 10 languages |

**The split is verifiability, not importance.** An IBAN is masked because mod-97 *returns*, not
because it looks like one — so those eight are provable, language-independent, and cost ~20 ms.
A name cannot be confirmed by any rule, so it takes a model, and the model is where the whole
cost lives (~4.7 s, ~563 MB) — and where the guarantee weakens: **without a model those three
travel upstream in clear**, which is what `NER_REQUIRED=1` turns into a startup failure.

**The two run side by side, not one after the other.** Both scan the full text and their spans
are merged; where they disagree on the same characters the deterministic match wins. No kind has
two engines — the NER emits only Person/Organization/Location and could not produce an email if
it tried. Which layer does more depends entirely on the traffic: a SQL result or a CSV export is
almost all deterministic, an ordinary chat turn is mostly NER.

### What it deliberately does not mask

Coverage is more useful stated in both directions, and a kind count compares badly across tools —
a document anonymizer that splits an address into street / number / postcode / city / province
reports five categories where this emits one `[LOCATION_1]`. That is resolution, not reach.

| not masked | why not |
|---|---|
| **dates · times · amounts** | No rule confirms them, and this proxy sits on **live agent traffic**: `[DATE_1]` where the model needed `2026-07-31` corrupts a tool call, and the agent then does the wrong thing quietly. A document anonymizer can afford that trade — an over-mask is visible to the human holding the document. A proxy cannot. **Excluded on purpose, not pending.** |
| **free-form addresses** | Nothing verifies "12 Rue de la Paix". It needs a model — `address` is in the optional GLiNER engine's default labels, off until its over-mask cost is measured. Tracked in [ROADMAP → *One model with more kinds*](docs/ROADMAP.md#backlog) |
| **age · gender** | Contextual attributes, same reason as above, and rarely the leak that matters in agent traffic |
| **a country not in the tables above** | Coverage here is a **list, not a rule**: 10 ID schemes, 5 VAT schemes, 9 domestic phone plans. A Brazilian mobile written without `+55` is not masked — not because the pattern is hard, but because *an unmeasured plan is not one we ship* |

The line through all four: **this masks what it can confirm, and says so where it cannot.** A
category is added when there is a corpus that measures it — which is how the nine phone regions
got here, and how the five VAT schemes below arrived.

**The 10 national ID schemes:** 🇺🇸 SSN · 🇮🇹 Codice Fiscale · 🇬🇧 NINO · 🇪🇸 DNI/NIE · 🇫🇷 NIR ·
🇩🇪 Steuer-ID · 🇳🇱 BSN · 🇵🇹 NIF · 🇱🇻 personal code · 🇨🇳 Resident ID. Masked **regardless of
locale configuration** — an ID that reaches the proxy gets masked even if its country isn't the
one you configured. An eleventh country's ID is not recognised: coverage here is a list, not a
rule, and *an unmeasured scheme is not one we ship*.

**VAT numbers — 5 schemes, always on:** the Italian **Partita IVA** in both its bare 11-digit form
(`P.IVA 00159560366`) and its VIES form, plus VIES-form 🇩🇪 🇬🇧 🇳🇱 🇵🇹 numbers (`DE136695976`,
`GB220430231`, `NL111222333B01`, `PT524287244`). They get their **own** token, `[TAXID_1]`, rather
than reusing `[NATID_1]`: a VAT number identifies a *business* and is personal data only when that
business is a sole trader, and a token that cannot tell it from a Codice Fiscale destroys the
distinction for everything downstream. Like the national IDs, this tier is **not** gated by
`PII_LOCALES`.

> Two costs, published rather than rounded off. **A bare 11-digit Partita IVA is a mod-10 check, so
> about 1 arbitrary 11-digit number in 10 is masked** — an order reference or a long id can become
> `[TAXID_1]`. It is restored byte-identically on the way back, so the round trip is exact; the cost
> is that the model sees a placeholder where it might have wanted the number. The *prefixed* forms
> have no such cost — `DE…`/`GB…`/`NL…`/`PT…` need the literal country code as well as the checksum.
> And 🇪🇸 🇫🇷 🇱🇻 VAT numbers are **not** recognised: their checksums are not measured here, and an
> unmeasured recogniser is not one we ship — the same rule that decided the phone regions.

**Domestic phone numbers — 9 regions, on by default:** 🇩🇪 🇪🇸 🇫🇷 🇬🇧 🇮🇹 🇱🇻 🇳🇱 🇵🇹 🇨🇳 numbers
written with **no `+CC`** (`020 7946 0958`, `06 69821234`, `347 1234567`, `91 123 45 67`). This is
the one tier `PII_LOCALES` narrows. A bare domestic number collides with ordinary digit sequences,
so what makes it safe is not the pattern but the check: the pure-Rust
[`phonenumber`](https://crates.io/crates/phonenumber) library confirms the candidate is a real
**assigned** number for that region. Measured over 35 real renderings and 453 digit-shaped
non-phones: **recall 1.000**, zero false positives on ports, amounts and reference numbers, and
**zero `Phone` spans at all** on a real 22 KiB Claude Code turn.

> The costs are published rather than rounded off: a space-separated date (`28 01 2026` is a valid
> Latvian number) or a numeric table column (`512 105 205` is a real Suzhou landline shape) can be
> masked. The full per-category matrix, and why that trade was taken, is in
> [ARCHITECTURE → *Domestic phone coverage*](docs/ARCHITECTURE.md#domestic-phone-coverage--re-measured-2026-07-29-m10).

---

## Quick start

### Run a released binary

Download the asset for your platform from the [latest
release](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest): one executable,
no installer, nothing to unpack. Take the unsuffixed name unless you want a `-cuda` / `-webgpu`
variant (see [GPU acceleration](#gpu-acceleration)).

```powershell
# Windows
Move-Item .\llm-proxy-pii-rust-x86_64-pc-windows-msvc.exe .\llm-proxy-pii-rust.exe

$env:NER_MODEL_REPO   = "jiting/xlm-roberta-base-ner-hrl_onnx"
$env:NER_REQUIRED     = "1"
$env:UPSTREAM_API_KEY = "sk-..."    # optional — a client's own header wins
.\llm-proxy-pii-rust.exe
```

```sh
# Linux / macOS  ·  on macOS also: xattr -d com.apple.quarantine ./llm-proxy-pii-rust
mv llm-proxy-pii-rust-x86_64-unknown-linux-gnu llm-proxy-pii-rust
chmod +x llm-proxy-pii-rust

export NER_MODEL_REPO=jiting/xlm-roberta-base-ner-hrl_onnx
export NER_REQUIRED=1
export UPSTREAM_API_KEY=sk-...
./llm-proxy-pii-rust
```

Two startup lines then tell you **which proxy you actually got**:

```text
2026-07-29T12:37:04.844780+02:00  INFO … ONNX NER detector loaded model="…model_quantized.onnx" pool_size=1 …
2026-07-29T12:37:04.885031+02:00  INFO … listening on http://127.0.0.1:8080
```

| | |
|---|---|
| `NER_MODEL_REPO` | **The one you can't skip.** The model is not bundled, and without it the proxy still starts — masking structured PII only, sending names, organizations and locations upstream in clear. One-time revision-pinned fetch into the HuggingFace cache |
| `NER_REQUIRED=1` | Turns that silent downgrade into a startup failure. If the first line above is missing, you are running structured-only |
| `PII_LOCALES` | Not needed — all nine domestic-phone regions are on by default. Set it only to *narrow* the set. National IDs are always on whatever you set |

There is no config file; configuration is environment-only, and the full table is under
[Configuration](#configuration). `--version` reports which build you have, `--help` lists every
variable, and `RUST_LOG` (default `info`) sets the log level.

### Build from source

Requires Rust **1.89+**.

```sh
cargo build-onnx --release
UPSTREAM_API_KEY=sk-... ./target/onnx/release/llm-proxy-pii-rust
```

> `build-onnx` is a repo alias for `build --features onnx --target-dir target/onnx`. The hybrid
> builds into **its own directory** so a plain `cargo build`/`cargo test` — which writes the same
> filename under `target/` — can never overwrite it with a structured-only binary. A default build
> works too, and drops the NER.

### Either way — talk to it exactly like the real provider

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

## How it works

1. **Detect** — deterministic recognizers scan every text-bearing field of the schema (`content`,
   `name`, tool-call arguments, tool descriptions, nested parameter descriptions), plus the NER
   for names / orgs / places.
2. **Mask** — each value becomes a typed placeholder, recorded in a per-request vault. Masking
   runs **to a fixpoint**, because replacing one value can expose another.
3. **Teach the model** — a system instruction is injected telling it the placeholders stand for
   real data and must be reused verbatim, including in tool-call arguments.
4. **Forward** — the provider sees placeholders and nothing else.
5. **Restore** — the vault puts the real values back: in buffered replies, in
   `tool_calls[].function.arguments`, and **incrementally in SSE streams** (a placeholder split
   across two chunks, `[EMA` + `IL_1]`, still resolves).

### The bar it holds itself to

- **Fail closed.** An unreadable request shape, an unknown content-block type, a required detector
  that errors, masking that can't reach a stable fixpoint, or a request so digit-dense it exhausts
  the phone-validation allowance all **block the request (400)** rather than forward anything of
  unknown PII status. Only `POST /v1/chat/completions` (and `POST /v1/messages` when
  `UPSTREAM_PROVIDER=anthropic`) is proxied — everything else is `404`, never forwarded.
- **Never log raw PII.** Logs carry kinds, counts and placeholders — never values. Enforced by a
  test, not by convention.
- **Linear under load, bounded per request.** The masking path is provably linear in field *size*
  and entity *count*, and CPU-bound work runs off the async executor. The phone-validation
  allowance is scoped to the **whole request**, so no client can buy more CPU by splitting a body
  across more fields; across every shape measured a request costs at most about **3 s**. *A proxy
  that is down protects nothing.*
- **Deterministic.** The same value always maps to the same placeholder within a request, so
  stateless multi-turn conversations stay coherent.

> **The allowance is reachable by a large, legal body — worth knowing before you meet it.** It
> counts *phone numbers*, not bytes: about **62,500** per request when written the common grouped
> way (`320 123 4567`), about 500,000 at the cheapest rendering. Which byte size that lands on
> depends on what surrounds them — the same 62,500 numbers are refused at **793 KB** as a bare
> column, **2 MB** as `name,phone`, **4.4 MB** as a six-column export. An ordinary 5,000-row export
> spends **8%**; a typical chat turn spends none. Retrying unchanged fails identically — send less
> digit-dense text (a `LIMIT` on the query behind an oversized tool result, fewer rows per call).

---

## Footprint & latency — measured, not claimed

Resident memory (Windows, debug build, idle after startup):

| build | private | working set | what dominates |
|---|---|---|---|
| **structured only** (default features) | **~10 MB** | ~36 MB | nothing — it's regex |
| **hybrid**, default `NER_POOL_SIZE=1` | **~563 MB** | ~585 MB | the NER: **one** ONNX session |
| **hybrid**, `NER_POOL_SIZE=2` | ~834 MB | ~856 MB | two sessions — each adds **~270 MB** |
| **hybrid + GLiNER int8** only (opt-in) | ~537 MB | ~560 MB | GLiNER's one session |
| **hybrid, XLM-R + GLiNER int8** (both, opt-in) | **~1073 MB** | ~1094 MB | **two models loaded** |

The deterministic layer is essentially free; **the model is the whole cost**, and it scales with
the pool (~290 MB shared base + ~270 MB per session). So pick the build to match the threat you
have: structured-only already covers seven of the ten kinds and is language-independent — the NER
buys you names, organizations and locations, nothing else.

Masking one **realistic Claude Code turn** (22.3 KiB: system prompt + 10 tool schemas + a user
message), reference laptop (Ryzen 5 PRO 8540U) on its ordinary energy-efficiency power plan, debug
build:

| detection | per turn | per KiB |
|---|---|---|
| **structured only** | **~20 ms** | ~0.9 ms |
| **hybrid** | **~4.7 s** | ~210 ms |

**The NER is ~100% of the cost** — the deterministic layer is well over 100× faster on the same
bytes. Treat `~4.7 s` as *this box's* number: across seven runs of the same code and fixture it
ranged **2.5–7.1 s**, and 4.7 is the figure that reproduced twice, independently, in isolation.
What is stable is the *improvement*, because that is the part about the code: the shipped default
is **at least 1.5× faster** than the pre-threading version — the floor the test actually asserts.

> **Measure your own box:** `cargo test-onnx --test m7_latency -- --ignored --nocapture
> --test-threads=1`. The `--test-threads=1` is load-bearing — without it cargo runs the benchmarks
> concurrently and they measure each other (1.5×). The harness prints a thread sweep, the
> concurrency figures, and a speedup ratio against an in-run calibration leg. Why `NER_POOL_SIZE`
> defaults to `1`, and when a centralizing proxy should raise it (~30% more throughput at `N×` the
> model RAM): [ARCHITECTURE → *NER threading*](docs/ARCHITECTURE.md).

---

## GPU acceleration

The NER can run on a GPU instead of the CPU. **Whether that is faster is a question about your
hardware, so the tool measures it rather than telling you** — `--bench-providers` runs the
model × provider matrix on your machine and names the winner. It works in *every* build:

```powershell
llm-proxy-pii-rust.exe --bench-providers
```

**One release binary per backend — download the one your machine can run.** A single ONNX Runtime
distribution carries a single set of execution providers, so no build can contain them all.

| your machine | download | accelerator you get |
|---|---|---|
| **Windows x64** | `…-x86_64-pc-windows-msvc` | **DirectML** — any DX12 GPU, incl. integrated |
| Windows x64, NVIDIA | `…-x86_64-pc-windows-msvc-cuda` | **CUDA** |
| Windows x64, cross-vendor | `…-x86_64-pc-windows-msvc-webgpu` | **WebGPU** |
| **Windows arm64** | `…-aarch64-pc-windows-msvc` | **DirectML** |
| **macOS (Apple Silicon)** | `…-aarch64-apple-darwin` | **CoreML** — Neural Engine / GPU |
| macOS, cross-vendor | `…-aarch64-apple-darwin-webgpu` | **WebGPU** (Metal) |
| **Linux x64** | `…-x86_64-unknown-linux-gnu` | CPU only |
| Linux x64, NVIDIA | `…-x86_64-unknown-linux-gnu-cuda` | **CUDA** |
| Linux x64, cross-vendor | `…-x86_64-unknown-linux-gnu-webgpu` | **WebGPU** (Vulkan) |
| **Linux arm64** | `…-aarch64-unknown-linux-gnu` | CPU only |

The **bold** row is the default pick for each platform; DirectML and CoreML are already inside it,
free. Rows are missing where ONNX Runtime ships no prebuilt (no CUDA/WebGPU on arm64; no ROCm or
OpenVINO anywhere).

Whichever you run, the backend stays **off unless you ask for it** (`NER_EXECUTION_PROVIDER`) and
falls back to CPU if the device isn't there. **On this project's reference box (AMD DX12 iGPU) the
CPU wins** and stays the default — DirectML-fp16 came out 1.45× at seq 128 but **0.38× at seq
512**, which is the length that matters because fields are chunked to 480 tokens. A discrete GPU
has 10–20× the bandwidth and would very likely win: that is exactly why the benchmark exists
instead of a claim.

> Two traps the report will not let you fall into — **backend and quantization are coupled** (int8
> is a CPU format; benchmarking the shipped int8 model on a GPU makes a good GPU look 2–5× slow, so
> pass an fp16 export via `NER_BENCH_MODELS`), and **the decision is made at seq 512**, not on the
> average. Full reasoning, the artifact matrix and what `ok` does *not* mean:
> [ARCHITECTURE → *Execution providers*](docs/ARCHITECTURE.md).

---

## Providers & clients

The proxy speaks the OpenAI Chat Completions schema on *both* sides, plus Anthropic's native
`/v1/messages`. So "does it work with X?" is two questions: where it **forwards**, and what can
**talk to it**.

**Upstream — where it forwards.** Any OpenAI-compatible endpoint. A `UPSTREAM_PROVIDER` preset
picks the right *envelope* for a known provider; masking is **identical** regardless — the preset
changes routing, never what gets scrubbed.

| `UPSTREAM_PROVIDER` | Chat path | Client headers forwarded |
|---|---|---|
| `openai` (default) | `/v1/chat/completions` | `openai-organization`, `openai-project` |
| `copilot` | `/chat/completions` *(no `/v1`)* | `editor-version`, `editor-plugin-version`, `copilot-integration-id`, `openai-intent` |
| `anthropic` | `/v1/chat/completions` | `anthropic-version`, `anthropic-beta` |

…or any other OpenAI-compatible endpoint via `UPSTREAM_BASE_URL` (local models behind Ollama /
vLLM / LM Studio, Groq, Mistral, …). Each part is overridable (`UPSTREAM_CHAT_PATH`,
`UPSTREAM_FORWARD_HEADERS`, `UPSTREAM_EXTRA_HEADERS`).

**Clients — what can talk to it.** A client works if you can point it at an OpenAI-compatible base
URL. Most modern agents can, via BYOK or a custom provider. *Verified 2026-07-15 — BYOK support in
these tools moves fast; check their current docs before relying on it.*

| Client | Through the proxy? | How |
|---|---|---|
| OpenAI SDK · `curl` · Cline · Continue | ✅ | set the client's `base_url` to the proxy |
| **opencode** | ✅ | a custom `@ai-sdk/openai-compatible` provider, `baseURL: http://127.0.0.1:8080/v1` |
| **pi** (`@earendil-works/pi`) | ✅ | a provider with `"api": "openai-completions"` |
| **GitHub Copilot CLI** | ✅ | BYOK: `COPILOT_PROVIDER_BASE_URL=http://127.0.0.1:8080/v1` |
| **GitHub Copilot Chat** (VS Code) | ✅ | BYOK → an "OpenAI Compatible" provider (chat only) |
| **Claude Code** · Anthropic SDK | ✅ | `UPSTREAM_PROVIDER=anthropic`; the native `/v1/messages` body is masked **in place**, no OpenAI translation. Verified live end-to-end against real Anthropic |
| **Native clients of other vendors** — Gemini CLI, Bedrock / Vertex native SDKs | ❌ | they speak a vendor **native** protocol the proxy doesn't serve |

> **Only two native surfaces exist: OpenAI-compatible and Anthropic `/v1/messages`.** An agent
> wired to any other vendor's native protocol is **not** masked — the concrete case today is the
> Gemini CLI. Such a client needs a per-provider, schema-aware adapter, and a missed schema field
> is a leak, so this is deliberate, **unscheduled** work: [Option B — native provider
> adapters](docs/ROADMAP.md#backlog). Until then, front such an agent through an OpenAI-compatible
> mode if it has one; if it speaks only its vendor's native API, the proxy cannot protect it yet.

---

## Configuration

Everything is environment-driven.

| Variable | Default | Purpose |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Address the proxy listens on |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | Upstream provider base URL |
| `UPSTREAM_API_KEY` | *(unset)* | Injected as `Authorization: Bearer …` **only** when the client sends none of its own |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — routing preset (`anthropic` also enables the native `/v1/messages` route) |
| `UPSTREAM_CHAT_PATH` | *(preset)* | Override the chat-completions path |
| `UPSTREAM_MESSAGES_PATH` | `/v1/messages` | Override the native Anthropic Messages path (only used when `UPSTREAM_PROVIDER=anthropic`) |
| `UPSTREAM_FORWARD_HEADERS` | *(preset)* | Comma-separated client headers to pass through |
| `UPSTREAM_EXTRA_HEADERS` | *(none)* | `Key=Value;Key2=Value2` static headers for every upstream request |
| `MAX_BODY_BYTES` | `16777216` | Request body limit (16 MiB) |
| `PII_LOCALES` | `de,es,fr,gb,it,lv,nl,pt,cn` | The regions whose **domestic phone numbers** (no `+CC`) are detected. Setting this **replaces** the set rather than adding to it; set it **empty** to turn the tier off — a different thing from leaving it unset. **National IDs are always on regardless** |
| `PII_CACHE_ENTRIES` | `16` | Detection cache: a byte-identical system prompt is scanned once and reused, saving the dominant NER pass. Keyed on exact bytes, so a hit can never mask *less*. `0` disables it |
| `RUST_LOG` | `info` | Log filter. Timestamps are local time with an explicit offset |

<details>
<summary><b>NER (unstructured entities) — <code>--features onnx</code></b></summary>

<br>

| Variable | Default | Purpose |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(unset)* | Explicit local model files — **zero outbound calls**, always wins if set |
| `NER_MODEL_REPO` | *(unset)* | Opt-in auto-download (`owner/name`) of a revision-pinned model into the standard HuggingFace cache. The only outbound call in the whole tool, made once at startup, and it fetches **model artifacts, not user data** |
| `NER_MODEL_REVISION` | `478a2a3` | Pinned revision for auto-download |
| `NER_POOL_SIZE` | `1` | Concurrent ONNX session pool size. **Default `1`** = the single-client shape (~563 MB, whole box per request). Raise to **`N`** for a centralizing proxy: ~30% more throughput at ~270 MB more RAM per session |
| `NER_INTRA_THREADS` | *derived* | Threads **per session**. Defaults to `max(1, base / NER_POOL_SIZE)`, where `base = min(physical cores, available parallelism)` — the two knobs **multiply**, and the product must fit the box. **The base is PHYSICAL cores, not logical threads (since v1.3.0):** on an SMT machine that halves the derived default, so both siblings of a core no longer run the same int8 GEMM. Set this explicitly to get the old shape back — an explicit value always wins. The startup log prints the base and which count decided it |
| `NER_EXECUTION_PROVIDER` | `cpu` | Hardware backend for the NER + GLiNER sessions. `cpu` is the default and the only one that has passed the determinism guard. Your platform's accelerator is **already in the `onnx` build**; a provider absent from the build, or whose device is missing, **falls back to CPU** (logged), never a startup failure — but a typo does fail startup. **Don't guess — run `--bench-providers`** |
| `NER_BENCH_MODELS` | *(unset)* | Extra model files (comma-separated) for `--bench-providers` to compare against the configured one — e.g. an fp16 export next to the shipped int8 |
| `NER_REQUIRED` | off | **Fail closed for names**: at least one ML detector must load, and every loaded one runs unwrapped — a failure blocks the request (400) instead of silently degrading to structured-only |

With neither `NER_MODEL_PATH` nor `NER_MODEL_REPO` set, the build simply runs structured-only.

</details>

<details>
<summary><b>GLiNER (contextual / open-label entities) — <code>--features onnx</code>, opt-in</b></summary>

<br>

A **second, optional** ML engine: a zero-shot span extractor for the contextual, open-label PII the
deterministic layer can't anchor and the XLM-R NER doesn't cover (a bare national phone, a
free-form address). It is **off by default and not a successor** to XLM-R — on the shipped int8
model its name recall is lower (0.58 vs XLM-R's 0.83), so it *adds* to the NER rather than
replacing it, and enabling both loads **two** models (~1.07 GB).

| Variable | Default | Purpose |
|---|---|---|
| `GLINER_MODEL_PATH` + `GLINER_TOKENIZER_PATH` + `GLINER_CONFIG_PATH` | *(unset)* | Explicit local files — **zero outbound calls**. Unset = GLiNER off |
| `GLINER_LABELS` | `person,organization,location,phone number,address` | Natural-language entity types; each maps to a `PiiKind` (an unmappable label is rejected) |
| `GLINER_THRESHOLD` | `0.15` | Per-span probability threshold — low because the int8 model's confidences run low; lower = more recall, more over-mask |
| `GLINER_POOL_SIZE` / `GLINER_INTRA_THREADS` | `1` / *derived* | Session pool + per-session threads, same shape as the NER's knobs |

`onnx-community/gliner_multi_pii-v1` (int8) is the tested model; `model_fp16.onnx` raises Person
recall to 0.67 at more RAM. The measured decision is in [DEVLOG 2026-07-19](docs/DEVLOG.md).

</details>

<details>
<summary><b>Debug — see the masking with your own eyes</b></summary>

<br>

| Variable | Purpose |
|---|---|
| `PII_DEBUG_SKIP_DEMASK` | Skips the response de-mask, so your client receives **the placeholders the provider saw** — direct proof the round-trip is wired. Fires a loud startup warning. Never enable in production |
| `RUST_LOG=llm_proxy_pii_rust=trace` | Logs the exact (masked) bytes sent upstream. The de-masked reply is **never** logged |

Run the same prompt twice — once with `PII_DEBUG_SKIP_DEMASK=1`, once without — and compare. Full
procedure: [`docs/MANUAL_VERIFICATION.md`](docs/MANUAL_VERIFICATION.md).

</details>

---

## Documentation

| | |
|---|---|
| [Changelog](CHANGELOG.md) | What changed in each release — and what the release page shows |
| [Architecture & invariants](docs/ARCHITECTURE.md) | How it works, and **what must never break** |
| [Testing strategy](docs/TESTING.md) | The guards, and why each one exists |
| [Manual verification](docs/MANUAL_VERIFICATION.md) | Prove the chain end-to-end against a real provider |
| [Roadmap](docs/ROADMAP.md) · [Development log](docs/DEVLOG.md) | What's next; how every decision was reached |
| [Development setup](docs/SETUP.md) | Toolchain (incl. Windows, no admin) |

```sh
cargo test         # structured-only suite  (target/)
cargo test-onnx    # + the NER path         (target/onnx/)
```

---

## License

Copyright (C) 2026 Francesco Stimola.

Dual-licensed:

- **Open source** — [GNU Affero General Public License v3.0 or later](LICENSE)
  (`AGPL-3.0-or-later`). As a network-served privacy proxy, AGPL ensures anyone who runs a
  **modified** version as a service shares their changes. Running it unmodified carries no such
  obligation — for anyone, companies included; the AGPL itself is free, always.
- **Commercial** — for cases the AGPL doesn't cover for you (embedding a modified version, or
  linking the crate into a closed-source product, without the network-copyleft obligation), a
  separate commercial license is available. [Open a commercial-license
  issue](https://github.com/francesco-stimola/llm-proxy-pii-rust/issues/new?template=commercial-license.md)
  to discuss.

Contributions are welcome under the terms in [CONTRIBUTING.md](CONTRIBUTING.md), which is what
keeps dual-licensing possible.

---

<div align="center">

🇮🇹 [Leggi in italiano](README.it.md)

</div>

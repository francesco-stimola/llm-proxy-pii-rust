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

### Run a released binary

Download the asset for your platform from the [latest
release](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest): one executable,
no installer, nothing to unpack. Take the unsuffixed name unless you want a `-cuda` / `-webgpu`
variant (see *GPU acceleration* below).

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

Two startup lines then tell you which proxy you actually got:

```text
2026-07-29T12:37:04.844780+02:00  INFO … ONNX NER detector loaded model="…model_quantized.onnx" pool_size=1 …
2026-07-29T12:37:04.885031+02:00  INFO … listening on http://127.0.0.1:8080
```

> Timestamps are **local time with the offset shown**, and the default log level is `info` — so a
> freshly downloaded binary says what it is doing without any configuration. `RUST_LOG` still
> overrides it (`RUST_LOG=warn`, `RUST_LOG=llm_proxy_pii_rust=debug`, …).
> `llm-proxy-pii-rust --version` reports which build you have, and `--help` lists every
> environment variable.

| | |
|---|---|
| `NER_MODEL_REPO` | **The one you can't skip.** The model is not bundled, and without it the proxy still starts — masking structured PII only, sending names, organizations and locations upstream in clear. One-time revision-pinned fetch into the HuggingFace cache |
| `NER_REQUIRED=1` | Turns that silent downgrade into a startup failure. If the first line above is missing, you are running structured-only |
| `PII_LOCALES` | Not needed — all nine domestic-phone regions are on by default. Set it only to *narrow* the set (it replaces the default). National IDs are always on whatever you set |

There is no config file — configuration is environment-only, and the full table is under
[Configuration](#configuration).

### Build from source

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

## What it detects

**Structured PII — deterministic, always on, no model.** Every match is checksum- or
rule-validated, so the false-positive rate stays near zero.

| | |
|---|---|
| **Universal** | email · phone (US + `+CC`) · credit card (Luhn) · IBAN (mod-97 + per-country length) · API keys & secrets (`sk-…`, `sk-ant-…`, `AKIA…`) |
| **National IDs** *(10 countries)* | 🇺🇸 SSN · 🇮🇹 Codice Fiscale · 🇬🇧 NINO · 🇪🇸 DNI/NIE · 🇫🇷 NIR · 🇩🇪 Steuer-ID · 🇳🇱 BSN · 🇵🇹 NIF · 🇱🇻 personal code · 🇨🇳 Resident ID |
| **Domestic phone** *(9 regions, all on by default)* | 🇩🇪 🇪🇸 🇫🇷 🇬🇧 🇮🇹 🇱🇻 🇳🇱 🇵🇹 🇨🇳 numbers written with **no `+CC`** — `020 7946 0958`, `06 69821234`, `347 1234567`, `91 123 45 67`, `01 23 45 67 89` — each checked against that country's **real numbering plan**, so order numbers and IDs aren't over-masked |

National IDs are masked **regardless of locale configuration** — privacy-first: an ID that
reaches the proxy gets masked even if its country isn't the one you configured.

The **domestic phone** tier is the only one `PII_LOCALES` touches, and it is now **on out of the
box**. A bare domestic number collides with ordinary digit sequences, so what makes it safe is not
the pattern but the check: the pure-Rust [`phonenumber`](https://crates.io/crates/phonenumber)
library confirms a candidate is a real **assigned** number for the region. Measured over 35 real
renderings and 405 digit-shaped non-phones: **recall 1.000**, zero false positives on ports, money
amounts and reference numbers, and **zero `Phone` spans at all** on a real 22 KiB Claude Code turn.
What it does cost is space- or dash-separated dates (`28 01 2026` is a valid Latvian number) —
the full matrix, and why that trade was taken, is in
[ARCHITECTURE → *Domestic phone coverage*](docs/ARCHITECTURE.md).

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

Resident memory (Windows, debug build, idle after startup; structured-only measured 2026-07-16, the
hybrid pool rows 2026-07-17):

| build | private | working set | what dominates |
|---|---|---|---|
| **structured only** (default features) | **~10 MB** | ~36 MB | nothing — it's regex |
| **hybrid**, default `NER_POOL_SIZE=1` (`--features onnx`, XLM-R int8) | **~563 MB** | ~585 MB | the NER: **one** ONNX session |
| **hybrid**, `NER_POOL_SIZE=2` | ~834 MB | ~856 MB | **two** sessions — the model held twice; each session adds **~270 MB** |
| **hybrid + GLiNER int8** only (opt-in, `GLINER_MODEL_PATH`, XLM-R off) | ~537 MB | ~560 MB | GLiNER's one session — comparable to XLM-R alone (M8) |
| **hybrid, XLM-R + GLiNER int8** (both, opt-in) | **~1073 MB** | ~1094 MB | **two models loaded** — GLiNER adds **~510 MB** on top of XLM-R (M8) |

The deterministic layer is essentially free; **the model is the whole cost**, and it scales with the
pool: **~290 MB of shared base plus ~270 MB per session** (measured — 563 MB at `pool=1`, 834 MB at
`pool=2`, so `pool=N` ≈ 290 + N×270 MB — *not* a clean doubling, because the runtime and the first
session's arenas are shared). `NER_POOL_SIZE` (**default `1` since 2026-07-17**) is one session — the
lean single-client shape; a **centralizing** proxy raises it to `N` for concurrent throughput at that
RAM. Pick the build to match the threat you actually have: structured-only already covers emails,
IBANs, cards, secrets and 10 national ID schemes, and it is language-independent. The NER buys you
names, organizations and locations — nothing else.

> **GLiNER (M8) is opt-in and *additive*** — it does **not** replace the NER (measured: on int8 its
> name recall is below XLM-R's — see [ROADMAP M8](docs/ROADMAP.md#m8)). Enabled alongside XLM-R it loads
> a **second** model, so budget the RAM: **~1.07 GB** for both vs ~563 MB for XLM-R alone (measured).
> Run GLiNER *instead* of XLM-R (~537 MB) only if you specifically want its contextual kinds and accept
> the weaker name recall.

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
default is **at least 1.5× faster** than the pre-threading version — the floor the test actually
asserts — and **typically ~1.7–2.3×** depending on the box. (We quote the floor, not the best
number seen: the speedup cancels the box's power state but not its raw speed, so a faster machine
compresses the ratio *toward* the floor rather than away from it — which is why every tighter band
we published kept being undercut by the next clean run.) If you want to check this repo's claim on
your own box, that ratio is what to check — the harness prints it, computed against a calibration
leg measured seconds away in the same run.

**Which shape should you run?** The default — **`NER_POOL_SIZE=1`** (since 2026-07-17) — is the
single-client shape (a coding agent, an IDE): it holds **one** ONNX session, so it uses **~270 MB
less RAM** than the pooled shape (**measured**: 563 MB vs 834 MB — dropping the second session frees
its ~270 MB copy of the weights, about a third off the total) and gives your one in-flight request
the whole box. It costs the personal case **nothing**, because a single request only ever occupies
one session anyway, and **latency between the two shapes is a wash** (we have measured each winning by
10–15% on different runs; the difference is inside this box's noise). If instead you run a
**shared / centralizing** proxy fronting concurrent clients, set **`NER_POOL_SIZE=N`**: the pool is
worth **~30% more throughput** (equivalently, `pool=1` is ~−23% under concurrent load) — that one
*is* a real, measured trade, bought at `N×` the model RAM.

> **Measure on your own box before believing any of this:** `cargo test-onnx --test m7_latency --
> --ignored --nocapture --test-threads=1`. **The `--test-threads=1` is load-bearing** — without it
> cargo runs the benchmarks concurrently and they measure each other (1.5×). The harness prints the
> per-field breakdown, a thread sweep with min/median/**spread**, the concurrency figures, and —
> because absolute milliseconds are not comparable across machines — **a speedup ratio against an
> in-run calibration leg**, plus how far your box is from the one quoted here. A *release* build is
> irrelevant (measured: 3%): the cost is inside ONNX Runtime, a prebuilt native library, so
> compiling our Rust harder changes nothing.

### GPU acceleration — measure, don't guess (`--bench-providers`)

The NER can run on a GPU instead of the CPU (`NER_EXECUTION_PROVIDER`, M9). **Whether that is
faster is a question about your hardware, so the tool measures it rather than telling you.**

**`--bench-providers` works in every build** — you never need a special one to ask the question:

```powershell
llm-proxy-pii-rust.exe --bench-providers
```

**One release binary per backend — download the one your machine can run.** A single ONNX Runtime
distribution carries a single set of execution providers, so no build can contain them all; the
choice moves to which artifact you grab. Every combination below is one ONNX Runtime actually ships
a prebuilt for:

| your machine | download | accelerator you get |
|---|---|---|
| **Windows x64** | `…-x86_64-pc-windows-msvc` | **DirectML** — any DX12 GPU (AMD/NVIDIA/Intel, incl. integrated) |
| Windows x64, NVIDIA | `…-x86_64-pc-windows-msvc-cuda` | **CUDA** |
| Windows x64, cross-vendor | `…-x86_64-pc-windows-msvc-webgpu` | **WebGPU** (D3D12/Vulkan via Dawn) |
| **Windows arm64** | `…-aarch64-pc-windows-msvc` | **DirectML** |
| **macOS (Apple Silicon)** | `…-aarch64-apple-darwin` | **CoreML** — Neural Engine / GPU |
| macOS, cross-vendor | `…-aarch64-apple-darwin-webgpu` | **WebGPU** (Metal via Dawn) |
| **Linux x64** | `…-x86_64-unknown-linux-gnu` | CPU only |
| Linux x64, NVIDIA | `…-x86_64-unknown-linux-gnu-cuda` | **CUDA** |
| Linux x64, cross-vendor | `…-x86_64-unknown-linux-gnu-webgpu` | **WebGPU** (Vulkan via Dawn) |
| **Linux arm64** | `…-aarch64-unknown-linux-gnu` | CPU only |

The **bold** row is the default pick for each platform. Rows are missing where ONNX Runtime has no
prebuilt: there is no CUDA or WebGPU build for either arm64 platform, and no ROCm or OpenVINO build
for *any* target — those would need a from-source ONNX Runtime, which this project does not ship.

Note that DirectML and CoreML come free: they are inside their platform's plain distribution, so the
standard binary already has them. CUDA and WebGPU are not — they change which runtime is downloaded,
which is exactly why they are separate artifacts instead of weight in everyone's default build.

Whichever you run, the backend is still **off unless you ask for it** (`NER_EXECUTION_PROVIDER`), it
falls back to CPU if the device isn't there, and `--bench-providers` tells you whether it is worth
using at all. Without the `onnx` feature the flag still runs and explains there is no ML layer to
accelerate.

It runs the **model × provider matrix** on your machine and names the winner:

Real output from this project's box (an AMD DX12 iGPU), abridged only where marked:

```text
Execution-provider benchmark (M9)
  threads: 12 intra-op per session, pool 1 (of 12 cores)
  models: model_quantized

…/onnx/model_quantized.onnx
  provider     |    seq 128 |    seq 256 |    seq 512 | status
  ---------------------------------------------------------------------------------
  cpu          |      26.4ms |      45.9ms |     109.7ms | ok
  directml     |     121.4ms |     380.2ms |     691.7ms | ok

Decision is made at seq 512 — fields are chunked to 480 tokens, so a full window
runs near there, and long fields are what make a slow turn.

=> FASTEST: cpu on model_quantized (109.7ms).
   Keep the default — leave NER_EXECUTION_PROVIDER unset, and point
   NER_MODEL_PATH at that model.

[… MEASUREMENT and NOTE ON QUANTIZATION blocks …]

  !! An int8/quantized model is in this run and NO fp16 model is, so every GPU
     row here is very likely an UNDERESTIMATE — this comparison cannot answer
     'is the GPU worth it?'. Add an fp16 export via NER_BENCH_MODELS to get the
     honest matrix (CPU-int8 vs GPU-fp16).
```

Note what that run does **not** do: it does not conclude the GPU is bad. It measured the shipped
int8 model, so it says so and tells you the comparison is unfinished — which is the whole point.

Two things it will not let you get wrong — both mistakes this project made first:

- **Backend and quantization are coupled.** int8 is a *CPU* format whose ops partition badly onto
  GPUs; benchmarking the shipped int8 model on a GPU makes a perfectly good GPU look 2–5× slow.
  fp16 is the GPU format (on CPU it's up-cast to fp32, so it only pays off on a GPU). Pass an fp16
  export via `NER_BENCH_MODELS` to get the comparison that actually decides — the report refuses to
  present an int8-only run as an answer.
- **The decision is made at seq 512**, not on the average. Fields are chunked to 480 tokens, so the
  inferences that dominate latency run near there. A backend that wins at seq 128 and loses at 512
  has not won: seq 128 is already fast enough that the difference is invisible.

Run it on an **idle machine, on AC** — a busy or throttled box inflates every row (we measured ~3×
right after a heavy build; the ranking held, the absolute numbers didn't).

**On our reference box (AMD DX12 iGPU) the CPU wins** and stays the default: DirectML-fp16 came out
1.45× at seq 128 but **0.38× at seq 512**. A shared-memory iGPU is bandwidth-bound. That is a fact
about *that* iGPU — a discrete GPU has 10–20× the bandwidth and would very likely win, which is why
the selector exists. Yours may differ: that's what the benchmark is for.

A provider that can't initialize never fails startup — it falls back to CPU (logged), and the
benchmark marks that row `unavailable — fell back to cpu` instead of passing CPU timings off as a
GPU's. Note the limit of that check: it catches a provider that fails to **initialize**, not ONNX
Runtime's per-node partitioning, which can still run individual nodes on the CPU inside a provider
that did register. Read `ok` as "the backend was engaged", not "every node ran on it".

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
| **Claude Code** · Anthropic SDK | ✅ (M6) | point it at the proxy with `UPSTREAM_PROVIDER=anthropic`; the native `/v1/messages` body is masked **in place** (no OpenAI translation). Verified live end-to-end against real Anthropic. |
| **Native clients of other vendors** — e.g. the **Gemini CLI** (native Gemini API), Bedrock / Vertex native SDKs | ❌ **not supported** | they speak a vendor **native** protocol the proxy doesn't serve. The proxy masks OpenAI-compatible clients (above) and Anthropic's native `/v1/messages` (M6) — *nothing else*. Adding another native schema needs a per-provider adapter — **[Option B, Backlog](docs/ROADMAP.md#backlog)** (Gemini is the named next candidate). |

> **The test is the base URL, not the brand.** GitHub Copilot even lands on *both* axes — an
> *upstream* preset (`UPSTREAM_PROVIDER=copilot`) **and**, via BYOK, a *client* (Copilot CLI / Chat):
> same brand, two independent settings. (Copilot Chat's custom-endpoint BYOK is chat-only and was
> still rolling out in VS Code at the time of writing; the CLI's is generally available.)

So **Anthropic works as an *upstream*** (via its OpenAI-compatible endpoint) **and now as a *native
client*** — **M6** serves Anthropic's native `/v1/messages` (content blocks, `tool_use`/`tool_result`,
streaming) so a native client like Claude Code is masked without an OpenAI-compatible mode. The route
is registered only when `UPSTREAM_PROVIDER=anthropic`; on any other upstream `/v1/messages` still 404s.

> **Stated plainly: only two native surfaces exist — OpenAI-compatible and Anthropic `/v1/messages`.**
> A coding agent wired to any **other vendor's native protocol** is **not** masked. The concrete case
> today is the **Gemini CLI** (native Gemini `generateContent`); Bedrock / Vertex native SDKs are the
> same class. Such a client would need a **per-provider, schema-aware native adapter**, and a missed
> schema field is a leak — so this is deliberate, **unscheduled** work, tracked as
> [**Option B — native provider adapters**](docs/ROADMAP.md#backlog) (with Gemini named as the most
> likely next one). Until an adapter exists, front such an agent only through an OpenAI-compatible /
> BYOK mode if it has one; if it speaks only its vendor's native API, the proxy cannot protect it yet.

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
| `PII_LOCALES` | `de,es,fr,gb,it,lv,nl,pt,cn` | The regions whose **domestic phone numbers** (no `+CC`) are detected. All nine are on by default; setting this **replaces** the set rather than adding to it, and a code outside the list contributes nothing. **National IDs are always on regardless** — see the coverage matrix in [ARCHITECTURE](docs/ARCHITECTURE.md) |
| `PII_CACHE_ENTRIES` | `16` | Detection cache (S3): the byte-identical system prompt is scanned once and reused, saving the dominant NER pass. Keyed on exact bytes, so a hit can never mask *less* than a fresh scan. `0` disables it |
| `RUST_LOG` | `info` | Log filter. Unset (or empty) means `info`, so startup lines are visible with no configuration; set e.g. `warn` or `llm_proxy_pii_rust=debug` to override. Timestamps are local time with an explicit offset |

<details>
<summary><b>NER (unstructured entities) — <code>--features onnx</code></b></summary>

<br>

| Variable | Default | Purpose |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(unset)* | Explicit local model files — **zero outbound calls**, always wins if set |
| `NER_MODEL_REPO` | *(unset)* | Opt-in auto-download (`owner/name`) of a revision-pinned model into the standard HuggingFace cache. The only outbound call in the whole tool, made once at startup, and it fetches **model artifacts, not user data** |
| `NER_MODEL_REVISION` | `478a2a3` | Pinned revision for auto-download |
| `NER_POOL_SIZE` | `1` | Concurrent ONNX session pool size. **Default `1`** = one session — the single-client shape (~563 MB, whole box per request). Raise to **`N`** for a centralizing proxy: ~30% more throughput at ~270 MB more RAM per session (see the latency note above) |
| `NER_INTRA_THREADS` | *derived* | Threads **per session**. Defaults to `max(1, cores / NER_POOL_SIZE)` — the two knobs **multiply**, and the product must fit the box. Set it only if you know why |
| `NER_EXECUTION_PROVIDER` | `cpu` | Hardware backend for the NER + GLiNER sessions (M9). `cpu` is the default and the only one that has passed the determinism guard. Your platform's accelerator is **already in the `onnx` build** (DirectML on Windows, CoreML on macOS, CUDA on x86_64 Linux) — set this to its name to use it. A provider absent from the build, or whose device is missing, **falls back to CPU** (logged, with `requested=` on the startup line), never a startup failure; a typo (e.g. `vulkan`, not an ORT backend) does fail startup. **Only `directml` has been benchmarked** — and it lost to the CPU here. **Don't guess — run `--bench-providers`** (below) |
| `NER_BENCH_MODELS` | *(unset)* | Extra model files (comma-separated) for `--bench-providers` to compare against the configured one — e.g. an fp16 export next to the shipped int8. Backend and quantization are **coupled**, so this is how you get the comparison that actually decides |
| `NER_REQUIRED` | off | **Fail closed for names**: with it set, **at least one** ML detector (the NER and/or GLiNER below) must load, and every loaded one runs unwrapped — a failure blocks the request (400) instead of silently degrading to structured-only |

With neither `NER_MODEL_PATH` nor `NER_MODEL_REPO` set, the build simply runs
structured-only.

</details>

<details>
<summary><b>GLiNER (contextual / open-label entities) — <code>--features onnx</code>, opt-in</b></summary>

<br>

A **second, optional** ML engine (M8). GLiNER is a *zero-shot span extractor* — it detects the
contextual, open-label PII the deterministic layer can't anchor and the XLM-R NER doesn't cover: a
**bare national phone** with no `+CC`, a free-form **address**. It is **off by default and not a
successor** to XLM-R — on the shipped int8 model its name recall is lower — so it *adds* to the NER
rather than replacing it (measured decision: `docs/DEVLOG.md` 2026-07-19).

| Variable | Default | Purpose |
|---|---|---|
| `GLINER_MODEL_PATH` + `GLINER_TOKENIZER_PATH` + `GLINER_CONFIG_PATH` | *(unset)* | Explicit local files (model `.onnx` + `tokenizer.json` + the model's `gliner_config.json`) — **zero outbound calls**. Unset = GLiNER off |
| `GLINER_LABELS` | `person,organization,location,phone number,address` | Comma-separated natural-language entity types; each maps to a `PiiKind` (an unmappable label is rejected) |
| `GLINER_THRESHOLD` | `0.15` | Per-span probability threshold — low because the int8 model's confidences run low (measured); lower = more recall, more over-mask |
| `GLINER_POOL_SIZE` / `GLINER_INTRA_THREADS` | `1` / *derived* | Session pool + per-session threads, same shape as the NER's knobs (they multiply) |

`onnx-community/gliner_multi_pii-v1` (int8 `model_quantized.onnx`) is the tested model. For **higher name
recall** at more RAM, point `GLINER_MODEL_PATH` at `model_fp16.onnx` (~580 MB) instead — measured Person
recall **0.67 vs int8's 0.58** (fp32 gives no further gain on CPU, where ORT up-casts fp16→fp32). Neither
reaches XLM-R's 0.83, which is why GLiNER stays opt-in. Enabling GLiNER **alongside** the XLM-R NER loads
**two** models — budget the RAM accordingly.

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

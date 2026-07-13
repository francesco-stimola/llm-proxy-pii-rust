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
  a deferred (Backlog) optimization behind a feature flag. GPU behavior isn't
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

**Locale coverage (M4) — three tiers.** The structured recognizers split into:
- **Universal** — email, secret, credit card, IBAN (already any-country) and phone
  (US + `+CC`). Always on.
- **National identifiers** — US SSN (keeps `PiiKind::Ssn` / `[SSN_N]`) plus, under
  `PiiKind::NationalId` / `[NATID_N]`, one per XLM-R-aligned country: IT Codice Fiscale,
  GB NINO, ES DNI/NIE, FR NIR, DE Steuer-ID, NL BSN, PT NIF, LV personal code, zh China
  Resident ID. **Always on regardless of `PII_LOCALES`** (privacy-first — a national ID that
  reaches the proxy is masked even if its country isn't configured). Each is checksum- or
  rule-specific (mod-23 / mod-97 / mod-11 / ISO 7064 / NINO prefix rules) to stay near-zero
  false-positive when always on. The pure-numeric 9-/11-digit IDs (BSN/NIF, DE/LV) accept a
  small fraction of arbitrary numbers on checksum alone (~18% of 9-digit tokens); this is an
  **accepted over-mask tradeoff** (M4-R6) — privacy-first, never a leak — not context-gated
  (that would leak); the contextual precision path is GLiNER (Backlog).
- **FP-prone** — ambiguous recognizers (e.g. national *phone* formats with no `+CC`) —
  **opt-in per locale** via `PII_LOCALES` (`fp_prone_recognizers`). None yet.

So `PII_LOCALES` (default `it, us`, `Config.pii_locales`) gates only *ambiguous*
recognizers, not "which countries". The **language** domain for the NER is the model's
declared languages (XLM-R HRL: ar/de/en/es/fr/it/lv/nl/pt/zh — validated, see
`docs/DEVLOG.md`); structured PII is language-independent.

**Overlap resolution (`src/pii/overlap.rs`).** Every detector's candidate spans are merged
by `resolve_overlaps`: rank by `PiiKind::priority` (desc), then span length (desc), then
greedily keep what doesn't overlap something already kept. Structured PII outranks the ML
NER, so a checksum-backed IBAN always beats a `Person` guess on the same characters.

`Email` is a deliberate special case (**M4-R9**), because it is the only structured kind
carrying `@` and can overlap another recognizer **two** ways that need *opposite* outcomes:

| Shape | Overlap | Winner | Why |
|---|---|---|---|
| `4111111111111111@x.com` | Email **contains** the card | **Email** | the card is a substring of the local part — a false decomposition; masking only it would forward `@x.com` in clear |
| `4111 1111 1111 1111@x.com` | **partial** (an email local part can't hold a space, so the email is just `1111@x.com`) | **CreditCard** | Email would mask only the last group and leave `4111 1111 1111` **in clear** — a leak |

So containment is resolved **ahead of** priority by a gate (`drop_spans_contained_in_an_email`)
that removes structured spans lying *entirely inside* an email; everything else falls through to
priority, where `Email` sits **below every other structured kind** so the checksum-backed span
wins any partial overlap. The same gate covers the grouped IBAN and the spaced GB NINO
(`AB 12 34 56 C@x.com`).

> ⚠️ **Known leaks — this is not yet fail-safe in both directions (M4-R10 / M4-R11, OPEN — see
> `docs/ROADMAP.md`).** The resolver settles an overlap by **dropping the whole loser span**, so the
> loser's bytes are *abandoned in clear*. Two consequences, both reproduced through `Vault::mask`:
> (1) the containment gate deletes a contained structured span *before* the priority sort, but the
> containing email is **not guaranteed to survive** it — a third span partially overlapping the email
> drops the email too, stranding the already-deleted card/secret (`555 867 5309.4111111111111111@x.com`
> → `[PHONE_1].4111111111111111@x.com`); (2) with `Email` now the lowest structured tier, a structured
> span partially overlapping a **real email** wins and abandons the email's **local part**
> (`555 867 5309john.doe@example.com` → `[PHONE_1]john.doe@example.com`). The planned fix is to make the
> resolver's invariant *"no structured span's bytes are ever abandoned"* — **merge** partially-overlapping
> structured spans into their union instead of dropping the loser. Until then, treat the table above as the
> *intent*, not a guarantee. A left-over bare `@domain` is acceptable (not PII); a left-over local part is not.

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
- **NER fail-closed is configurable (`NER_REQUIRED`).** Structured PII is always
  fail-closed. For the M2 NER layer: by default a missing/failing NER falls back to
  structured-only (fail *open* for names — an explicit `FailOpen` wrapper). Setting
  **`NER_REQUIRED`** makes it fail *closed*: a configured-but-unloadable model is
  fatal at startup (`build_detector`/`AppState::new` return `Result`), and a
  per-request inference error blocks the request (400) via the fallible
  `PiiDetector::try_detect` error channel — whose `DetectError` carries only a
  static label, never input text.

## Debug & observability (M2.6)

Opt-in developer tools to *see* that masking holds end-to-end. Both are **off by
default** and neither weakens the fail-closed posture — request-side masking always
runs, so the upstream never sees raw PII regardless.

- **`PII_DEBUG_SKIP_DEMASK`** (on `Config.debug_skip_demask`): skips the response
  de-mask so the (local) client receives the placeholders the provider saw — proof the
  round-trip is wired. A **loud `warn!`** fires at startup when it's on, so it can't
  quietly linger in a deployment.
- **`trace!` of the masked upstream body** (`RUST_LOG=…=trace`): the exact bytes sent
  upstream, logged just before forwarding — masked at that point, so safe. `debug!`
  keeps the concise kind-only audit lines.
- **Safety boundary (rule):** the masked request and the raw provider response
  (placeholders only) are safe to log; the **final de-masked client output (real
  values) is NEVER logged**. Same bar as the future audit logging.

## Streaming & multi-provider routing (M3)

**Streaming (SSE).** When a request sets `stream:true`, the proxy masks it exactly as
a buffered request, forwards it, and streams the response back while
**de-anonymizing incrementally** (`src/stream.rs`). A placeholder like `[EMAIL_1]`
can be split across two token deltas, so `SseDemasker` keeps a **hold-back buffer** per
streamed field — one for each choice's `delta.content` **and** one per
`delta.tool_calls[].function.arguments` — emitting everything up to the last point that
could still be an incomplete placeholder and holding the rest until the next delta (or
stream end) resolves it. Robustness: if the upstream answers a `stream:true` request
with a **non-SSE** body (a JSON error, or a provider that ignored `stream`), the proxy
falls back to the buffered path (real status + content-type, `on_response` de-mask)
rather than forcing an event-stream; a **mid-stream upstream error** becomes a terminal
`event: error` (after flushing buffered content) instead of a broken connection.
Streaming **never weakens fail-closed**: request-side masking runs first, so the
provider only ever sees placeholders; a clean request (nothing masked) streams through
untouched.

**Multi-provider routing — Option A.** Every provider is reached through its
**OpenAI-compatible** endpoint, so a single schema feeds the masker (no new leak
surface). A `UPSTREAM_PROVIDER` preset (`openai` / `copilot` / `anthropic`) sets the
per-provider *shape* — the chat path (`upstream_chat_path`; Copilot drops `/v1`), the
allowlist of client request headers to pass through (`forward_request_headers`, e.g.
`anthropic-version` or editor headers), and any required static headers
(`upstream_extra_headers`) — each overridable by env. Base URL + API key stay
env-driven. Auth: the client's own `Authorization` wins, else the configured key as
`Bearer`. Anthropic's *native* `/v1/messages` schema is out of scope (Option B, Backlog).

## Module layout

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | binary entry: tracing, config, run server |
| `src/config.rs` | runtime configuration |
| `src/server.rs` | axum router + handlers |
| `src/proxy.rs` | request/response value objects + the upstream HTTP client (per-provider path/headers, raw + JSON send) |
| `src/stream.rs` | streaming (SSE) incremental de-anonymizer with the split-placeholder hold-back buffer (M3) |
| `src/pipeline/mod.rs` | `Stage` trait |
| `src/pipeline/privacy.rs` | the privacy stage (only one wired) |
| `src/pii/mod.rs` | `PiiDetector` trait, `PiiEntity` / `PiiKind` / `Confidence` |
| `src/pii/recognizers.rs` | deterministic structured-PII recognizers (M1) |
| `src/pii/overlap.rs` | shared span overlap resolution (`PiiKind::priority`) |
| `src/pii/composite.rs` | `CompositeDetector` — combine detectors behind one trait |
| `src/pii/anonymizer.rs` | `Vault`: mask / demask |
| `src/pii/ner_decode.rs` | pure NER decode (label→kind, BIO→spans) — model-independent |
| `src/pii/onnx.rs` | ONNX NER detector (M2, feature `onnx`) — tokenizer + `ort` session pool |
| `src/pii/hf.rs` | HuggingFace Hub model resolution (M2.5, feature `onnx`) — opt-in revision-pinned fetch into the standard HF cache + `id2label` parse |

## Stack

tokio (async runtime) · axum + tower (HTTP + modular layers) · reqwest (upstream,
streaming) · serde / serde_json · regex + once_cell (recognizers) · `ort` (ONNX
Runtime, M2, feature `onnx`) · `tokenizers` (M2, feature `onnx`).

**Hybrid detection (M2).** `CompositeDetector` runs the deterministic recognizers
and — when the `onnx` feature is on and the model env vars are set — the
`OnnxNerDetector` over the same text, merging spans through `overlap`. NER config
is env-driven: `NER_MODEL_PATH`, `NER_TOKENIZER_PATH`, `NER_LABELS` (comma-separated
labels in class-id order), optional `NER_POOL_SIZE` (session pool for concurrency),
`NER_TOKEN_TYPE_IDS` (BERT-family models), and `NER_REQUIRED` (fail-closed switch).
A missing/failed model logs and falls back to structured-only. The model was chosen
by *measurement* (XLM-R int8 — `docs/M2-NER-EVALUATION.md`, `docs/DEVLOG.md`).

**Model management (M2.5, feature `onnx`).** The model file is resolved in priority
order (`src/pii/hf.rs` + `server.rs::load_onnx_ner`):

1. **Explicit local** — `NER_MODEL_PATH` (+ `NER_TOKENIZER_PATH` + `NER_LABELS`). Zero
   outbound calls; the airtight-privacy path, and it always wins.
2. **Opt-in auto-download** — when `NER_MODEL_PATH` is unset but `NER_MODEL_REPO`
   (`owner/name`) is set, the `hf-hub` crate fetches a **revision-pinned** model into
   the *standard* HF cache (`<home>/.cache/huggingface/hub`, library-managed, deduped
   across tools). Tunables: `NER_MODEL_REVISION` (default `478a2a3`), `NER_MODEL_FILE`
   (default `onnx/model_quantized.onnx`), `NER_TOKENIZER_FILE`, `NER_CONFIG_FILE`;
   `NER_LABELS` is derived from the model's `config.json` `id2label` unless set.

**Privacy note.** The auto-download is **opt-in** (only when `NER_MODEL_REPO` is set)
and fetches **model artifacts, not user data** — the one outbound call in the whole
tool, made once at startup and logged (repo + revision, never any input). It honors
`HF_HOME` / `HF_HUB_CACHE`; with neither set it pins the conventional cache instead of
`hf-hub` 1.0's `/tmp` fallback on Windows. Nothing user-supplied ever leaves the box.

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

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

**Word boundaries are ASCII — `(?-u:\b)`, never a bare `\b` (M4-R13).** Rust `regex`'s default
`\b` is **Unicode-aware**: a Han / Kana / Cyrillic letter *is* a word character, so there is **no
boundary between a CJK character and a digit**. Chinese and Japanese have no inter-word spaces, so
the glued form is the *natural* way to write it — and with a Unicode `\b` every anchored recognizer
was **inert** in CJK prose, forwarding the PII in clear (`我的信用卡号是4111111111111111` matched
*nothing*; the zh Resident ID pack shipped in M4 never fired in Chinese). All anchored recognizers
therefore use `(?-u:\b)`, which counts only `[0-9A-Za-z_]` as word characters. The deliberate
anti-false-positive guarantee is preserved **exactly** — an ID still cannot fire inside a longer
*ASCII* token (`card4111111111111111`, API keys, hashes, base64) — it merely stops treating a
non-ASCII **letter** as part of a number. (`Email` / `Phone` are anchored by character classes, not
`\b`, so they were never affected.)

**Candidate generation must see *overlapping* matches (M4-R17).** `Regex::find_iter` is
leftmost-**non-overlapping**: after a hit it resumes at the match's *end*. A real value that **starts
inside** an earlier match of the *same* recognizer is therefore never emitted as a candidate — and an
invariant over the candidate set is then satisfied **vacuously**, because the resolver never learns the
value exists. *An invariant is only ever as strong as the set it quantifies over.* So a recognizer
resumes one `char` past a match's **start**, and its overlapping hits are coalesced into maximal runs
(which keeps the candidate set bounded on pathological input at no cost to coverage — the resolver
would union those spans anyway).

**…but only where the pattern's length is *bounded* (M4-R19).** That rescan probes O(n) start
positions, each costing at most one maximal match, so it is **O(n · L)** for a pattern whose longest
match is L. Linear while L is bounded — a card is ≤ 19 digits, an IBAN ≤ 44 chars, every national ID ≤
18 — but the two **unbounded** patterns, `Email` (`[…]+@[…]+`) and `Secret` (`sk-[…]{6,}`), have L =
O(n), so it degenerates to **O(n²)**: a ~1 MB `content` field, far under the 16 MiB body limit, pegged a
core for *minutes* on an unauthenticated path. Those two therefore keep plain `find_iter` semantics
(`Scan::Sequential` in `recognizers.rs`), and it **costs no coverage** — a same-recognizer match that
starts inside an earlier one is *contained* in it (both run greedily to the same word boundary), and the
one shape that isn't, a chained `a@b.com@c.com`, is caught by the **fixpoint** below instead. So the two
mechanisms are complementary: **bounded recognizers rescan; unbounded ones iterate.**

> **The rule for a new recognizer:** if its match length has no upper bound, it must **not** take
> `Scan::Overlapping`. `tests/complexity.rs` (DOS-01…03) is the guard — it fails on a super-linear scan
> in seconds rather than hanging.

**Masking must run to a *fixpoint* (M4-R17).** Masking **rewrites the bytes around what it replaced**,
and a value is only recognizable in context — so masking can *expose* PII that was not detectable
before:

```text
4111111111111111555 867 5309   one 19-digit run: not Luhn-valid, so correctly NOT a card
                               (an ID never fires inside a longer token)
4111111111111111[PHONE_1]      masking the phone SPLIT the run — the leftover is a clean,
                               Luhn-valid card, and it would go upstream in clear
```

`Vault::mask_all` therefore re-detects until the text yields nothing. It converges because a
placeholder is **inert** (no recognizer can match `[KIND_N]` or span across it), so each pass strictly
shrinks the un-masked text. The round-trip stays exact — every pass records raw value → placeholder, and
`demask` restores them all in one tolerant pass.

**Exhausting `MAX_MASK_PASSES` fails *closed* (M4-R20).** The bound is a safety net, not a proof: *"each
pass strictly shrinks the un-masked text"* buys **eventual** convergence, never convergence **within**
four passes. So `mask_all` **confirms** the fixpoint rather than assuming it — one final `try_detect`,
and if anything is still detectable it returns `Err` and `PrivacyStage` **blocks** the request (400).
Forwarding a *probably*-clean text is exactly the failure mode a privacy proxy must not have. (No input
has ever been shown to need more than **2** passes — an exhaustive search over 314k glued inputs never
exceeded it, because masking *fragments* a digit run rather than peeling it — so this stays a latent
path. It is fail-closed regardless, which is the point: the bar does not depend on the search being
exhaustive.)

**Overlap resolution (`src/pii/overlap.rs`).** Detectors produce overlapping candidate spans;
`resolve_overlaps` reduces them to a non-overlapping set. Its governing rule is an **invariant,
not a ranking** (M4-R10 / M4-R11):

> **No structured span's bytes are ever abandoned.**

This replaced an earlier "highest priority wins" resolver that settled every overlap by **dropping
the whole loser span** — which silently left the loser's bytes *in clear*. A flat priority scalar can
only express "one of them wins"; it cannot express "**both** must be masked". On a **partial** overlap
(`555 867 5309john.doe@example.com` — the phone and the email share only `5309`) that meant whichever
side lost got forwarded unmasked, and re-tuning the priorities merely chose *which* PII leaked. Two
phases now:

1. **Structured union-merge.** Every group of transitively-overlapping structured spans collapses into
   its **union**. *Nothing structured is ever dropped*, so the invariant holds by construction. One
   sort-by-start sweep reaches the fixpoint. The union is masked as a single placeholder and restored
   verbatim, so the round-trip stays exact. It over-masks slightly (a bare `@domain` can land inside the
   placeholder) — the project's fail-safe direction: **over-mask, never leak**.
2. **NER greedy drop.** NER spans keep the whole-span drop (M2-R7): a `Person` overlapping a kept
   structured span is discarded entirely. Abandoning an *unstructured* remainder costs recall, never a leak.

**Naming the union** (`PiiKind::priority`: Secret > Iban > Card > Ssn ≈ NationalId > Phone > Email) —
the highest-priority **raw** candidate the union covers, *including one an email encloses* (M4-R15), so a
`Secret` glued to a phone isn't announced to the model as `[PHONE_1]` and the kind-only audit log doesn't
under-report it. **One exception:** when the union is *exactly* an `Email` span, the group is a genuine
email whose local part merely *looks* like a card or an ID (`4111111111111111@x.com`) — that enclosed
match is a false decomposition, not a second entity, so the email keeps the label.

| Shape | Overlap | Result |
|---|---|---|
| `4111111111111111@x.com` | the email **encloses** the card; union == the email span | one `[EMAIL_1]` — the card is a false decomposition of the local part |
| `4111 1111 1111 1111@x.com` | **partial** — a space-grouped card, and an email local part can't hold a space, so the email is only `1111@x.com` | one `[CARD_1]` over the **union** — card *and* trailing email masked |
| `555 867 5309john.doe@example.com` | **partial** — phone and email share `5309` | one `[PHONE_1]` over the union — the email's local part is **not** abandoned |
| `555 867 5309.sk-…@x.com` | **partial** phone + an email enclosing a secret | one `[SECRET_1]` — the union is named by the highest-priority raw candidate it covers |

Enclosure is a **naming** rule, not a deletion. (An earlier revision *deleted* the enclosed span before
ranking the rest — which stranded it in clear whenever the enclosing email then lost, M4-R10. The union is
identical either way, since an enclosed span merges *into* its enclosing email; expressing it as naming
keeps the behaviour and removes the trap.) The invariant is pinned by a property test (**PROP-03**,
`every_structured_candidate_byte_is_covered`): every raw structured candidate must be fully covered by
some resolved span. A winner-picking resolver cannot satisfy a per-byte invariant — which is exactly why
the two earlier priority-only fixes each passed their own case while leaking the other side.

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
- **Masking runs on the blocking pool, not on a tokio worker (M4-R19).** Detection is
  **CPU-bound** — regex scans over every text field, plus NER inference when it's on — and
  it sits on an **unauthenticated** path (it precedes any upstream auth). Run inline, a
  handful of concurrent large bodies would starve the executor and the whole proxy would
  stop serving, so the request-stage loop goes through `tokio::task::spawn_blocking`. If
  that task itself dies (a panic in a stage), the request is **blocked**, never forwarded:
  we'd be holding a body whose PII status is unknown. This bounds the *blast radius*; the
  actual cost is bounded by keeping detection **linear** (above).
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
| `src/pii/overlap.rs` | shared span overlap resolution — the *no-abandoned-bytes* invariant: structured union-merge → NER drop (`PiiKind::priority` only *labels* a union; enclosure by an email is a **naming** rule, `name_of` — the containment *gate* was removed in M4-R15) |
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
- **Coverage (M4, supersedes the original "IT + US")** — three tiers; see *Hybrid
  detection → Locale coverage* above. The NER's domain is its **model's** 10 languages;
  structured PII is language-independent and always on. `PII_LOCALES` gates only
  *FP-prone* recognizers — of which there are none yet, so it is a documented **no-op**.
- **Over-mask, never leak** — the standing tie-breaker. Where precision and recall
  conflict, recall wins: the pure-numeric national IDs accept ~18% of arbitrary 9-digit
  tokens (M4-R6) and a union may swallow a bare `@domain`. Both are **accepted on
  purpose**. The precision path is *context* (GLiNER, Backlog), never a keyword gate —
  gating a recognizer on nearby words reintroduces leaks.
- **Resolved (M1)**: the `Stage` signature threads a per-request `RequestContext`
  (carrying the `Vault`) from request to response.
- **Resolved (M1.5)**: the scanned text fields are fixed — see *Robustness &
  fail-closed → Field coverage* above.
- **Open (M4-R19, BLOCKER)**: candidate generation is **O(n²)** on the two
  unbounded-length recognizers (Email, Secret), and masking runs **inline** on the tokio
  worker with no per-field size cap — an unauthenticated DoS. See
  [`reviews/M4.md`](reviews/M4.md#m4-r19).

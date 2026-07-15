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

**Masking must be linear in the entity *count*, not just the field *size* (M4-R24).** These are **two
independent dimensions**, and closing one says nothing about the other. `Vault::mask` used to splice
placeholders in right-to-left with `String::replace_range`; each splice memmoves the whole tail, so *k*
entities in *n* bytes shift Θ(n·k) bytes — and a field of many **small** values (`a@b.co `, an SSN, a
phone) has *k* growing with *n*, so it is **Θ(n²) all over again**, on the same unauthenticated path.
13 MiB of repeated emails burned **~7 minutes**; the *same* 13 MiB as **one** giant email masked in
219 ms. Detection being linear does not bound the splice — it is a separate cost, which is exactly why
M4-R19 closed without touching it. `mask` now makes **one left-to-right copy into a fresh, capacity-
reserved buffer** — O(n + k), each byte touched once. Placeholder numbering is unaffected: it follows the
entities in start order, which splice direction never determined.

> **The rule:** the complexity guards must vary the entity **count**, not just the field **size**.
> DOS-01…03 each pin a *single* entity, so a per-entity quadratic lived right underneath them for four
> milestones — the *"a corpus has a shape, and that shape is a blind spot"* lesson (M4-R13) recurring on
> the DoS guards themselves. **DOS-04** is the many-entity guard.
>
> **Scope, stated honestly:** what is linear on both axes is the **structured** (default-build) masking
> path — detect → resolve → splice → de-mask. The optional `onnx` NER is a separate, opt-in cost with its
> own scaling behavior — chunked and measured linear (M5, PERF-01; see *Hybrid detection* below), not
> covered by this section's guards.

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

> **…but that proof covers the *recognizers*, not the NER — and a model swap must re-check it (M5-R4).**
> "A placeholder is inert" is proved **by construction** for the deterministic layer: `[KIND_N]` has no
> `@`, no `sk-`, nowhere near enough digits, and `[` / `]` sit outside every pattern's character classes.
> But `mask_all` runs the **`CompositeDetector`**, and the ML NER inside it is under no such constraint —
> nothing *structurally* stops a model from tagging `[PERSON_1]`, or a dense run of placeholders, as a
> `Person`/`Organization`. If one did, a pass would mask a placeholder, the text would not strictly shrink,
> `MAX_MASK_PASSES` would exhaust, and the request would **400** — fail-*closed* (M4-R20 saw to that), so
> **never a leak**, but a hard availability failure on ordinary input.
>
> For the current model it holds, and it is **measured, not assumed**: XLM-R int8 tags **zero** entities on
> placeholder-only text, and the full hybrid `mask_all` converges (`tests/ner_perf.rs`,
> `m5_r4_the_ner_treats_placeholders_as_inert`). That makes placeholder inertness an **empirical property of
> the chosen model**, and therefore a **model-swap checkpoint** — one that matters more than it sounds,
> because the Backlog's designated successor is **GLiNER**: a *zero-shot, open-label, **context**-driven*
> span extractor, i.e. precisely the kind of model that could look at `Contact [PERSON_1] at [ORG_1]` and
> tag both. **A new NER model must re-run that guard before it ships.**
>
> M5 is also what made this *reachable at scale*: before chunking, a field over ~500 tokens never reached
> the NER at all (it errored). Chunking now routes exactly the large, placeholder-dense fields through it.

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
  we'd be holding a body whose PII status is unknown. This bounds the *blast radius* (the
  async executor survives; `/healthz` still answers) — it does **not** bound the *cost*. The
  cost is bounded separately, by keeping the structured masking path linear on **both** of
  its axes: field size (M4-R19) **and** entity count (M4-R24). Both were quadratic, and the
  blocking pool is finite and shared, so either one alone was enough to stop the proxy
  masking — hence serving — for everyone. Measured end-to-end: a **13.4 MiB** body of ~2 M
  small emails now masks in **1.8 s** (it took ~7 minutes), and eight of them concurrently
  finish in 4.3 s while `/healthz` answers in 48 ms.
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

  > **The rule that governs it (M5-R7): a detector may degrade its own recall, but it may never
  > decide *for the caller* that degraded output is acceptable.** Fail-open vs fail-closed is
  > `FailOpen`'s decision, and the **only** road to it is the `try_detect` error channel. A detector
  > that quietly returns `Ok(partial)` where it could have returned `Err` has routed *around* the
  > switch the operator set — it silently converts `NER_REQUIRED` (block) into "forward a
  > partially-scanned body", which is the one thing that operator paid to prevent. This is not
  > hypothetical: the first M5-R2 fix did exactly that, clamping an over-long NER sequence and
  > returning `Ok`. The clamp was *better* for the default posture and *fatal* to the other one —
  > and a component that cannot see the posture must not choose between them. **When in doubt, a
  > detector returns `Err` and lets the wrapper decide.**

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
surface). The request **body** is identical across the presets; only the HTTP **envelope**
differs, and that is all a preset touches. A `UPSTREAM_PROVIDER` preset (`openai` / `copilot` / `anthropic`) sets the
per-provider *shape* — the chat path (`upstream_chat_path`; Copilot drops `/v1`), the
allowlist of client request headers to pass through (`forward_request_headers`, e.g.
`anthropic-version` or editor headers), and any required static headers
(`upstream_extra_headers`) — each overridable by env. Base URL + API key stay
env-driven. Auth: the client's own `Authorization` wins, else the configured key as
`Bearer`. Anthropic's *native* `/v1/messages` schema is **not** served here — that is the
inbound work of **[M6](ROADMAP.md#m6)** (the rest of Option B stays Backlog); see *The
wire-format boundary* next.

## The wire-format boundary — who speaks what to whom

The single most confusing question about this proxy — *"which providers does it work
with?"* — has a one-line answer once the axis is named: **the proxy speaks exactly one
wire format, the OpenAI Chat Completions schema (`POST /v1/chat/completions`), on *both*
hops.** That is the whole point of Option A above — one schema feeds the masker, so there
is a single shape to get right and no translation layer to leak through.

```
          OpenAI Chat Completions                        OpenAI Chat Completions
  client ───────────────────────────►  PROXY  ───────────────────────────►  upstream provider
  (OpenAI-compatible caller)          mask / restore     (any endpoint speaking that schema)
```

So *"which providers"* is really **two** independent questions, and conflating them is the
confusion:

- **Upstream — what the proxy forwards *to*.** Any endpoint that accepts the OpenAI Chat
  Completions schema. Presets (`UPSTREAM_PROVIDER`): `openai`, `copilot`, and
  **`anthropic` — via Anthropic's *OpenAI-compatible* endpoint, not its native API** —
  plus anything else through `UPSTREAM_BASE_URL` (local models behind Ollama / vLLM /
  LM Studio, Groq, Mistral, …). The preset only sets the *shape* (path, forwarded
  headers); the masking is identical for all.
- **Client — what speaks *to* the proxy.** Any OpenAI-compatible client: the OpenAI SDK,
  `curl` with OpenAI JSON, editor agents that target `/v1/chat/completions` (Cline,
  Continue), …

**The exception that trips everyone up:** *"works with Anthropic"* (as an **upstream**) is
true; *"works with Claude Code"* (as a **client**) is not — and they are not the same
statement. Claude Code is **not** an OpenAI client: it speaks Anthropic's **native**
Messages API (`POST /v1/messages`, content blocks, `tool_use`/`tool_result`), a path this
proxy does not serve, so it 404s (fail-closed). Serving that native *inbound* schema — so a
native client like Claude Code can be masked — is **[M6](ROADMAP.md#m6)**.

> **Why this framing is worth keeping in mind when planning work.** A feature request lands
> on exactly one of the two axes. *"Support provider X"* is usually **upstream** work — a
> preset, trivial when X is OpenAI-compatible. *"Support client Y"* is **inbound** work, and
> if Y speaks a native protocol it means **a new schema on the masking path** — the
> expensive, leak-sensitive kind (M6, and the remainder of Option B). Name the axis first,
> and the size and risk of the change follow.

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

**NER chunking (M5, PERF-01).** `OnnxNerDetector` tokenizes a field once; if it fits, it runs
exactly as before (M2). A field that doesn't is split into overlapping token windows
(`chunk_char_ranges`, a pure function unit-tested without a model), each **re-tokenized
independently** — a middle window needs its own `<s>…</s>` framing, so it can't be a raw slice
of the whole field's token ids — and run through the same single-window path; results are
merged, sorted, and exact duplicates from the overlap region deduped. Without this, an
oversized field didn't just run slowly: it made the ONNX call **error outright** (measured —
see *Decisions & open points* above), silently downgrading NER coverage by default or
**blocking every such request** under `NER_REQUIRED`. Chunking is a **recall** mechanism only,
never leak-relevant: structured PII is detected independently, over the whole field, and is
never chunked.

**Two constants, and the difference between them is the whole point (M5-R2).**

| constant | bounds | value |
|---|---|---|
| `MAX_WINDOW_TOKENS` | the **planning window** — how much of the *field's* tokenization one chunk covers | 480 |
| `MODEL_MAX_TOKENS` | the **model's usable sequence length** — what may actually be handed to the session | 512 |

They are not two names for one budget. A window is planned in the coordinates of the *whole
field's* tokenization, but then **re-tokenized from its own text**, which adds the two special
tokens and drifts at the cut edges — so the sequence that reaches the model is `window +
specials + drift`, **measured at 481–483, i.e. always over the planning bound**. `MODEL_MAX_TOKENS`
is 512 because XLM-R declares `max_position_embeddings: 514` but RoBERTa-family position ids
start at `pad_token_id + 1 = 2`. The 32-token gap is the drift headroom, and **the headroom
itself** — not the mere ordering — is the **compile-time invariant** (M5-R10):

```rust
const MIN_DRIFT_HEADROOM_TOKENS: usize = 16;
const _: () = assert!(MODEL_MAX_TOKENS - MAX_WINDOW_TOKENS >= MIN_DRIFT_HEADROOM_TOKENS);
```

Get it wrong and the crate does not build — including a window *over* the ceiling, which underflows
the const subtraction. This is the constraint the chunker actually relies on: `479 < 512` and
`511 < 512` satisfy `<` identically, yet a 511-token window re-tokenizes to ~514 and the `Expand`
error is back. The drift has "nowhere to go" the moment the headroom drops below it, not the moment
the window reaches the ceiling — so it is the **headroom** that must be pinned.

> **This is what two earlier wordings got wrong, and the pair is worth keeping as a warning.**
> First (M5-R2) this section said the window was *"conservatively under `max_position_embeddings`"* —
> a single budget, assumed safe; the window *was* under the limit and the **sequence was not**, on
> every chunk. Then the fix's own guard asserted only `MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS` (M5-R10)
> — true at any headroom ≥ 1, so it approved a 511-token window that overflows. **A bound you do not
> check is not a bound — and a compile-time invariant must encode the constraint the code relies on,
> not a weaker one that happens to hold at today's values.** `A < B` is not "A leaves room for
> drift"; and when the invariant is the *only* guard a modelless CI can run, the gap between those
> two is the whole exposure.

**The ceiling is checked at one choke point, and overflow is an `Err`, not a clamp (M5-R7).**
`run_and_decode` is the only path into the ONNX session (the direct call *and* every chunk), and
it **rejects** an over-long sequence rather than truncating it. An earlier revision clamped and
returned `Ok(partial)` — losing a window's tail instead of the whole field, which sounds strictly
better and is the wrong call: *whether a degraded NER is acceptable is a **posture** decision*,
and this codebase already has exactly one owner for it — the `FailOpen` wrapper and the
`try_detect` error channel (M2-R1/R2). Fail-open (default) swallows the error and proceeds
structured-only; **`NER_REQUIRED` unwraps the detector, so the error blocks the request (400)**,
which is what that operator asked for. Clamping returned `Ok` in *both* postures, quietly
forwarding a partially-scanned field to someone who had explicitly demanded a block — the failure
**relocated**, not closed. (The general rule this taught is in *Robustness & fail-closed* above.)
The error path is latent by design; `tests/ner_perf.rs`
(`m5_r2_…within_the_models_usable_length`) is the guard that keeps it that way.

**The one assumption the chunker rests on, stated (M5-R8).** `chunk_char_ranges` reads the
tokenizer's per-token offsets, and it takes the `(0, 0)` offset — the sentinel the tokenizer emits
for a **special token** — to mean *"this is `</s>`, i.e. the sequence end"*, which is why a window
reaching the last token uses `input.len()` rather than that offset (getting this wrong silently
dropped the whole final window; see M5-R2's history). That is sound **only because `(0, 0)`
appears exclusively at the sequence *ends***. A mid-sequence `(0, 0)` would either collapse a
window to zero length (silently unscanned) or restart it at byte 0. Verified against the real
XLM-R tokenizer over 17 adversarial inputs — CJK, combining marks, zalgo, emoji/ZWJ, zero-width
and control characters, 20 K-char single tokens, base64 runs, literal `<s>`/`<unk>` text: zero
mid-sequence sentinels. **A tokenizer that emitted one would break chunking, so a tokenizer swap
must re-check it** — the same class of model-swap checkpoint as placeholder inertness above.

> **Boundary fragments are cleaned up by the *resolver*, not by the `dedup()` — and that is
> load-bearing.** `infer_chunked`'s `dedup()` removes only **exact** duplicates. A window that
> cuts an entity in half emits a *truncated* one (`Mil` where the neighbouring window sees
> `Milan`); that is a different span, so it survives the dedup. What removes it is
> `overlap::resolve_overlaps`' NER phase (see *Overlap resolution* above): all three NER kinds share
> `PiiKind::priority() == 0`, so the phase tiebreaks on **span length, descending** — it takes the
> whole entity first and drops the overlapping fragment. **So the correctness of chunking depends
> on the NER kinds staying at equal priority.** Rank `Person` above `Location` and a truncated
> boundary fragment could outrank the entity it was cut from. (The window/stride arithmetic bounds
> how often this even arises: 480-token windows on a 448-token stride, so any entity ≤ 32 tokens is
> whole in at least one window. A longer one split across both windows is a *recall* miss — the
> OVL-02 / M2-R7 class, accepted for the best-effort NER layer.)

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

## Supply-chain & dependency security

A privacy proxy inherits the vulnerabilities of everything it links, so the dependency surface is scanned
automatically — not by hope. Two independent layers, both free on a public repo, deliberately kept **separate
from the build CI** (this is cheap and needs its own cadence):

- **`cargo-deny`** (`.github/workflows/security.yml` + `deny.toml`) runs **`check advisories bans sources`**
  against the **RustSec** advisory DB — on PR / push that touch the dependency manifests, on a **weekly
  schedule** (the important trigger: a CVE can be disclosed against a dependency you *already* have, with no
  code change to fire a run), and on demand. `advisories` is the security core; `sources` pins crates to
  crates.io (an unknown git/registry host is the shape a supply-chain attack takes — fail closed); `bans`
  flags duplicate versions. It reads `Cargo.lock` (no compile), so it stays fast. The action's default cargo
  (1.71) can't parse this tree — it pulls crates needing `edition2024` — so the workflow sets
  `rust-version: stable` (cargo-deny needs a cargo new enough to read the graph, ≥ 1.85, not the project MSRV).
  - **`licenses` is off the gating command on purpose.** It is *compliance*, not *security*, and noisy against
    the `onnx` crypto stack (`ring` / `aws-lc-sys` carry non-SPDX license refs). A ready-to-enable allow-list
    sits commented in `deny.toml` (this crate is AGPL-3.0-or-later) for when compliance — not vulnerability —
    is the actual question.
- **Dependabot** (`.github/dependabot.yml`) keeps the `cargo` and `github-actions` ecosystems patched (grouped
  weekly to limit PR noise). Its vulnerability-driven side — **alerts + security updates**, from the GitHub
  Advisory DB — is a separate repo toggle the maintainer enables in *Settings → Code security & analysis*.

GitHub's native **code scanning (CodeQL — Rust is GA since 2025-10), secret scanning + push protection** are
the maintainer's one-click complement in that same Settings pane; also free for public repos. cargo-deny and
Dependabot overlap on advisories but do not duplicate: cargo-deny keys on the Rust-native **RustSec** DB and
adds source/ban policy GitHub does not, while Dependabot keys on the **GitHub Advisory** DB and opens the fix PRs.

**Workflow tokens run least-privilege.** Every Actions workflow declares an explicit `permissions:` block —
read-only by default, with `contents: write` granted *only* to the release **publish** job — so the
`GITHUB_TOKEN` never inherits the repo's (possibly read-write) default. This is CodeQL's
`actions/missing-workflow-permissions` closed as a standing rule rather than a one-off.

**PRs are gated for correctness, too.** `ci.yml` runs fmt / clippy / test (both `default` and `--features
onnx`) plus an MSRV `cargo check` on every push and PR — lightweight, one platform, **no cross-compile** — so
a dependency bump (Dependabot's especially) that fails to build or breaks a test cannot be merged green. The
all-target release build stays tag/manual-only (`release-build.yml`). This is why Dependabot version updates
are safe to run: the gate, not vigilance, catches a breaking bump — including the 0.x "minor" bumps Dependabot
cannot recognise as breaking.

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
- **Resolved (M4-R19)**: candidate generation was **O(n²)** on the two unbounded-length
  recognizers (Email, Secret) and masking ran **inline** on the tokio worker. Fixed: the
  overlap rescan is now bounded-patterns-only (unbounded ones rely on the fixpoint), and
  masking runs on `spawn_blocking`. Detection is linear, verified by `tests/complexity.rs`.
- **Resolved (M4-R24)**: masking was *still* **O(n²)**, but in the **entity count**, not the
  field length — `Vault::mask`'s right-to-left `replace_range` splice shifted the tail once
  per entity (13 MiB of many small values ≈ 7 min of CPU). The splice is now a single
  left-to-right copy, O(n + k), and **DOS-04** guards it by varying the entity *count* — the
  axis DOS-01…03 held fixed at one. The structured masking path is now linear on **both**
  axes, measured.
- **Resolved (M5, PERF-01)**: `OnnxNerDetector` used to feed the **whole field as one
  sequence**, and the failure mode was not the suspected "quadratic self-attention" — it was
  **worse and simpler**: RoBERTa-family absolute position embeddings top out at
  `max_position_embeddings` (514 for the picked XLM-R int8), so a sequence past that limit made
  the ONNX graph's position-embedding lookup go **out of range** — measured (`tests/ner_perf.rs`)
  as an outright `Expand` op failure, not a graceful slowdown. Any field over roughly 2 KB of
  prose (~500 tokens) failed NER entirely: silently downgraded to structured-only under the
  default fail-*open* wrapper, but a hard **block** under `NER_REQUIRED` (every such request
  would 400) — an availability gap in the same family as M4-R19/R24, though opt-in and off by
  default so never the unauthenticated DoS those were. Fixed: `OnnxNerDetector::infer` now
  splits an oversized field into overlapping token windows
  (`chunk_char_ranges` + `infer_chunked`, `src/pii/onnx.rs`), each independently tokenized (its
  own `<s>…</s>` framing) and run under the sequence budget, with results merged and
  deduplicated. Measured linear scaling: 64/256/1024 repeated-sentence multiples run in
  448 ms / 2.07 s / 7.53 s (debug profile), recall intact (≥99.6% of expected entities — the
  small excess above 100% is an occasional un-deduped near-boundary double-detection, a
  precision nit, not a recall loss). This is a **recall** mechanism only: structured PII (the
  fail-closed layer) is detected independently over the whole field and is never chunked, so
  this changes NER coverage/availability, never masking correctness.

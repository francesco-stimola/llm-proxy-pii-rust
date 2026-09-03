# Roadmap

Development is split into milestones. Each builds on the previous one and is independently testable.
**This document is the single source of truth for "what's next"** — keep the checkboxes current as work
lands.

<a id="conventions"></a>
**How to read this file.** Each milestone lists its **scope** as checkboxes. Review findings are recorded
as a compact **ledger** — id · title · severity · status — and nothing more; the finding itself, its fix,
its tests and its lessons live in **[`docs/reviews/`](reviews/)**, one file per milestone. A finding has
**one home, for its whole life**: closing it flips a box here and appends a closure note there — it is
never copied or moved. So this file stays forward-looking, and the record stays complete.

Design conclusions that outlive a review are **promoted** into [`ARCHITECTURE.md`](ARCHITECTURE.md) (the
invariants) and [`TESTING.md`](TESTING.md) (the guards) — read *those* to understand the system.
[`DEVLOG.md`](DEVLOG.md) is the chronological build history.

---

## Status

This table is the whole backlog at a glance — **one row per milestone**, each linking to its section below.
The Status column is a **single label** — ✅ complete · 🔨 code-complete · 📋 planned — and nothing more:
findings, counts, dates and closure notes live in each milestone's section and in [`reviews/`](reviews/), so
this table can't drift. A **release tag** is noted next to the milestone it was cut from, never as its own row.

**A tag that isn't cut yet is written `tag `vX.Y.Z` (planned)`.** That exists so the table can always
name the version it is heading for without asserting something false — the alternative, leaving it out
until the tag exists, is what let `v1.2.0` be cut once with the docs silent about it. Drop `(planned)`
when the tag is real. This is **enforced, not remembered**: `release-build-publish.yml` refuses to
publish a `v*` tag the Status table does not name as cut, so a release the docs cannot explain cannot
happen. The guard reads the **table rows only** — prose like this sentence must never be able to move
it, which a first dry run proved was not a hypothetical.

| Milestone | Status |
|---|---|
| [M0 — Project setup](#m0) | ✅ complete |
| [M1 — Structured PII pipeline](#m1) | ✅ complete |
| [M1.5 — Robustness & fail-closed](#m15) | ✅ complete |
| [M2 — Unstructured entities (ONNX NER)](#m2) | ✅ complete |
| [M2.5 — HuggingFace model management](#m25) | ✅ complete |
| [M2.6 — Debug & observability modes](#m26) | ✅ complete |
| [M3 — Streaming & multi-provider routing](#m3) | ✅ complete |
| [M4 — Broad locale & language coverage](#m4) | ✅ complete |
| [M5 — Integration & performance testing](#m5) | ✅ complete · tag `v0.4.0` (interim badge) |
| [M6 — Native Anthropic `/v1/messages`](#m6) | ✅ complete |
| [M7 — NER latency](#m7) | ✅ complete |
| [M7.1 — system-prompt cache + fixpoint NER fix](#m71) | ✅ complete · tag `v1.0.0` |
| [M8 — GLiNER: contextual / open-label PII](#m8) | ✅ complete |
| [M8.1 — national phone recognizer (opt-in)](#m81) | ✅ complete · tag `v1.1.0` |
| [M9 — GPU optimization](#m9) | ✅ complete |
| [M9.1 — one release binary per backend](#m91) | ✅ complete · tag `v1.2.0` |
| [M10 — national phone coverage + release hygiene](#m10) | ✅ complete · tag `v1.2.1` |
| [M11 — deterministic coverage · NER thread base](#m11) | 🔨 code-complete · tag `v1.3.0` (planned) |
| [M12 — one model for everything not provable](#m12) | 📋 planned |

---

<a id="m0"></a>
## M0 — Project setup ✅
- [x] Repo, license (**AGPL-3.0-or-later** — relicensed from MIT 2026-07-13), README (EN) + README.it.md (IT)
- [x] Port PII test cases from the old proxy → `tests/reference/old-proxy/`
- [x] Tracking docs (ROADMAP, ARCHITECTURE, TESTING, SETUP, DEVLOG)
- [x] Rust module scaffold; `cargo build` green (177 deps)
- [x] Rust toolchain (rustup, per-user, no admin) + MSVC linker via portable Build Tools → `docs/SETUP.md`
- [x] Decisions locked: hybrid detection, CPU-first, `[KIND_N]` placeholders, IT + US *(locales superseded by M4)*

<a id="m1"></a>
## M1 — Structured PII pipeline (CPU, no ML) ✅

### Part A — Masking core ✅
- [x] `PiiDetector` trait + `PiiEntity` / `PiiKind` types (incl. `Secret`)
- [x] Deterministic recognizers: email, phone (IT/US), SSN, credit card (Luhn), IBAN — with priority-based overlap resolution *(the resolver was rewritten around an invariant in M4 — see [ARCHITECTURE](ARCHITECTURE.md))*
- [x] SECRET recognizer (`sk-…`, `sk-ant-…`, `AKIA…`) — deterministic; the old ML model missed these
- [x] `Vault`: mask to `[KIND_N]` + exact-restore demask, deterministic per value
- [x] Privacy stage wired into the pipeline (per-request `RequestContext` threads the `Vault` request → response)
- [x] axum server forwarding `/v1/chat/completions` upstream (non-streaming), `reqwest` client, `/healthz`, config from env
- [x] Reference tests ported; no false positives; multi-PII round-trip exact (corpus-driven + proptest)

### Part B — Prompt augmentation & round-trip ⭐ primary feature ✅
Make the masked data actually *usable* by the model — without this it mishandles placeholders, especially
in tool calls. A headline capability, not a nice-to-have.
- [x] Transparent system-prompt injection: `[KIND_N]` are typed real values, to be used verbatim (incl. tool-call arguments) and never altered
- [x] Round-trip covers `tool_calls` arguments (de-anon in responses) and tool results (re-anon in requests)
- [x] Deterministic placeholder assignment (same value → same token across turns)
- [x] Integration tests INT-01…06 (+ E2E-01/03 against a mock upstream); binary smoke test (E2E-BIN)

<a id="m15"></a>
## M1.5 — Robustness & fail-closed 🔒 ✅
For a privacy tool the failure mode *is* the product: when anything is unexpected, it must **fail closed**
(block / scrub), never forward raw PII. Hardens M1 before detection broadens. Full model in
[ARCHITECTURE → *Robustness & fail-closed*](ARCHITECTURE.md).
- [x] **Fail-closed policy** — an unreadable `content` shape or missing `messages` sets `RequestContext.block` → 400, never forwarded. Unproxied paths → 404
- [x] **Full field coverage** — `content` (string + array `text` parts, all roles), `name`, `tool_calls[].function.arguments`, legacy `function_call.arguments`, `tools[].function.description` + every nested `description`
- [x] **API scope** — chat/completions only is proxied; `/healthz` served; everything else 404s
- [x] **Adversarial / evasion tests** (`tests/adversarial.rs` + corpus); obfuscated emails pinned as a documented recall gap for NER
- [x] **Body-size limit** — `MAX_BODY_BYTES` (16 MiB) via `DefaultBodyLimit`
- [x] **Tolerant de-masking** — one pass restores `[EMAIL 1]` / `[email-1]` / `[ EMAIL_1 ]`; an unresolved known-kind placeholder is logged, never silently shipped

### Review ledger — M1 / M1.5 → [`reviews/M1.md`](reviews/M1.md)
Eight findings, **all closed** (`bb68707`, `775945f`). They pre-date the `M<n>-R<n>` scheme, so they carry
no ids: IBAN over-match · `iban_mod97` unwired · duplicate `system` message · response-header allowlist ·
tolerant de-mask (CR-4) · **array-content fail-closed gap (a leak)** · phone over-match · `Confidence`
write-only.

<a id="m2"></a>
## M2 — Unstructured entities (ONNX NER, CPU) ✅
Names / organizations / locations via a local ML model. Candidates & method:
[`M2-NER-EVALUATION.md`](M2-NER-EVALUATION.md).

**Model picked by measurement: XLM-R int8** (`jiting/xlm-roberta-base-ner-hrl_onnx` @ `478a2a3`) —
hybrid Org 1.00 / Loc 1.00 / Person 0.75, ~23 ms/case, 266 MB, multilingual. **Piiranha rejected** (~0
recall on natural sentences — it is a form/structured-PII model, and has no Organization label). Numbers
in [`DEVLOG.md`](DEVLOG.md) (2026-07-12).

- [x] `OnnxNerDetector` behind the `onnx` feature (CPU EP) — HF tokenizer + `ort` session **pool** (`NER_POOL_SIZE`), per-token argmax → BIO decode
- [x] Eval harness `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d) — scores a live model against `ner_cases.json` **through the hybrid resolver**
- [x] Evaluate the candidates head-to-head and pick one — done, measured
- [x] `CompositeDetector` — fan out to N detectors, merge via the shared `overlap::resolve_overlaps`
- [x] Extend the corpus with unstructured-entity cases — `tests/corpus/ner_cases.json`
- [x] Symmetric response de-masking (`demask_content` mirrors `mask_content`)
- [x] **Fail-closed for NER** — `NER_REQUIRED` + the `PiiDetector::try_detect` error channel; unset (default) = an explicit `FailOpen` wrapper keeps the fail-open posture for names

### Review ledger — M2 → [`reviews/M2.md`](reviews/M2.md)
**All 10 closed** (`244b49e`, `1e473ce`, `889dfea`).

| ID | Title | Status |
|---|---|---|
| [M2-R1/R2](reviews/M2.md#m2-r1) | The NER could fail silently → `try_detect` + `NER_REQUIRED` + `FailOpen` | [x] |
| [M2-R3](reviews/M2.md#m2-r3) | Label/logit count mismatch undetected | [x] |
| [M2-R4](reviews/M2.md#m2-r4) | `outputs["logits"]` panicked; `token_type_ids` unsupported | [x] |
| [M2-R5](reviews/M2.md#m2-r5) | `B_LOC`-style labels didn't split adjacent entities | [x] |
| [M2-R6](reviews/M2.md#m2-r6) | Off-boundary token offset silently dropped (→ the SentencePiece leading-space trim) | [x] |
| [M2-R7](reviews/M2.md#m2-r7) | NER whole-span drop undocumented — *recall, never a leak* (still the rule) | [x] |
| [M2-R8](reviews/M2.md#m2-r8) | `DetectError` could carry input text | [x] |
| [M2-R9](reviews/M2.md#m2-r9) | A poisoned mutex killed the session pool | [x] |
| [M2-R10](reviews/M2.md#m2-r10) | Eval harness counted membership, not multiset | [x] |

<a id="m25"></a>
## M2.5 — HuggingFace model management (auto-download via `hf-hub`) ✅
Ergonomics + reproducibility, **not a correctness gap**. The `hf-hub` crate (behind `onnx`) fetches the
**revision-pinned** model into the *standard* HF cache — library-managed, deduped, reproducible.
Implemented in `src/pii/hf.rs`, wired in `server.rs::load_onnx_ner`.
- [x] `HfModelSpec::resolve` → `(repo, revision, filename)` → a local cache path. **`NER_MODEL_PATH` (explicit local file) keeps priority** — set it for zero outbound calls
- [x] **Opt-in** auto-download via `NER_MODEL_REPO`; tunables `NER_MODEL_REVISION` (default `478a2a3`), `NER_MODEL_FILE`, `NER_TOKENIZER_FILE`, `NER_CONFIG_FILE`. One-time **model** fetch (not user data) — logged, never silent
- [x] Derive `NER_LABELS` from `config.json` `id2label` (class-id order, contiguity-checked → fail closed); explicit `NER_LABELS` overrides
- [x] Pin the standard cache — `hf-hub` 1.0 falls back to `/tmp/.cache` on Windows (no `HOME`), so `build_client` sets `<home>/.cache/huggingface/hub` unless `HF_HOME`/`HF_HUB_CACHE` say otherwise
- [x] Docs: env contract + privacy note (ARCHITECTURE / SETUP)
- [x] Tests without network (`parse_id2label`, `standard_hub_cache_dir`); the real download is exercised only by the `#[ignore]`d eval harness

### Review ledger — M2.5 → [`reviews/M2.5.md`](reviews/M2.5.md)
**Both closed** (`fe8fbc1`, `b97bb72`).

| ID | Title | Status |
|---|---|---|
| [M2.5-R1](reviews/M2.5.md#m25-r1) | The "no new native deps" claim was false → **decided: keep `hf-hub` 1.0** (Option A) + the `DEP-01` footprint guard | [x] |
| [M2.5-R2](reviews/M2.5.md#m25-r2) | Empty `id2label` returned `Ok(vec![])` instead of failing closed | [x] |

> **The rule that came out of it:** the **default build is native-dep-free**, and that is now enforced by
> `tests/dependency_footprint.rs`, not by a comment. The whole ONNX/HF stack stays behind the `onnx`
> feature.

<a id="m26"></a>
## M2.6 — Debug & observability modes 🔍 ✅
Opt-in developer tools to confirm, with your own eyes, that masking holds end-to-end. **Off by default**;
neither weakens fail-closed (request-side masking always runs).
- [x] **`PII_DEBUG_SKIP_DEMASK`** — skip the response de-mask so the client receives the **placeholders** the provider saw. On `Config`, not a bare env read (isolated + testable); **loud `warn!`** at startup
- [x] **`trace!` of the masked upstream body** — the exact bytes sent upstream, logged just before forwarding (masked at that point, so safe). `debug!` keeps the concise kind-only audit lines
- [x] **Safety boundary (design rule) — upheld:** the masked request and the raw provider response (placeholders only) are safe to log; the **final de-masked client output is NEVER logged**

### Review ledger — M2.6 → [`reviews/M2.6.md`](reviews/M2.6.md)
Sound, no blockers. **Both hardening nits closed** (`b97bb72`).

| ID | Title | Status |
|---|---|---|
| [nit 1](reviews/M2.6.md#m26-n1) | Never-log-raw-PII was verified by *inspection*, not by a test → `tests/log_safety.rs` (`DBG-02`) | [x] |
| [nit 2](reviews/M2.6.md#m26-n2) | Three copies of the env-flag parser (one gates a fail-closed switch) → one `config::env_flag` | [x] |

<a id="m3"></a>
## M3 — Streaming & multi-provider routing (Option A) ✅
SSE token streaming with incremental de-anonymization **and** routing to multiple providers via their
**OpenAI-compatible** endpoints. The two intertwine (real Copilot/Anthropic usage is streamed), so they
landed together. Streaming never weakens fail-closed: request-side masking runs first, so the provider only
ever sees placeholders.

### Streaming
- [x] Streaming passthrough (`stream:true` → `Body::from_stream`, `text/event-stream`); a clean request streams through untouched
- [x] **Hold-back buffer** for placeholders split across chunks — `SseDemasker` keeps one buffer per streamed field (`Content{choice}` **and** `ToolArg{choice,tool}`), emitting up to the last point that could still be an incomplete placeholder
- [x] Non-SSE upstream response → buffered fallback (M3-R1); mid-stream upstream error → terminal `event: error`, clean end

### Multi-provider routing — Option A (OpenAI-compat normalization)
Route every provider through its OpenAI-compatible endpoint so a **single schema** feeds the masker — no
new leak surface. *(A per-schema masker is Option B — Backlog.)*
- [x] `UPSTREAM_PROVIDER` preset (`openai` / `copilot` / `anthropic`) → chat path, request-header allowlist, static extra headers. Base URL + key stay env-driven
- [x] Per-provider auth (client `Authorization` wins, else the configured key as `Bearer`) and overrides (`UPSTREAM_CHAT_PATH` / `UPSTREAM_FORWARD_HEADERS` / `UPSTREAM_EXTRA_HEADERS`)
- Anthropic's **native** `/v1/messages` schema is deliberately out of scope — Option B (Backlog)
- **Request-level provider routing** → moved to Backlog (per-instance already covers Copilot + Anthropic)

### Review ledger — M3 → [`reviews/M3.md`](reviews/M3.md)
Sound, no blockers. **Both closed** (`6e69522`, `eb4c1e6`).

| ID | Title | Status |
|---|---|---|
| [M3-R1](reviews/M3.md#m3-r1) | A `stream:true` request answered with JSON was force-wrapped as SSE | [x] |
| [M3-R2](reviews/M3.md#m3-r2) | A de-masked value containing `"` broke tool-call `arguments` JSON → `demask_json_string` | [x] |

<a id="m4"></a>
## M4 — Broad locale & language coverage ✅
Extend PII coverage beyond IT + US so the proxy protects data regardless of the user's language or the
upstream provider.

**Scope — the multilingual question, closed within a bounded domain, on two axes:**
- **Language (unstructured / NER):** declared support = the **NER model's** languages (XLM-R HRL: ar, de,
  en, es, fr, it, lv, nl, pt, zh). Beyond these we don't claim to catch names/orgs/locations — only
  structured PII, which is language-independent. *If the model changes, the domain moves with it.*
- **Structured PII — three tiers:** **universal** (email, IBAN, card, secret, phone US + `+CC`) always on;
  **national IDs** always on **regardless of `PII_LOCALES`** (privacy-first — [M4-R1](reviews/M4.md#m4-r1));
  **FP-prone** recognizers opt-in via `PII_LOCALES`. So **`PII_LOCALES` gates *ambiguous* recognizers, not
  "which countries"** — the FP-prone seam, empty at M4, now carries the GB/DE national-phone recognizers
  ([M8.1](#m81)); other codes remain a no-op until vetted.

- [x] Locale-parametrized recognizer architecture — universal / national-ID / FP-prone tiers (`with_locales`, `fp_prone_recognizers`)
- [x] **National-ID packs for all XLM-R-aligned countries**, always-on, each checksum- or rule-gated: US SSN (keeps `PiiKind::Ssn`) · IT Codice Fiscale · GB NINO · ES DNI/NIE · FR NIR · DE Steuer-ID · NL BSN · PT NIF · LV personal code · zh Resident ID. **`ar` gets no pack** — the language spans ~20 countries with different ID schemes, so there is no single "Arabic" national ID; Arabic names/locations stay covered by the NER
- [x] **IBAN per-country length validation** — `Verified` only when mod-97 **and** the ISO 13616 length both hold; a wrong-length IBAN of a known country is still masked, flagged `Structural`
- [x] **Validate the NER across its declared domain** — scored XLM-R int8 on all **10** languages: Person 0.83 / Org 1.00 / Loc 0.91 (per-language notes in DEVLOG 2026-07-13)
- [x] Extend the corpus with multi-language cases (`multilingual_preview`) **and non-ASCII structured cases** (`non_ascii_scripts` — see M4-R13)
- [x] Provider-agnostic verification — `e2e_masking_is_provider_agnostic`: `openai` vs `anthropic` presets → a byte-identical masked body upstream
- [x] **Locale phone national formats** → descoped from M4 (the FP-prone tier's first recognizer; the `+CC` arm already covers the unambiguous case), pointed at [M8](#m8) as a context problem — but **resolved in [M8.1](#m81)** by a *deterministic* path instead: the pure-Rust `phonenumber` crate's `is_valid()` validates a loose `0`-trunk candidate against the region's real numbering plan, the assigned-range check a hand-written regex can't do. GB/DE shipped opt-in via `PII_LOCALES`

### Review ledger — M4 → [`reviews/M4.md`](reviews/M4.md)
**Seven review rounds, 24 findings — all closed.** More than every other milestone combined, because
**M4-R7 → R9 → R10/R11 → R13 → R17 → R19 → R24 is one bug rediscovered again and again.** The
[retrospective](reviews/M4.md#retrospective) is the most useful page in this repo.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M4-R1](reviews/M4.md#m4-r1) | `PII_LOCALES` gated the national IDs → always-on, privacy-first | design | [x] |
| [M4-R2](reviews/M4.md#m4-r2) | GB NINO over-match (`PO123456A`) → HMRC prefix rules | precision | [x] |
| [M4-R3](reviews/M4.md#m4-r3) | IT Codice Fiscale had no check character | precision | [x] |
| [M4-R4](reviews/M4.md#m4-r4) | ARCHITECTURE mis-grouped US SSN under `NationalId` | docs | [x] |
| [M4-R5](reviews/M4.md#m4-r5) | FR NIR missed the INSEE special months (a **miss = a leak**) | recall | [x] |
| [M4-R6](reviews/M4.md#m4-r6) | Pure-numeric IDs over-mask ~18% of 9-digit tokens — **accepted**, documented, *not* context-gated | tradeoff | [x] |
| [M4-R7](reviews/M4.md#m4-r7) | An email whose local part is a card/IBAN/secret gets fragmented | precision | [x] |
| [M4-R9](reviews/M4.md#m4-r9) | …but M4-R7's fix **leaked a space-grouped card/IBAN** glued to a domain | **LEAK** | [x] |
| [M4-R10](reviews/M4.md#m4-r10) | …and M4-R9's containment gate **strands the span it deleted** (a `Secret` in clear) | **LEAK** | [x] |
| [M4-R11](reviews/M4.md#m4-r11) | …and M4-R9's demotion **abandons a real email's local part** | **LEAK** | [x] |
| [M4-R12](reviews/M4.md#m4-r12) | Three docs stated the superseded priority order as current | docs | [x] |
| [M4-R13](reviews/M4.md#m4-r13) | **Unicode `\b` made every recognizer INERT in CJK** — survived 4 reviews | **LEAK** | [x] |
| [M4-R14](reviews/M4.md#m4-r14) | The merge's one fallback degrades *toward* the leak | hardening | [x] |
| [M4-R15](reviews/M4.md#m4-r15) | A `Secret` announced as `[PHONE_1]` → union naming; **deleted the containment gate** | observ. | [x] |
| [M4-R16](reviews/M4.md#m4-r16) | PROP-03 itself was still ASCII-only — the blind spot inside the guard | guard | [x] |
| [M4-R17](reviews/M4.md#m4-r17) | `find_iter` **hides** a value from the candidate set → the invariant passed *vacuously* | **LEAK** | [x] |
| [M4-R18](reviews/M4.md#m4-r18) | Five comments still described the deleted containment gate | docs | [x] |
| [M4-R19](reviews/M4.md#m4-r19) | **Overlap rescan is O(n²) → unauthenticated DoS** (151 s on a 200 KB field) → the rescan is now for **bounded** patterns only; the unbounded two rely on the fixpoint | **BLOCKER** | [x] |
| [M4-R20](reviews/M4.md#m4-r20) | `mask_all` fails **open** on pass exhaustion → the fixpoint is now **confirmed**, not assumed | low-med | [x] |
| [M4-R21](reviews/M4.md#m4-r21) | `mask_all` runs the detector ≥2× (~2× NER inference) → **accepted as designed** (it *is* the fixpoint confirmation); measured by M5's PERF-01 | low | [x] |
| [M4-R22](reviews/M4.md#m4-r22) | No MSRV in `Cargo.toml` — an M5 CI blocker-in-waiting | low | [x] |
| [M4-R23](reviews/M4.md#m4-r23) | Four code comments cite ROADMAP sections whose content moved to `docs/reviews/` | docs | [x] |
| [M4-R24](reviews/M4.md#m4-r24) | **Masking is O(n²) in entity count** — a second DoS on the same path ([sibling of R19](reviews/M4.md#m4-r19)); DOS-01…03 missed it (each pins one entity) → the splice is now one left-to-right copy, and **DOS-04 varies the count** | **BLOCKER** | [x] |

<a id="m5"></a>
## M5 — Integration & performance testing
Prove the whole system holds **end-to-end** and **under load**, then document it. The feature set
(structured + NER + streaming + multi-provider) is complete enough to test as a product.

> **Prerequisite met: M4's ledger is clean (all 24 findings).** The DoS class needed **two** fixes, because
> the masking path has **two** size axes: field *size* ([M4-R19](reviews/M4.md#m4-r19), the candidate rescan)
> and entity *count* ([M4-R24](reviews/M4.md#m4-r24), the mask splice). Both are now linear and guarded —
> DOS-01…03 vary the size, **DOS-04 varies the count** — so a load result measures the product, not an
> algorithmic bug. `fmt` and `clippy` are green.
>
> *(This box used to add "CI has its MSRV ([M4-R22](reviews/M4.md#m4-r22))". It did not:
> [M5-R5](reviews/M5.md#m5-r5) found the declared `1.82` could not even parse the dependency tree. The
> floor is now **1.89** — measured, and **built** by CI.)*

- [x] **Real integration tests** — full mask → forward → (stream) → de-mask round-trips, tool-call
  round-trips, multi-turn determinism, and the fail-closed paths. **Mock upstreams cover all three preset
  shapes** (OpenAI / Copilot / Anthropic) — no accounts needed. The **real-provider smoke is
  Anthropic-only** (the only provider we have; opt-in, needs a key, never in CI without one) —
  `tests/anthropic_smoke.rs`, written and gated, **not yet run against a live key** (no credentials in this
  environment).
  - [x] Implement the two cataloged-but-missing e2e cases **E2E-02** (CSV `tool_result`) and **E2E-04**
    (`SELECT … FROM DUAL`) against a mock.
- [x] **Manual "does the whole structure hold?" procedure** (`docs/MANUAL_VERIFICATION.md`). **Bar redefined
  to opt-in (2026-07-15).** A run against the *real* provider needs a human with a live `ANTHROPIC_API_KEY`,
  which this environment does not have — and the natural workaround (routing a Claude Code session through
  the proxy) is blocked: Claude Code speaks only native `/v1/messages`, the proxy is OpenAI-compat-only, so
  it 404s — that is Option B (Backlog). Instead the procedure was **dry-run-validated against a mock upstream,
  through the real binary**, running the same PII prompt twice:
  - **Run A** (`PII_DEBUG_SKIP_DEMASK=1`) — client got the **placeholder** (`[EMAIL_1]`); the upstream saw
    only the placeholder; the raw value never appeared in the trace (DBG-02).
  - **Run B** (normal) — client got the **restored value**; the trace still showed only the placeholder, and
    the de-masked output was **never** logged.

  A vs B on the same input yielded a **byte-identical masked body upstream** → the chain holds end-to-end.
  The permanent guarantee lives in CI (DBG-01 `e2e_debug_skip_demask…`, DBG-02 `tests/log_safety.rs`); the
  real-provider dual-run stays **ready and opt-in** in `docs/MANUAL_VERIFICATION.md` + `tests/anthropic_smoke.rs`
  (E2E-INT-01) for whoever holds a key. See DEVLOG 2026-07-15.
- [x] **Performance / load harness** — concurrent connections, large bodies, streaming throughput; latency
  of the mask → forward → de-mask path (NER on/off). *Stability under load was the founding
  motivation — measure it, don't assume it.* `tests/perf.rs` (system-level: concurrent connections,
  streaming throughput over the real router) + `tests/ner_perf.rs` (NER-specific, `--features onnx`,
  `#[ignore]`d). RAM was not separately instrumented — Windows has no portable in-process equivalent to
  `/proc/self/status`, and latency/throughput were the numbers that mattered for the questions below.
  - [x] Score the **fixpoint's second detector pass** ([M4-R21](reviews/M4.md#m4-r21)): `mask_all` runs the
    detector ≥2×, which roughly **doubles NER inference**. Re-measured live
    (`tests/ner_perf.rs::m4_r21_the_fixpoints_second_pass_roughly_doubles_ner_inference`): ~1.8–3× on a
    short field, consistent with the ~2× (64 ms → 127 ms) recorded when M4-R21 was closed. Confirmed as a
    deliberate correctness cost, not a regression — no fix needed.
  - [x] **Measure the `onnx` NER on large fields, and chunk it if the numbers say so.** The numbers said
    so, and said something worse than expected: past ~500 tokens the ONNX call **errored outright**
    (`Expand` op, position-embedding overflow) rather than merely slowing down — a hard block under
    `NER_REQUIRED`. Fixed with overlapping-window chunking (`src/pii/onnx.rs`); linear afterward (measured
    448 ms / 2.07 s / 7.53 s at 64×/256×/1024× field size). Found while closing
    [M4-R24](reviews/M4.md#m4-r24); detail in [`ARCHITECTURE.md`](ARCHITECTURE.md) → *NER chunking*.
  - [x] `tests/complexity.rs` is the harness's floor and now covers **both** axes: DOS-01…03 vary the field
    *size*, **DOS-04** varies the entity *count* ([M4-R24](reviews/M4.md#m4-r24) — the axis the first three
    silently held at one). A load result can no longer be a measurement of an algorithmic bug in the
    structured path.
- [x] **Update the root `README.md`** (+ `README.it.md`) to describe the shipped product. Rewritten from
  the "early development" placeholder into a product README: what it protects and why, a `curl` showing
  the *actual* masked body the provider receives, the detection matrix (universal + 10 national IDs +
  NER languages), the fail-closed / never-log / linear-under-load bars, provider presets, and the full
  env reference (NER + debug folded into `<details>`). The **internal development status is deliberately
  *not* in the README** — that is what this file is for; the README defers here.
- [x] **Release binaries (GitHub Actions) — tag-driven pipeline.**
  - **Pipeline restructured (2026-07-15) — tag-driven, no push CI.** The cross-compile matrix lives **once**
    in a reusable **`release-build.yml`** (`workflow_call`, `name: Release build`), called by **two** entry
    points that can't drift:
    - **`release-build-publish.yml`** (`name: Release build & publish`) — triggers **only on a `v*.*.*` tag**;
      builds every target, then publishes a GitHub Release. **No `workflow_dispatch`** and no publish `if`-gate —
      "a manual run can't publish" is **structural** (no manual trigger at all), which retires
      [M5-R11](reviews/M5.md#m5-r11)'s whole risk class rather than guarding it. **Its status badge is what the
      READMEs now show** — "did the last tag build?".
    - **`manual-build.yml`** (`name: Manual build`) — `workflow_dispatch` only; same `release-build.yml`, keeps
      binaries as **throwaway artifacts** (30-day). No publish job, so it *cannot* cut a release. Since no
      other job cross-compiles, this is **the way to confirm every target still compiles before you tag**.
    - **`ci.yml` was disabled** (renamed `ci.yml.disabled`) — a deliberate change of approach: nothing ran on
      push/PR. **`cargo test` moved into `release-build.yml`** (so the suite still runs — on every
      target's native runner — but at `manual-build` / tag time, not per push; a release that fails its tests
      never publishes). **`fmt` / `clippy` / `msrv` became local-only** gates (the CLAUDE.md "green before
      done" bar), so a lint break surfaced at build time or locally, not on a PR. **Since reversed in part,
      and this is the current state:** when Dependabot was enabled, `security.yml` (cargo-deny) and a
      **trimmed `ci.yml`** (fmt/clippy/test/msrv on push + PR) came back so dependency-bump PRs self-verify.
      So **push/PR CI exists today** — it just doesn't cross-compile; the all-target build stays tag/manual
      only, which is why `manual-build.yml` above is still how you confirm every target before tagging. See
      DEVLOG 2026-07-15 / ARCHITECTURE → Supply-chain.
    - **Targets: Linux x86_64 + arm64, macOS arm64, Windows x86_64 + arm64.** macOS is arm64-only because
      `ort` ships no prebuilt ONNX Runtime for `x86_64-apple-darwin` at the pinned `rc.12` (Intel Macs are
      legacy). Linux and Windows each ship both arches, built **natively** on their own runner
      (`ubuntu-24.04-arm`, `windows-11-arm`) — no cross-linker. **Windows-arm64 (`aarch64-pc-windows-msvc`)
      builds natively on `windows-11-arm`** — **validated green by a `manual-build` run (2026-07-15)**, despite
      `aws-lc-sys`'s upstream win-arm64 friction (the native runner has the ARM64 toolchain it needs to build
      its ARMv8 crypto from source; a local x86_64 cross-check could not). ONNX Runtime links
      **statically**, so each artifact is a single self-contained binary.
  - **The release profile is the max-optimization one** (`[profile.release]`): `opt-level = 3`,
    **fat LTO**, `codegen-units = 1`, symbols stripped — worth the ~5 min build, because the masking path
    *is* the product's latency. **`panic` stays `unwind` on purpose**: `abort` would turn a caught masking
    panic — which today blocks **one** request, fail-closed ([M4-R19](reviews/M4.md#m4-r19)) — into a
    process abort, i.e. an **outage**. That is not a smaller failure; availability is a privacy property
    here (a proxy that is down protects nothing, and clients fail over to the raw provider).
  - **First tag is `v0.4.0` — an interim tag** to populate the release badge, cut only after a `manual-build`
    run is green on **all** targets (win-arm64 especially). It is **not** the `1.0.0` release: `1.0.0` still
    waits on **[M6](#m6)** — it ships only once Claude Code works end-to-end through the native `/v1/messages`
    route against real Anthropic, plus the manual live-provider verification. *(The earlier per-push CI that
    built every release target — push `9a966df` — was retired; a trimmed per-PR `ci.yml` (no cross-compile)
    later returned. See DEVLOG 2026-07-15.)*

<a id="m5-ledger"></a>
### Review ledger — M5 → [`reviews/M5.md`](reviews/M5.md)
**Round 1 (2026-07-14): 6 findings — all closed. No leak, no fail-open, no over-mask regression.** The
chunking fix was verified against the **pre-fix** commit (the `Expand` error reproduces) and driven
through the real binary end-to-end with NER on an oversized field.

**M5-R5 turned out to be the sharp one.** Chasing it revealed the declared MSRV was **fiction**: `1.82`
cannot even *parse* the dependency tree. Measured floors: **1.86** (default) / **1.89** (`--features
onnx`). Since the shipped product always runs with `onnx` on, the manifest declares the **single** real
floor — **1.89** — and CI *builds* on it. This is exactly the failure M4-R22 tried to prevent and
structurally could not: `rust-version` makes cargo refuse a too-**old** toolchain; it cannot notice the
crate drifting **past** its own declared floor. **Only a job that builds on the MSRV can.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M5-R1](reviews/M5.md#m5-r1) | TESTING.md still says the NER is unchunked and quadratic — the claim M5 disproved *and* fixed | docs | [x] |
| [M5-R2](reviews/M5.md#m5-r2) | `MAX_SEQUENCE_TOKENS` didn't bound what reaches the model — chunks re-tokenize to 481–483 vs a usable limit of 512 → the ceiling is now **enforced** at the one choke point, and the drift is **asserted** | hardening | [x] |
| [M5-R3](reviews/M5.md#m5-r3) | The chunk slice hard-indexed tokenizer offsets — the one place on the masking path that could panic on attacker input | hardening | [x] |
| [M5-R4](reviews/M5.md#m5-r4) | The fixpoint's convergence proof covers the recognizers, not the NER — placeholder inertness is **empirical** for the model, and a **model swap must re-check it** (GLiNER especially) | invariant | [x] |
| [M5-R5](reviews/M5.md#m5-r5) | CI never exercised the declared MSRV → **and the declared MSRV was wrong**: measured, declared as the single real floor (**1.89**), and now **built** by CI | low → med | [x] |
| [M5-R6](reviews/M5.md#m5-r6) | The READMEs' Status was self-referentially stale and understated the `1.0.0` gate | docs | [x] |

**Round 2 (2026-07-14) — closure verification: 5 of 6 hold; M5-R2's *valve* did not. All 3 now closed.**
Re-measured both MSRV floors + both negative controls, re-ran the four live NER guards (they reproduce the
481–483 drift), confirmed `run_and_decode` really is the single session choke point and that clamping
cannot corrupt a span — then found that the clamp returns `Ok`, which is precisely what `NER_REQUIRED`
exists not to get.

**M5-R7 is the one to remember: my own fix committed the M4 retrospective's signature move.** Clamping an
over-long NER sequence *relocated* the failure instead of closing it — it is the right call for the
default (fail-open) posture and fatal to `NER_REQUIRED`, and **a component that cannot see the posture
must not choose between them.** The overflow is now an `Err` that flows into the channel that already owns
that decision (`FailOpen` / `try_detect`). Promoted to [`ARCHITECTURE.md`](ARCHITECTURE.md) → *Robustness
& fail-closed*: **a detector may degrade its own recall, but it may never decide *for the caller* that
degraded output is acceptable.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M5-R7](reviews/M5.md#m5-r7) | The clamp traded `NER_REQUIRED`'s fail-closed block for a silent partial scan — the M5-R2 fix *relocated* the failure → overflow is now an **`Err`**; the posture is the caller's | **fail-closed** | [x] |
| [M5-R8](reviews/M5.md#m5-r8) | ARCHITECTURE's *NER chunking* still named the constant M5-R2 deleted — in the file M5-R1 had just made the single home | docs | [x] |
| [M5-R9](reviews/M5.md#m5-r9) | The M5-R2 guard hand-copied the one constant `chunk_char_ranges` was made `pub` to avoid hand-copying | guard | [x] |

**Round 3 (2026-07-14) — closure verification + the product changes: all 3 closures hold.** M5-R7's
restored contract was driven **through the real binary** (a `run_and_decode` error → **400, nothing
forwarded** under `NER_REQUIRED`; swallowed to structured-only by default), and the chunked path's
recall is unchanged. The MSRV collapse, the release profile, the manual release trigger, the optional
provider token and both READMEs check out — including the `curl` example, byte for byte. Three new
findings, all from the product changes; **[M5-R10](reviews/M5.md#m5-r10) is the sharp one**: the
compile-time invariant that the M5-R7 closure's "this path is unreachable" argument *rests on* pins
`window < ceiling`, not the 32-token drift headroom it is cited for — measured, the assert passes at
window values where most chunks overflow the model.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M5-R10](reviews/M5.md#m5-r10) | The compile-time invariant guards `window < ceiling`, not the drift headroom it is cited for — the assert passes where most chunks overflow | hardening | [x] |
| [M5-R11](reviews/M5.md#m5-r11) | The release publish gate keys on the *ref*, not the *event* — a manual run on a tag ref does cut a release | low | [x] |
| [M5-R12](reviews/M5.md#m5-r12) | The `panic = "unwind"` rationale cites the wrong status code for the path it exists to protect | docs | [x] |

<a id="m6"></a>
## M6 — Native Anthropic `/v1/messages` (Claude Code passthrough)
Let a **native** Anthropic client — **Claude Code** (CLI + IDE extension) and the Anthropic SDK — route
through the proxy and be masked, without the client converting to OpenAI. This **promotes the Claude-Code
slice of [Option B](#backlog)**: today the proxy is OpenAI-compat-only, in *and* out, so a native client's
`POST /v1/messages` hits a 404 and nothing is masked.

> **Why now, and why this slice only.** The point of the tool is to sit in front of the LLM you actually
> use — and the one we use is Claude Code. It speaks *only* the native Anthropic Messages API (confirmed: no
> OpenAI-compat mode), so "front Claude Code" is precisely "accept `/v1/messages`". The broader Option B
> (native adapters for *every* provider) stays in Backlog — a missed schema field is a leak, so we add
> exactly one native schema: the one with a real user behind it.

**Approach — native-in, mask-in-place (decided 2026-07-15).** Accept the Anthropic-native body and mask it
**directly**, forwarding native→native — *not* translating it to the OpenAI shape and back. Translation adds
two lossy schema boundaries, each a leak surface; masking in place adds none. This is the architecture of the
prior proxy's own working Claude Code route (`llmproxy-extended` @ `d9962a4`, *"Anthropic-native /v1/messages
route for Claude Code passthrough"*), reused as the blueprint. That project's multi-provider *translation*
adapter is **not** adopted (wrong shape for a native client, and against fail-closed) — it serves only as a
**field map** for the Anthropic schema.

- [x] **Inbound `POST /v1/messages`** beside `/v1/chat/completions`, **registered only when
  `UPSTREAM_PROVIDER=anthropic`** (decided 2026-07-15 — the only upstream that speaks native `/v1/messages`;
  other providers 404 it, so no mis-route). Unknown/unreadable shapes still **fail closed** (FC-03 unchanged);
  every other path stays 404. A `WireSchema` tag on `RequestContext` routes the privacy stage to the native
  walk without disturbing the OpenAI path.
- [x] **Mask the native body in place** — no OpenAI round-trip. Coverage must be exhaustive (a missed field
  is a leak — the Option B risk, pinned by tests):
  - the top-level **`system`** field (a string *or* a `{type:text}` block array) — masked **in place** (we
    walk the native schema directly; no inject-as-message trick, unlike the prior proxy);
  - `messages[].content` as a **string** *or* an **array of content blocks**, dispatched on `type`:
    `text` → `text`; `tool_use` → every string leaf of `input` (a JSON *object*, not an encoded string);
    `tool_result` → its `content` (string or nested block array — recursive); `thinking` → `thinking`;
    `document` → `title` / `context` + its `source` dispatched on `source.type` (`text` → `data`, `content` →
    nested block array recursed, `base64` / `url` → skipped, **unknown → fail closed** — M6-R1; the plan's
    "skip document" would have leaked a text-bearing document); `image` / `redacted_thinking` → non-text, skipped;
  - `tools[].description` + `input_schema` descriptions (reuse the OpenAI `mask_schema_descriptions`).
  - **Unknown block-type → fail closed, 400** (decided 2026-07-15 — strict, per this repo's ethos). The
    consequence: the **known set must be exhaustive for real Claude Code traffic** — pinned by a guard test, so
    a new Anthropic block type is a *conscious* addition, never a silent leak. **First cut handles the core
    Claude-Code request blocks** (`text` / `tool_use` / `tool_result` / `thinking` / `image` / `document` /
    `redacted_thinking`); **server-side result blocks** (`server_tool_use` / `web_search_tool_result`, sent
    only when server-side tools are enabled) fail closed for now — safe (no leak), a conscious future addition.
  - **`thinking` is masked but never *de*-masked:** it is generated over already-masked input, so it holds only
    placeholders; leaving them intact keeps the block's `signature` valid across a multi-turn replay (re-masking
    an inert placeholder is a no-op). Promoted to ARCHITECTURE.
- [x] **Native→native forward + auth passthrough.** The client's own credential wins — `Authorization: Bearer`
  (the OAuth `sk-ant-oat01-*` token Claude Code sends) forwarded **verbatim**, else a client `x-api-key`, else
  the proxy's configured key injected as `x-api-key`; no credential at all → **401** before forwarding.
  `anthropic-version` (default `2023-06-01` when the client omits it) + `anthropic-beta` pass through the
  allowlist. **Never** place an OAuth token in `x-api-key` (Anthropic 401). *This is why auth is not a blocker
  — the proxy never needs to hold a key.* *(Client-credential-wins matches this scope item and the existing
  chat-path posture; the terser DEVLOG T6 line had proxy-key first — reconciled toward the ROADMAP + precedent,
  recorded in DEVLOG 2026-07-16.)*
- [x] **Response demask (buffered) + native augmentation.** The reply is a top-level `content[]` array
  (`text` + `tool_use.input` string leaves, restored with the **plain** demask since `input` is a real JSON
  object serde re-escapes), not `choices[].message` — the buffered demask mirrors the request walk. The
  placeholder-augmentation prompt is injected into the top-level `system` field (string or block array), only
  when something was masked.
- [x] **Anthropic SSE demask** — de-anonymize the streamed response: `content_block_delta` with
  `delta.type:text_delta` (`delta.text`) **and** `input_json_delta` (`delta.partial_json`, JSON-aware), held
  back per content-block `index` and flushed at each `content_block_stop` (a delta after the stop is
  protocol-invalid, so the flush is injected **before** the whole stop frame — the demasker holds the `event:`
  line to control frame ordering). `SseDemasker` is factored into the shared split-placeholder core
  (`split_demaskable`, the hold-back buffer) + a per-schema delta rewriter (OpenAI vs Anthropic). **The prior
  proxy left this a TODO — its streamed replies were forwarded un-demasked — and closing it is the point: a
  placeholder reaching the client is the exact failure this tool exists to prevent.**
- [x] **Verification — mock.** A native-schema coverage / fail-closed suite against a mock upstream (10 e2e +
  13 unit) — round-trip, tool defs (incl. a content-source document), fail-closed on an unknown block/document
  source, 404-when-not-anthropic, the full auth matrix, and the streaming split-placeholder.
- [x] **Verification — live, and it holds.** Ran 2026-07-16: a **real Claude Code session** round-tripped
  through the proxy against **real Anthropic**. The native route, the auth passthrough (200 first try with
  **no credential configured** — it forwards its own, the proxy held no key), the in-place masking
  (`contatta [EMAIL_1], IBAN [IBAN_1]`), the response de-mask (real values restored on screen) and DBG-02
  (**zero** raw PII in the trace, on real traffic) all held. This is what closed M6's own gate. Details:
  DEVLOG 2026-07-16.
  > **Scope, stated honestly: that run exercised the *route*, not the *hybrid*.** No model was configured,
  > so it silently ran structured-only — email and IBAN masked because they are *deterministic*
  > recognizers, while the NER never ran (a `Person` would have gone upstream in clear). The **CC battery**
  > (CC-01…CC-09 × OFF/ON, `NER_REQUIRED=1`) is what verifies the hybrid, and it is **blocked on
  > [M7](#m7)** — twice over: its prompts need rewriting, and 9 scenarios × 2 runs is impractical at the
  > current latency. **`1.0.0` now waits on M7, not on this box.**
- **Out of scope:** Bedrock / Vertex native endpoints, `/v1/messages/count_tokens`, the broad multi-provider
  translation registry, and **server-side tool-result blocks** — all remain Backlog.

### Review ledger — M6 → [`reviews/M6.md`](reviews/M6.md)
**Round 1 (2026-07-16): 5 findings — 1 leak, 1 hardening, 3 docs/test-quality. All closed.** The leak (R1) was
reproduced through the real router and is now pinned by a unit test + an e2e.
**Round 2 (2026-07-16): all 5 closures verified sound** (re-measured 85 default / 97 onnx lib, `fmt`/`clippy`
clean; drove the new document surface through the real router — leak closed at every dispatch, fail-closed on
every unmodeled shape, no over-reach). **One new cosmetic nit — R6.**
**Round 3 (2026-07-16): R6 verified closed + final adversarial sweep — nothing new.** Messages caller-neutral
and value-free for both callers, fail-closed recursion pinned; suite green on both feature sets. **M6 ledger
clean, 6/6 closed, nothing left to fix.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M6-R1](reviews/M6.md#m6-r1) | A `document` block with a non-text-but-text-bearing source (`source.type:content`) is forwarded **unmasked** → `source.type` dispatch + fail-closed on unknown | **leak** | [x] |
| [M6-R2](reviews/M6.md#m6-r2) | `mask_anthropic_system`'s array branch fails **open** on an unrecognized object (same class as R1, one level up) | hardening | [x] |
| [M6-R3](reviews/M6.md#m6-r3) | M6 test counts are wrong: DEVLOG "96 lib tests default" (default is 84) + ROADMAP/DEVLOG "14 unit" (actual 12) | docs | [x] |
| [M6-R4](reviews/M6.md#m6-r4) | Some e2e placeholder-presence asserts are satisfiable by the augmentation prompt text (non-vacuous via the `!contains(raw)` checks, but weaker than they read) | test-quality | [x] |
| [M6-R5](reviews/M6.md#m6-r5) | `inject_augmentation_anthropic` doc says "Prepend" but the code appends | docs | [x] |
| [M6-R6](reviews/M6.md#m6-r6) | A document `content`-source sub-block reuses `mask_tool_result_content`, so its client-facing 400 reason says "tool_result" (no PII; diagnostic-accuracy only) → renamed `mask_content_block_array`, caller-neutral messages | cosmetic | [x] |

**Delivery (decided 2026-07-15):** one feature branch → PR (`feat/m6-anthropic-messages`), **streaming
included from the start** (Claude Code streams by default, so a buffered-only first cut wouldn't be usable
with the real client), tests adversarial-first; the reinstated per-PR CI gates it. The full implementation
plan (files, the 7 stages, the schema walks) lives in DEVLOG 2026-07-15.

**Gate to `1.0.0`.** The first tag ships only once Claude Code works end-to-end through this route, verified
against real Anthropic — not merely once CI is green.

<a id="m7"></a>
## M7 — NER latency: make the hybrid usable with a real client

**Opened 2026-07-16 by the M6 live gate, and it now holds `1.0.0`.** M6 proved the chain is
*correct* against real Anthropic. The same session proved the product is *too slow to use*: with the
NER on, Claude Code re-sends 20–40 KB of system prompt + tool schemas **every turn**, and we
re-scan all of it from scratch. Full measurements: DEVLOG 2026-07-16. **The plan is
DEVLOG 2026-07-16 → *M7 implementation plan*; start at S0.**

> **Start by re-measuring — the headline numbers are suspect, and that is the first finding.** The
> ~0.96 s/KB figure came from a fixture **densely packed with names**. Real traffic is the opposite
> shape: ~30 KB of boilerplate with **~zero** PII plus a ~100-byte user message that has it all —
> and `mask_all` runs **per field**, so the boilerplate takes **one** pass (~286 ms/KB → ~9 s/turn),
> not two. A realistic turn is likely **~9 s, not 27 s**, which **reorders the leads**: the fixpoint
> matters far less than written, the cache far more. *The corpus had a shape, and the shape was a
> blind spot — the exact charge this milestone levels at M5's PERF-01, earned one level down.*

> **This is not a leak and not an algorithmic bug** — the path is linear (M4's DoS guards hold) and
> the structured layer is free (20 ms for 29 KB, ~1,400× faster than the hybrid). It is a *constant
> factor*, and **availability is a privacy property here**: a proxy too slow to keep switched on
> protects nothing, because it gets switched off.
>
> **Why M5 missed it.** PERF-01 measured a *repeated synthetic sentence* and concluded "linear" —
> true, and beside the point. It never measured a **real client's payload**. The lesson is the M4-R13
> shape again: *a corpus has a shape, and that shape is a blind spot.* A synthetic benchmark's shape
> is "one field, grown"; a real client's is "the same 30 KB of boilerplate, re-sent forever".

- [x] **Re-measure first, with a *realistic* fixture — this gates every box below.** Done
  (`tests/m7_latency.rs`, 112 fields / **22.3 KiB**, shape-asserted). **The headline was ~6×
  pessimistic: a realistic turn masked in 4.2–4.7 s, not 27 s** — and not the ~9 s predicted below
  either. But **the stated mechanism was wrong**, and that is the more useful half: see the box
  under this list.
- [x] **Declare the bar, then decide against it: the bar was MISSED — we stopped anyway, and that is
  the honest sentence.** A realistic turn masks in **~4.7 s** at the shipped default on the reference
  box (isolated, balanced/energy-efficiency power plan; reproduced independently at 4,724 and
  4,757 ms) — **~60% over the ~3 s bar**.
  S3 (cache) and S4 (fixpoint) are still **not done, on purpose**, but *not* because the bar was met:
  because **what M7 could deliver, it delivered** — a reproducible **~2×** — and the remaining gap is
  the machine, not the code. Both leads trade real risk (state on the masking path; lost detection)
  and they should be bought deliberately, not because we were already in here.

  > **This box said "2.46 s" once, and that number went in the READMEs. It was the fastest of seven
  > observations and it has never reproduced** ([M7-R12](reviews/M7.md#m7-r12)). The honest figure is
  > ~4.7 s. Worse, the *explanation* was invented: DEVLOG called the spread "power and thermal
  > regime, nothing else", and the data refute it — a **battery** run (3.94 s) beat **three AC** runs
  > (4.76 / 4.84 / 4.93 s). No power model orders that. "Throttled AC" was a label assigned *post hoc
  > from the number itself*; no thermal state was ever measured. **A run was called slow because it
  > was slow, and then cited as evidence of throttling.**
  >
  > **And then the machine's owner explained why no power model could ever have ordered it: there was
  > no power difference.** Both sets of runs were on the **same balanced/energy-efficiency plan** —
  > "AC" meant only that the charger was attached, and the profile never changed. So the variable was
  > not merely mis-modelled; **it did not vary.** The label separated nothing, and the mystery it was
  > invented to explain was an artefact of the label. *One question to the person who owns the box
  > would have retired the whole story — and no amount of re-measuring could have, because we were
  > measuring a constant and theorising about its effect.* Whatever a **performance** plan does here is
  > simply unmeasured; the published figure is the ordinary-laptop case on purpose.
  >
  > **The variable that *is* measured:** test **concurrency** — cargo runs the perf tests in
  > parallel, worth **1.50×** at constant power. The documented command measured the product against
  > four other copies of itself. `--test-threads=1` is now part of the contract.
  >
  > **So the bar is not the guard, and never could have been.** The assert is a **ratio** against an
  > in-run calibration leg ([M7-R9](reviews/M7.md#m7-r9)): it held ~**1.7–2.3×** across every regime
  > above, while the absolute moved 2.9×. The **asserted** floor is **≥1.5×** — and that floor, not a
  > tighter band, is the claim to quote, because the ratio cancels the box's power state but not its
  > raw speed, so a faster box compresses the speedup *toward* the floor (2.19× reference box, 1.74×
  > a faster one — [M7-R18](reviews/M7.md#m7-r18)). Every tighter band published has been undercut by
  > the next clean run; the floor has not.
  >
  > **Its domain, stated because an invariant without one is worse than none**
  > ([M7-R13](reviews/M7.md#m7-r13)): the speedup **scales with the core count**, so the ≥1.5×
  > floor is asserted only where there are cores to parallelize over (**≥4**). Since the
  > 2026-07-17 `pool=1` flip the derived default is `intra = cores`, a strict no-op versus the
  > pre-M7 single-thread shape **only at 1 core** — from two cores up it already adds threads
  > (DEVLOG 2026-07-17). And the ratio buys regime-independence, **not sensitivity** — at a 1.5
  > floor against a ~1.7 worst case it still tolerates a ~13% regression ([M7-R14](reviews/M7.md#m7-r14)).
  >
  > **S3 is the named lead** for the two open cases (the ~4.7 s turn, and ~40 KB traffic): the
  > boilerplate is byte-identical every turn, so a content-keyed cache makes turn 2+ nearly free —
  > and unlike threads, it is *indifferent to the box*.
- [x] **Use more than 1 core of 12.** `NER_INTRA_THREADS` (`src/pii/onnx.rs`,
  `resolve_pool_and_intra` / `default_intra_threads`) — explicit env wins, else **derived**:
  `max(1, available_parallelism() / NER_POOL_SIZE)`. Measured on both axes rather than picked by
  intuition. **The intra-derivation improved *both* axes over the pre-M7 shape** — at the `2×6` M7
  shipped as its default, latency ~4.7 s → ~2.5 s (~1.9×) *and* throughput 0.288 → 0.609 turns/s
  (2.11×). The **shipped default is now `pool=1` since the 2026-07-17 flip** — a ~−23% throughput
  trade *vs that pooled shape*, taken for the personal case's half-RAM (a centralizing operator sets
  `NER_POOL_SIZE=N`); see DEVLOG 2026-07-17. Full tables: DEVLOG 2026-07-16.
  > **⚠ SUPERSEDED BY [M11 Track B](#m11-b) (2026-09-02) — the divisor, not the derivation.** This
  > box is left as written because it is the record of **what M7 shipped and why**. What changed:
  > the base `intra` is divided out of moved from `available_parallelism()` (**logical** threads) to
  > `min(physical_cores, available_parallelism())`, so on the reference box the default is `1×6`
  > rather than `1×12` and `NER_POOL_SIZE=2` gives `2×3` rather than `2×6`. Everything else here —
  > the formula's shape, the single home, the `0`-is-unset symmetry, the deployment argument, the
  > −23% throughput trade behind the `pool=1` flip — is unchanged. Read
  > [ARCHITECTURE → *NER threading*](ARCHITECTURE.md) for the current rule.

> **What the measurement overturned.** Three claims in this milestone's own text did not survive
> contact with a realistic fixture. Recording them because the *pattern* is the point — this is
> the third time in this repo a shape has been mistaken for a finding (M4-R13, M5's PERF-01, now).
>
> 1. **"The boilerplate has ~zero PII, so it costs one pass."** It does not. The hybrid tags
>    `(Organization, "An")` — a **two-character fragment of "Anthropic's"** — in PII-free
>    instruction prose, so the biggest field in the turn pays a **second** full NER scan. A real
>    Claude Code system prompt says "Anthropic" and "GitHub" constantly, so this is the normal
>    case, not an edge one. **The fixpoint lead (S4) is therefore *more* relevant than the plan
>    concluded, not less** — though the bar was met without it.
> 2. **"`pool = 1` is not a trade at all"** (my own words, one level up). Measured: it **regresses
>    throughput ~21–23%** (0.472 vs 0.609 turns/s at concurrency 4; the reviewer independently
>    measured −21%). Intra-op scaling is sublinear, so 4 sessions × 3 threads beats 1 × 12 in
>    aggregate. The deployment-shape argument below stands exactly as written — which is why the
>    default is derived and overridable, not a constant.
> 3. **"SMT may hurt; 6 may beat 12."** **Unresolved — and the first cut claimed otherwise from
>    n=1** ([M7-R2](reviews/M7.md#m7-r2)). The sign flips run to run (`1×6` wins by 11%, then `1×12`
>    by 8%, then `1×6` by 3%…), because the same configuration drifts ~40% *between* runs — the
>    effect is smaller than the noise it was read off. The sweep now repeats each shape and prints
>    min/median/spread, which is the guard that would have stopped the claim being made.
>    **[M11 Track B](#m11-b) (2026-09-02) closed this — by decision, NOT by resolving it.** It
>    remains unresolved *as a measurement* on this hardware, and M11 says so explicitly: it adopts
>    physical cores as the conventional intra-op base for GEMM-bound inference and stops paying for
>    a knob no timing here can read. M11's own re-run is consistent with that (`1×6` 2.48× vs `1×12`
>    2.42× — still inside the noise, still not a result).
>
> **Confirmed, on the other hand:** scaling really is sublinear — 12 threads buy **~2×**, not 12×.
> And a lone request really does occupy **one session**, so the pool is inert at concurrency 1 —
> but believe that from the **code** (the field walk holds `&mut Vault`; `infer_chunked` loops its
> windows; so `pool` cannot help one request), not from a timing table that cannot resolve it.
>
> **The meta-lesson, which is the same one a third time:** the fixture's *shape* was the blind spot
> at S0, and the measurement's *design* was the blind spot at S1. Getting the corpus right does not
> save you if you then read a conclusion off a single sample.

  > **The two knobs multiply — that is the trap.** `NER_POOL_SIZE × intra_threads` is the thread
  > count. Today it is `2 × 1 = 2`. Naively setting `intra = 12` while `pool = 2` gives **24 threads
  > on 12 cores** — oversubscription, and plausibly *slower* than now. The invariant is that the
  > **product** fits the box, not that either factor saturates it. So the default should be
  > **derived**, not fixed: `intra = max(1, available_parallelism() / NER_POOL_SIZE)`, overridable by
  > env like `NER_POOL_SIZE` already is. A fixed number is wrong on a 2-core VM and on a 64-core
  > server alike.
  >
  > **…but that product is the *saturated-load* count, and the divisor is a pessimization here —
  > read the code before adopting the formula** (DEVLOG → S1a). A single request is sequential at
  > **three** nested levels — fields (`&mut vault`), chunks (`infer_chunked`'s `for` loop), then the
  > model call — so **one request uses exactly one core no matter what `pool` is**; the pool only
  > wakes for a *second* concurrent request. A single request can reach `intra`, never `pool ×
  > intra`. At the default `pool = 2` the formula yields `12 / 2 = 6` and leaves **6 cores idle** in
  > precisely the concurrency ≈ 1 case M7 exists for. Either fan the chunks across the pool
  > (near-linear, but each session copies the weights: ~400 MB each, so `pool = 6` ≈ **2.5 GB** vs
  > today's 834 MB) or run `pool = 1, intra = all` (free, sublinear ~3×). **And parallelize
  > *detection*, never *minting*** — placeholder numbering follows encounter order, so a parallel
  > field walk makes `[EMAIL_1]` a coin flip; the `&mut` is load-bearing.
  >
  > **And the right shape depends on the deployment, which the proxy cannot know.** A personal proxy
  > in front of Claude Code (the M6 case) has concurrency ≈ 1 → latency is everything → `pool=1,
  > intra=all`. A shared proxy fronting many clients wants the current shape, scaled. Both are
  > legitimate; that is precisely why this is config with a sane default, not a constant.
  >
  > **Two things to measure, not reason about:** (1) **SMT** — `available_parallelism()` reports
  > *logical* cores (12 = 6 physical × HT), and for dense math hyperthreading often *hurts*, so 6 may
  > beat 12; (2) **scaling is sublinear** — expect ~3× from 6 threads, not 6×, and less on an **int8**
  > model whose kernels are memory-bandwidth-bound rather than ALU-bound. Benchmark both axes before
  > believing either.
- **S3 (system-prompt entity cache) and S4 (skip the NER on the fixpoint's later passes)** — the two
  latency leads M7 named but **deliberately did not buy** (M7 delivered its reproducible ~2×; each of
  these trades real risk — state on the masking path; lost NER recall at the seams — and should be bought
  on purpose, not because we were already in here). **Deferred to [M7.1](#m71)**, where the mechanism and
  the risk each must answer are written out in full.
- [x] **Rewrite the CC prompts as natural agent tasks** — a **verification debt from M6**, tracked
  here because M7 owns the re-run. The battery said *"reply with exactly this sentence: contact
  jane.doe@example.com, IBAN …"*, and the model **refused it as an injection attempt** — correctly.
  Claude Code is an *agent with a repo context*, not a completion endpoint, and it inherited that
  context precisely because the fixture lives **inside** the repo. So CC-01/CC-02/CC-08 never ran.
  **Done:** CC-01 formats a contact as JSON, CC-02 writes a release note thanking a person at an org
  in a city (so it *must* carry Person/Org/Location), CC-08 generates a reminder list from the CSV
  (so it must restore the same placeholders across ~30 streamed lines). The rationale is promoted to
  [TESTING.md → the prompt-design box](TESTING.md#cc-prompt-design) — it governs every future
  scenario, not just these three.
  > **The design question underneath, worth answering once:** *how do you test PII masking with a
  > client that refuses suspicious prompts?* The answer is not to argue the agent out of its
  > judgement (a `CLAUDE.md` telling it to comply would work and would be the wrong fix — it makes
  > the test a special case of itself). It is to use prompts that are **plausible work**: read this
  > file and summarise it, format this contact as JSON, run this query. Which is *also* what real
  > Claude Code traffic looks like — so the rewrite makes the battery both runnable **and** more
  > representative. CC-03/04/06/09 are already this shape; the chat-only ones are not.
- [x] **Re-run the CC battery — CLOSED (2026-07-18).** Live run against real Anthropic through the proxy:
  masking (Run ON) leak-clean and de-mask (Run OFF) proven, both postures, DBG-02 = 0 throughout (incl. all
  3 secrets in CC-06 and the MCP tool-result in CC-09). The fail-closed non-convergence 400 (CC-08, then
  CC-05 and CC-09) was root-caused live via the instrumented diagnostic to NER sub-word fragmentation and
  fixed by **[S4](#m71)**; re-run on the S4 binary, **all three now converge** — **zero fixpoint 400 across
  the whole run.** The privacy property held on every turn, including the fail-closed blocks. Sub-items:
  - [x] **CC-08's non-convergence — resolved (2026-07-18).** The long-reminder-list turn hit a
    **fail-closed 400** ("masking did not reach a fixpoint in 4 passes"): the guard fired **correctly**
    (blocked before forwarding, zero leak), but a 400 on ordinary work is a real availability defect.
    **The suspected cause (NER re-tagging a placeholder) was disproven** — a live re-run converged, and
    ~10 offline reconstructions against the production composite (placeholder-dense fields, the raw CSV,
    the chunked path) all converge in ≤1 pass; the trigger is content-specific to that one session and
    stays unpinned **by choice**. Resolution (owner's call): **placeholder inertness now enforced *by
    construction*** — `mask_all`'s `keep_maskable` drops any detection that is one of our own `[KIND_N]`
    tokens, so the fixpoint converges regardless of the NER (M5-R4 upgraded from empirical to algorithmic;
    the `m5_r4` test stays as a model-swap canary) — **plus a value-free non-convergence diagnostic**
    (per-pass kind tally + residue kinds + `placeholder_tags_suppressed`) so any recurrence names its own
    cause. Tests FC-07 + unit; docs in `ARCHITECTURE.md` (fixpoint), `TESTING.md`, `DEVLOG.md`.
  - [x] **CC-09 — done, leak-clean (2026-07-18).** The `tool_result` masking test. The original
    `customer-lookup.sql` carried PII as **literals in the query text**, so reading it masked them before
    execution — the path it exists for never ran. Fixed: `fixtures/cc09-setup.sql` creates a synthetic
    `cc09_customers` (out-of-band, throwaway SQLite via the `python-sql` MCP server), and the agent runs
    the PII-free `SELECT * FROM cc09_customers` (`customer-lookup.sql` is now that). Verified: client saw
    `[EMAIL_2]/[PHONE_1]/[SSN_1]/[CARD_1]/[IBAN_1]/[SECRET_1]`, DBG-02 = 0 on all six raw values. TESTING /
    MANUAL_VERIFICATION / the two fixtures updated to match.
  - [x] **The Run OFF half — done (2026-07-18).** Each scenario needs **both** postures: ON shows the
    request left masked, OFF shows the client got it restored — only together do they prove the *same*
    round-trip. **OFF pre-S4:** CC-03 ✅, CC-04 ✅, CC-06 ✅, CC-07 ✅ (DBG-02 = 0); **CC-05 and CC-09 hit the
    fail-closed non-convergence 400** — the fragmentation bug, both pinned by the instrumented diagnostic
    (real `ORG`/`PER` fragments, `placeholder_tags_suppressed=0`), fixed by **[S4](#m71)**. **Re-run on the
    S4 binary (cache on), with the user:** CC-05 ✅, CC-09 ✅ — both now **converge** where they 400'd; CC-08
    ✅ both postures (OFF → real emails restored across 30 lines, ON → `[EMAIL_2/3/4]`). **Zero fixpoint 400
    across the whole S4 run; DBG-02 = 0 on every value.** CC-03/04/06/07 were leak-clean pre-S4 and S4 only
    ever masks *more* (full NER on pass 0 unchanged; it drops the NER only on later passes), so their OFF
    de-mask (unchanged code) carries. CC-01 / CC-02: already OFF+ON. **The battery is closed.**
  It needs a human at the keyboard with a live key and a real Claude Code — this environment has neither,
  so it cannot be automated away. Procedure: [`MANUAL_VERIFICATION.md`](MANUAL_VERIFICATION.md);
  **`NER_REQUIRED=1` is non-negotiable** (it is what makes a silently structured-only run fatal — the
  trap that made the first M6 live run test half the product).
  - [x] Per-turn latency recorded as a product figure next to the RAM ones in both READMEs —
    measured on the realistic fixture, not claimed.

> **Do not "fix" this with a build profile.** Measured: `release` (fat LTO, opt-level 3) buys **3%**
> — 27,728 ms → 26,863 ms on 29 KB. The cost is inside ONNX Runtime, a prebuilt native library that
> is already optimized; compiling *our* Rust harder changes nothing.

<a id="m7-ledger"></a>
### Review ledger — M7 → [`reviews/M7.md`](reviews/M7.md)
**Round 1 (2026-07-17): 7 findings — no leak, no fail-open, no detection regression. All closed**
(`89d2ca9` + the closure commit). The S0 refutation reproduces exactly; `intra_threads` was verified
**empirically inert** for detection (identical spans at intra 1…12), and the stop-at-the-bar
decision holds. The findings are about the **measurement and its record** — which, for a milestone
whose deliverable *is* a measurement, is where the risk lives.

**M7-R1 and M7-R2 are the pair to remember, because together they are this milestone's own lesson
turned back on it.** M7 exists because M5 measured the wrong *shape*; M7 then measured the right
shape **in the wrong configuration** (the harness defaulted the pool to 1, the server to 2 — so the
executable bar guarded a config nobody ships) and read conclusions off **single samples** whose
noise exceeded the effect. *Getting the corpus right does not save you if the measurement's design
is the next blind spot.* Both are now structural: `resolve_pool_and_intra` is the one home for the
default, so the harness cannot drift from the server; the sweep repeats and prints min/median/spread.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R1](reviews/M7.md#m7-r1) | The headline "2.17 s" and the executable bar measure `NER_POOL_SIZE=1`, which the server does not default to | measurement | [x] |
| [M7-R2](reviews/M7.md#m7-r2) | The sweep draws settled conclusions from n=1 runs; the SMT one does not replicate (sign flips; noise > effect) | measurement | [x] |
| [M7-R3](reviews/M7.md#m7-r3) | Nothing pins `intra_threads`' detection-inertness — every recall guard pins `intra=1`; verified true, guarded by nothing | invariant | [x] |
| [M7-R4](reviews/M7.md#m7-r4) | THREAD-01's `cores.max(pool)` silently exempts the one regime where the product *does* exceed the box | test-quality | [x] |
| [M7-R5](reviews/M7.md#m7-r5) | `NER_POOL_SIZE=0` isn't filtered like `NER_INTRA_THREADS=0`; the startup log then names a pool the process lacks | observ. | [x] |
| [M7-R6](reviews/M7.md#m7-r6) | The fixture is 22.3 KiB, not 22.8 KB — and the `ms/KB` columns use the other unit | docs | [x] |
| [M7-R7](reviews/M7.md#m7-r7) | The `(Organization, "An")` over-mask — deferring is right; its only durable home is the archive | tradeoff | [x] |

**Round 2 (2026-07-17) — closure verification: all 7 hold; 2 of them left a crack, and 3 findings
came out of it.** Re-verified the whole battery (85 default / **103** onnx lib, `fmt`/`clippy` clean
on both feature sets), proved `resolve_pool_and_intra` is genuinely the only place either knob is
resolved, and worked M7-R4's split grid by hand — **it is exactly equivalent to the 40-pair original,
so nothing was lost.** `intra_threads`' detection-inertness reproduced a second time (134 entities,
identical at intra 1…12 — **194** once [M7-R8](reviews/M7.md#m7-r8)'s fix made the guard actually reach
the chunked path, still identical).

**M7-R8 and M7-R9 are round 1's own findings arriving one level down, which is the thing to notice.**
R1 taught the milestone to name a number's **shape**; R9 is the number's *other* un-named variable —
the box's **power regime**, worth 1.6× where the shapes were worth 1.17×, so the bar fails at 3.9 s on
an idle reviewer box while a real 20% regression still ships green. R2 taught it to **repeat** a
measurement; the guard then used min-of-3, which by construction cannot see the between-run drift R2
diagnosed. And R3 asked for a chunked-path guard; R8 is that guard asserting **bytes** for a property
the code branches on in **tokens** — [M5-R10](reviews/M5.md#m5-r10)'s shape, and the M4 retrospective's
lesson 6 (*a quantity a test never varies is a quantity the test cannot see*), a third time.

**All 3 closed** (the round-2 closure commit). **M7-R9 confirmed itself while being fixed**, which is
the detail worth keeping: re-measured on **AC** at the user's prompting, the shipped default came back
at **4,933 ms** — worse than the reviewer's *battery* number, and 2× my own AC figure from hours
earlier. The absolute is simply not a property of the code on this box. The ratio, measured across all
three regimes, sat at **1.85× / 2.26× / 2.10×**.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R8](reviews/M7.md#m7-r8) | NER-THREAD-01's ">512-token chunked field" is 442 tokens and never chunks; its assert counts bytes for a token property | guard | [x] |
| [M7-R9](reviews/M7.md#m7-r9) | The bar's assert is decided by the box's power regime, which min-of-3 cannot see — 3.9 s on the reviewer's box; the headline never names the variable | measurement | [x] |
| [M7-R10](reviews/M7.md#m7-r10) | M7-R6's unit fix reached one number too far — `903 ms/K(i)B` now appears in both units in the same file | docs | [x] |

**Round 3 (2026-07-17) — closure verification: all 3 hold, and the ratio is the right instrument.**
Re-verified the battery (85 default / 103 onnx lib, `fmt`/`clippy` clean on both feature sets); drove
M7-R8's fix against the **real tokenizer** (`repeat(60)` = **662 tokens**, 2 windows — genuinely
chunked, and NER-THREAD-01 reproduced at **194 entities, identical at intra 1…12**, its third
independent confirmation and the first to cover the chunked path). **M7-R9's ratio survived a regime
nobody had tried**: the absolute moved **1.50×** between the harness's documented invocation and an
isolated run — same box, same AC state, 40 minutes apart — while the ratio held at **1.81× / 2.12×**.

**But that experiment is also [M7-R12](reviews/M7.md#m7-r12), and it is the round's finding.** R1
named the number's *shape*; R9 named its *power regime*; **the power regime does not order the
milestone's own data** — battery (3,943 ms) is **faster** than three of the four AC measurements
(4,757 / 4,841 / 4,933), and "throttled AC" was assigned post hoc from the number itself. *A variable
that cannot sort your observations is not the variable.* **The stop decision still stands — but on
the ratio, not on the bar**: across six measurements by two people the ~3 s bar was met **once**.

**All 6 closed** (the round-3 closure commit). **M7-R12 was decided by re-measuring under its own
prescription** — isolated (`--test-threads=1`), on verified AC, with the new calibration leg reporting
the box at **1.02× the reference**, i.e. demonstrably not slow: the shipped default came back at
**4,724 ms**, reproducing the reviewer's 4,757 independently. So **~4.7 s is the honest figure, the
`2.46 s` headline was the fastest of seven observations and has never reproduced, and the bar is
missed.** The READMEs, this file and DEVLOG now say so; the stop rests on the **ratio** (~1.7–2.3×
across every regime, floor asserted at ≥1.5×), which is the part that is about the code.

> **The pattern this milestone kept re-learning, four rounds deep, and it is the thing worth carrying
> out of M7.** R1: name the number's **shape**. R2: **repeat** the measurement. R9: name its **power
> state**. R12: *the power state doesn't order the data either* — and the variable that does is the
> harness running five benchmarks against each other. Each fix named one more variable and left the
> next one hidden **behind the qualification it had just added**. The escape was not a better label
> but a different instrument: a **ratio against an in-run calibration leg**, which needs no label
> because it cancels whatever the box is doing. *When you cannot enumerate the variables, stop
> naming them and measure against something that moves with them.*

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R11](reviews/M7.md#m7-r11) | The READMEs dropped the two-shape latency table as unsupportable; ROADMAP and DEVLOG still advertise it | docs | [x] |
| [M7-R12](reviews/M7.md#m7-r12) | "Power regime, nothing else" doesn't order its own data — battery beats 3 of 4 AC runs; the ~2.5 s claim is the best of six | measurement | [x] |
| [M7-R13](reviews/M7.md#m7-r13) | The ratio guard is vacuous at ≤2 cores, where the shipped default *is* `PRE_M7_SHAPE` — and it reports that as a regression | guard | [x] |
| [M7-R14](reviews/M7.md#m7-r14) | Both of the bar's constants are uncalibrated: the 8 s ceiling nearly fires on the documented command; the 1.5 floor keeps the 20% blindness | guard | [x] |
| [M7-R15](reviews/M7.md#m7-r15) | M7-R8's closure credits the chunked path with 60 entities that are 20 extra sentences | docs | [x] |
| [M7-R16](reviews/M7.md#m7-r16) | The byte-proxy assert M7-R8 killed is still alive 90 lines down, guarding the same property | guard | [x] |

**Round 4 (2026-07-17) — closure verification + the AC/battery correction (`2aad0cc`, docs-only): all
16 round-1–3 findings hold; the new framing is supported and independently corroborated.** Re-verified
the suite (85 default / **104** onnx lib, `fmt`/`clippy` clean on both feature sets); `m7_s2` green on
three isolated runs. `2aad0cc` claims only that the owner's statement *removes* the AC/battery
explanation — not that it explains the 2.5–7.1 s spread — and the READMEs, ROADMAP, DEVLOG and R12's
addendum all hold that line (the remaining spread is stated unaccounted-for except the measured 1.50×
test-concurrency factor). **Corroborated from the box the docs could not read:** its AC power overlay is
literally Windows' *"Best power efficiency"*, so "AC" and "battery-efficiency" are the same plan on this
machine, as the owner said. **Two new low-severity findings, both [M7-R11](reviews/M7.md#m7-r11)'s
pattern a fifth time** — a correction that stops one instance short.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R17](reviews/M7.md#m7-r17) | 2aad0cc's AC/battery correction reached the four doc headlines but not TESTING:533 or the harness source, which still cite "battery beat three AC runs" as distinct regimes | docs | [x] |
| [M7-R18](reviews/M7.md#m7-r18) | The advertised "~1.8–2.3× / 1.81–2.26× across every regime" default speedup is undercut by a clean isolated run (default 1.71–1.75×); the `1.5×` guard floor is unaffected | measurement | [x] |

**Round 5 (2026-07-17) — closure verification + convergence call: both round-4 findings hold; the
engineering is done.** Re-verified the suite (85 default / **104** onnx lib, `fmt`/`clippy` clean on
both feature sets); `ner_perf` and `m7_s2` green **isolated**. `e4a8163`'s source diff is
**comment/string only** — every constant, assert and derivation is byte-identical to round 3, so all
guards hold as verified. **M7-R17 holds** (the AC/battery framing now survives only in the ledger rows
and the dated review record, both allowed) and **M7-R18 holds and reproduced under me** — my box is
*fast* (pre-M7 4,461 ms vs the reference 10,100 ms → 0.44×) and the default ratio compressed to
**1.77×**, exactly R18's "a faster box pulls the ratio toward the floor". **The call: close M7 once
[M7-R19](reviews/M7.md#m7-r19) — a one-word docs fix — lands.** No leak, no fail-open, no regression,
no engineering gap; R19 is the single thing actually wrong (a stale `8 s` where the guard asserts
`15 s`), and everything else is correct, dated history that self-corrects, or hedged colour around the
`≥1.5×` floor. Not holding the milestone open for prose.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R19](reviews/M7.md#m7-r19) | TESTING's PERF-M7-05 summary still calls the sanity ceiling "8 s"; the guard asserts 15 s (M7-R14 raised it in round 3), and the same entry says so 25 lines down | docs | [x] |

**M7 review ledger closed — 19 findings across 5 rounds, all closed.** Round 5 was the convergence
round the workflow exists to reach: a single one-word docs fix, then nothing left that is *wrong*.
The two residual-cosmetic items the reviewer logged were dealt with — the `~1.7–2.2×`/`~1.7–2.3×`
typical-band split is unified to **~1.7–2.3×** (the widest observed, 2.26×), and the round-2 ROADMAP
narrative that "reads retired framing before its own correction" is **kept by deliberate choice**: it
is a problem→resolution passage whose two halves are adjacent paragraphs, and the tension carries the
milestone's own lesson (*I invented an explanation; the box's owner retired it in one line*). Flatten
it and the lesson goes with it. **The code has been byte-stable since round 3** (rounds 4–5 touched
only comments, strings and docs); no leak, no fail-open, no detection regression was ever found. What
remains open below is **not** a review finding — it is the one scope item that needs a human.

[**Review 6**](reviews/M7.md#review-6) **(2026-07-17) confirmed the ledger is at a fixed point** and
adds no finding: M7-R19's fix and the band unification both hold and are self-consistent across every
live surface, `6f27f05`'s source diff is a single one-word comment edit, and the isolated guards re-ran
green (85 default / 104 onnx lib; `m7_s2` 1.75×/2.01× ≥ the 1.5× floor; NER-THREAD-01 194 entities
identical at intra 1…12). The findings trended 3→3→2→1→**0**. **The ledger is closed and stays closed.**

**Round 7 ([2026-07-17](reviews/M7.md#review-7)) — the `NER_POOL_SIZE` default flip (2 → 1), a delta on
the closed milestone.** Verified independently (104 onnx / 85 default lib green, `clippy-onnx` clean); the
flip driven through the real model — an operator setting nothing resolved, *as of that round*, to
`pool=1 intra=12` (the whole box for a lone request, half the RAM), `NER_POOL_SIZE=2` gave the pooled
`2×6`, both clearing the then-shared `≥1.5×` floor. (Both shapes changed in
[M11 Track B](#m11-b) — `1×6` and `2×3` on that box, against per-shape floors.) No detection code changed; no leak, fail-open, over-mask or determinism impact — pool is a
concurrency/RAM knob only. The −23% throughput cost of `pool=1` is named honestly everywhere the trade is
discussed (ARCHITECTURE / TESTING / DEVLOG / both READMEs). **One docs finding:** the delta updated every
doc *except this file*, so the M7 body still advertises the retired `pool=2` default in two current-state
claims.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R20](reviews/M7.md#m7-r20) | The ROADMAP M7 body still advertises the pre-flip `pool=2` default — "not a trade against the shared-proxy case at all" and "zero below 4 cores" now misdescribe the shipped `pool=1` | docs | [x] |

**Round 8 ([2026-07-18](reviews/M7.md#review-8)) — CC-08's resolution (`6cbd461`): placeholder inertness
*by construction*.** Verified independently (107 onnx / 88 default lib green, both `clippy` clean; the
`#[ignore]`d `m5_r4` inertness canary re-run against the **real cached model** — zero entities on a chunked
placeholder field). **The safety claim holds and I could not break it:** `detect_maskable` decides on
`entity.text`, and `entity.text == input[entity.span]` for every entity reaching it (NER re-slices in
`ner_decode`; structured/merged spans re-slice in `overlap::materialize`), so dropping a placeholder-shaped
detection can never strand real PII — a real value is never bracket-wrapped. The final-confirmation filter
does not weaken M4-R20 (real residue is never bracket-shaped, so it still blocks); the diagnostic is
genuinely value-free (labels/counts only). No leak, no fail-open, no round-trip or over-mask regression.
**One low-severity finding:** the `placeholder_tags_suppressed` canary is emitted only on the fail-closed
branch, so the docs' "makes a filter-leaning model visible" claim doesn't hold in the converging happy path.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R21](reviews/M7.md#m7-r21) | `placeholder_tags_suppressed` is logged only on the fail-closed branch — the "makes a filter-leaning model visible" claim (ARCHITECTURE / TESTING NER-INERT-01) is unsupported in the converging happy path; the `m5_r4` test is the real canary | observ. | [x] |

**Round 9 ([2026-07-18](reviews/M7.md#review-9)) — M7.1 landing (S3 cache + S4 fixpoint NER fix).**
Verified independently (116 onnx / 97 default lib green, the S3 e2e among 15 `proxy_e2e`, `clippy-onnx`
clean; the live `m7_s4_…converge_instead_of_400` + `m5_r4` inertness driven against the real cached
XLM-R). **Both features are fail-closed-sound and I could not break either.** S3: `try_detect` is a pure
function keyed on the whole input, spans index the same bytes on a hit, the two-generation map is bounded
to `2·cap`, the `if let` lock guard drops before the re-lock (no deadlock) and detection runs off-lock
(no poisoning) — a hit can never mask less. S4: the structured recognizers run on **every** pass and the
M4-R20 confirm, so the fail-closed layer's block-on-non-convergence guarantee is intact; masking can
expose only *structured* PII (never a name), so dropping the NER after pass 0 is a **recall** call inside
the layer that owns recall, and `NER_REQUIRED` (a *failure* switch, not a recall promise) is unweakened;
`FailOpen::redetect` correctly delegates so a wrapped NER stays idempotent. No leak, no fail-open, no
over-mask or determinism regression. **Two low-severity docs/observability findings**, each a later
commit leaving an earlier claim stale.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M7-R22](reviews/M7.md#m7-r22) | S4's `detect_maskable` → `keep_maskable` rename left a broken intra-doc link in source + four current-design references (ARCHITECTURE / TESTING ×2 / ROADMAP) naming the old symbol | docs | [x] |
| [M7-R23](reviews/M7.md#m7-r23) | S4 keeps the NER out of every masked pass, so M7-R21's runtime `placeholder_tags_suppressed` canary can't observe a filter-leaning *idempotent* NER (the GLiNER case the docs cite); FC-08 only passes on a non-idempotent fake S4 forbids; `m5_r4` is the durable canary | observ. | [x] |

<a id="m71"></a>
## M7.1 — system-prompt cache (S3) & the fixpoint NER fix (S4) ✅

**Both done 2026-07-18 — and one of them turned out to be a bug, not just a speed-up.** M7 shipped its
reproducible ~2× and stopped at a ~4.7 s turn (the bar was missed on purpose — see M7's *"Declare the bar,
then decide against it"*). Both items here were deferred rather than dropped because each traded a real
risk that had to be argued first; both have now landed with that argument answered. **S4** turned out to be
the *fix* for the CC-05/CC-08 fail-closed 400 — the NER re-running every fixpoint pass is what re-tagged its
own sub-word fragments past the bound on a dense system prompt — with its recall risk measured (0 losses).
**S3** caches the byte-identical system prompt's detection, keyed on the exact bytes so a hit can never mask
less. With S4 in, the CC battery's non-convergence blocker is resolved (re-run pending on the S4 binary).

- [x] **S3 — don't re-scan an unchanged system prompt every turn. Done 2026-07-18.** *(The named lead: the
  ~4.7 s turn and the ~40 KB-traffic case both point here.)* Claude Code re-sends 20–40 KB of system prompt
  + tool schemas every turn, **byte-identical**. `CachingDetector` (`src/pii/cache.rs`) wraps the composite
  and memoizes `try_detect` **keyed on the exact field bytes**, so turn 2+ skips the scan; the per-request
  vault still mints placeholders, so numbering is unchanged. **The threat argument the risk demanded is the
  design:** `try_detect` is a pure function of its input and the key is the whole input, so a hit returns
  exactly what a fresh scan would — it **can never mask less**. Only `Ok` results cached (an error still
  fails closed); bounded (two-generation, ~`2×` `PII_CACHE_ENTRIES`, fields 256 B–128 KiB); `redetect` never
  cached; `PII_CACHE_ENTRIES=0` disables it. Tested unit (CACHE-01) + e2e
  (`e2e_cache_on_a_repeated_large_field_still_masks_both_times`). Default **on** (16 entries).
- [x] **S4 — NER on pass 0 only; structured recognizers on later passes. Now the *fix* for the CC-05/CC-08
  non-convergence, not just a latency lead.** **Done 2026-07-18** — a `redetect` method on `PiiDetector`
  (default `try_detect`, the NER overrides it to return nothing), `mask_all` calls it on every pass after
  the first *and* the fixpoint confirm; the real model now converges on dense system-prompt text that
  400'd before (`ner_perf.rs::m7_s4_dense_org_names_converge_instead_of_400`), plus deterministic
  `Fragmenter` unit tests (FC-09). *(Investigated 2026-07-18; DEVLOG.)* Passes ≥2 of the fixpoint
  exist to catch masking **exposing** PII (a masked phone splits a digit run → a card) — a
  **deterministic-recognizer** phenomenon. Masking a name to `[PERSON_1]` never reveals a new name, so the
  NER buys nothing after pass 0 — **but it is exactly what keeps the loop from converging.** The NER tags
  *sub-word fragments* (`"lack"` of `"Slack"`, `"An"` of `"Anthropic"`, [M7-R7](reviews/M7.md#m7-r7)); each
  mask splits the word and the next pass re-tags the leftover. On a real Claude Code **system prompt** —
  dense with `Slack`/`GitHub`/`Claude`/`Anthropic` — this needed **> `MAX_MASK_PASSES` (4)** and **400'd on
  live traffic** (CC-05; the diagnostic pinned it: `per_pass=[[ORG 6, PER 2],[ORG 2],[PER 1],[PER 1]]`,
  `placeholder_tags_suppressed=0` — real fragments, not placeholders). M7-R7 called this "a latency cost,
  not a correctness one" — **that was wrong**: past 4 passes it is a fail-closed availability failure.
  - **Measured (offline, production composite).** PLAIN passes **grow with the dense text**: 6 (5 KB) → 11
    (15 KB) → 13 (30 KB), unbounded — so a **bound bump is out** (no safe fixed value, and each pass is a
    full NER scan). **S4 converges in 1 pass at every size.** A **word-boundary snap** of NER spans was
    tried and **rejected** — it makes convergence *worse* (8 passes vs 4).
  - **The recall risk is now answered (the argument M7-R7 required).** Dropping the NER after pass 0 could,
    in principle, miss a name a later pass would have exposed at a seam (`[PERSON_1]-Jones`). Measured: **0**
    losses — S4 masks every expected entity PLAIN does across the labelled corpus (25 NER entities, 15
    cases), and **0** raw PII survives when real names/emails/IBAN are injected into fragmenting dense text.
    Masking only ever *reduces* NER context, so later passes discover no genuinely-new names — only the
    fragments they created.
  - **Implementation.** A `redetect` method on `PiiDetector` (default = `try_detect`); `OnnxNerDetector`
    overrides it to return empty (idempotent after pass 0); `CompositeDetector::redetect` runs the
    structured recognizers only; `Vault::mask_all` uses `try_detect` on pass 0 and `redetect` after (and for
    the fixpoint confirm). Fold in the latency win M4-R21 priced (~940 ms of a 4.2 s turn). Builder→reviewer.

### Landing & release — `v1.0.0` (2026-07-18)

**Reviewed as round 9 of the [M7 ledger](reviews/M7.md)** (the round that lands M7.1's S3 + S4): 116 onnx / 97
default lib green, clippy clean, findings R22/R23 closed — M7.1 has no separate ledger, its review lives
there. With that, the **first tagged release** was cut here, off M7.1: **tag `v1.0.0` on `main`**, `Cargo.toml`
at `1.0.0`, after the CC battery closed on the S4 binary. Every gate met — the M6 native route + M7 latency +
M7.1 (S3/S4), both postures (`openai` / `anthropic`) leak-clean, **zero fixpoint 400** on the S4 binary.
(Superseded by `1.1.0` at [M8](#m8); a release tag is noted next to its milestone in the Status table, not as
its own row.)

<a id="m8"></a>
## M8 — GLiNER: contextual / open-label PII detection ✅

**Promoted from Backlog 2026-07-18.** The path for **ambiguous, anchor-less PII** the deterministic layer
can't disambiguate (a bare national phone, a free-form postal address) and the current XLM-R (PER/ORG/LOC
only) doesn't cover. GLiNER is a *zero-shot, open-label* span extractor — you pass labels like `"person"` /
`"phone number"` / `"address"` at inference and it matches by **context** — so it is a candidate
**successor** to XLM-R (named entities *and* contextual PII in one model), not merely an add-on. Full
implementation plan (stages, files, the span-decode design): **[DEVLOG 2026-07-18](DEVLOG.md) → *M8 GLiNER
implementation plan*.** Model landscape in [`M2-NER-EVALUATION.md`](M2-NER-EVALUATION.md).

> **GLiNER ≠ Piiranha, and that difference is why this is open.** Piiranha (a *fixed-label* mDeBERTa
> token-classifier) was **measured and rejected** at M2 (~0 recall on natural sentences — it fired on
> subword fragments). GLiNER is the *opposite* architecture: open-label span scoring, matched by context.
> It was set aside at M2 for *first-version scaffolding simplicity* — a separate detector was more
> integration than a first pass warranted — **not** for any capability doubt. **When this is picked up, the
> evolution to evaluate is GLiNER, not Piiranha.**

**Two accepted over-masks this targets — different bugs, both named because a model change touches both:**
- [M4-R6](reviews/M4.md#m4-r6) — a **deterministic recognizer** over-matching pure-numeric IDs (~18% of
  9-digit tokens). Its path out is **context**, GLiNER's whole premise. A *precision* gain, not the primary
  goal (recall stays metric #1).
- **M7's `(Organization, "An")`** ([M7-R7](reviews/M7.md#m7-r7)) — the **NER** emitting a two-character
  fragment of `"Anthropic's"`, so every Claude Code system prompt reaches the model as `"[ORG_1]thropic's"`.
  A **span-quality** problem a better model fixes at the source. **S4 already made it non-fatal** (the NER
  runs once, so the fragment no longer chains the fixpoint into a 400); M8 could make it non-existent.

**And the recall frontier it opens — the ex-Backlog *"Locale phone national formats"*, folded in here.**
The two over-masks above are *precision*; GLiNER's larger prize is **recall on PII the platform cannot
catch today**. A bare national phone with no `+CC` anchor (UK `020 …`, DE `030 …`) collides with ordinary
number sequences, so the deterministic layer deliberately does **not** match it (the
`fp_prone_recognizers(code)` seam was wired and empty then, gated by `PII_LOCALES`) and XLM-R doesn't cover
phone numbers at all — so it goes **upstream unmasked**. GLiNER's `"phone number"` label was the presumed
clean path here — but a post-merge study found a *deterministic* one that beats it, and **[M8.1](#m81)**
filled the seam for GB/DE with the `phonenumber` crate's assigned-range validation instead (GLiNER keeps the
free-form address, which genuinely needs context). The address remains the S2 eval's contextual case.

**Measure first — the milestone is empowered to say no.** Same non-negotiable as M2: no heavy model
pre-emptively. GLiNER is scored through the **hybrid resolver** (not the NER alone), int8, against the lean
CPU bar, *before* it is adopted — and the eval can reject it, exactly as M7 stopped at its bar and Piiranha
was rejected at M2. The decision it produces — **successor** (replace XLM-R), **addition** (run alongside
for contextual kinds only), or **rejected** — gates everything downstream.

**Done 2026-07-19 — measured, and shipped opt-in (the verdict is *addition, not successor*).** Built and
validated **end-to-end against the real model** (`onnx-community/gliner_multi_pii-v1` int8): a
`GLiNerDetector` + `gliner_decode` behind the `PiiDetector` trait, wired **opt-in** via `GLINER_MODEL_PATH`.
At the measured-optimal threshold (**0.15**) int8 GLiNER matches XLM-R on **Location (0.91)** and
**Organization (1.00)** — but its **Person recall is 0.58 vs XLM-R's 0.83** (single-word / CJK / Arabic
names it never scores), so replacing XLM-R would regress the most important kind. **So XLM-R stays the
default NER; GLiNER ships off by default**, enabled to add the contextual, open-label kinds XLM-R can't — a
**bare national phone** (no `+CC`) and a free-form address. Full numbers + threshold sweep + the decision:
[DEVLOG 2026-07-19](DEVLOG.md). **The verdict is now measured across the whole quantization spread**
(int8 / fp16 / fp32): less aggressive quantization lifts Person recall 0.58 → 0.67 (fp16 ≡ fp32, since ORT
up-casts fp16→fp32 on CPU — so fp16 is the higher-recall option, ~580 MB; fp32 is pointless on CPU) but is
**still < XLM-R's 0.83** — the residual gap is the *model*, not the quantization. Review-clean (3 rounds,
7 findings).

### Scope
- [x] **The ONNX I/O contract, verified from the real export first (S0).** GLiNER is **not**
  token-classification: its graph takes extra inputs (the entity types as text + word/span masks) and emits
  **span logits**, and the exact input/output names & shapes differ between community exports. Pin
  `onnx-community/gliner_multi_pii-v1` at a revision and *document* its contract before building the decode
  — building against a guessed contract is the trap.
- [x] **`GLiNerDetector` + a span-decode path (S1).** A new detector (`src/pii/gliner.rs`) behind the `onnx`
  feature, and a model-independent `gliner_decode` ((span, label, score) → threshold → greedy non-overlap →
  `PiiEntity`), unit-tested without a model — the span×label analogue of today's BIO `ner_decode`. Slots
  into `CompositeDetector` behind the `PiiDetector` trait like `OnnxNerDetector`, so the pipeline is
  untouched.
- [x] **Open-label config → `PiiKind` map (S1).** `GLINER_LABELS` is a list of natural-language types
  (`"person"`, `"organization"`, `"location"`, `"phone number"`, …), each mapped to a `PiiKind`. Start by
  mapping to **existing kinds** (phone number → `Phone`, address → `Location`) so the placeholder vocabulary
  and de-mask are unchanged; a genuinely-new kind (a free-form `Address`) is a deliberate `PiiKind` addition
  the eval must justify — it ripples through `label`/`from_label`/`priority`/`is_structured`.
- [x] **The measured eval + the decision (S2).** `tests/gliner_eval.rs` (`#[ignore]`, `--features onnx`)
  scores GLiNER int8 through the hybrid on an **extended** `ner_cases.json` (add contextual-PII cases: bare
  national phones, free-form addresses, single-word names) — recall / precision / F1 per type + CPU latency /
  RAM / size vs XLM-R int8. **Recall is metric #1** (a miss is a leak); to be a *successor* it must at least
  match XLM-R's M4 10-language floor (Person 0.83 / Org 1.00 / Loc 0.91). Numbers → DEVLOG.
- [x] **Chunking with the shared label-prefix budget (S3).** GLiNER prepends the labels to **every** window,
  so the usable text budget is `model_max − prefix − specials − drift`, not the whole sequence — the more
  labels, the smaller the window. Port M5's chunking discipline (the compile-time headroom invariant
  [M5-R2](reviews/M5.md#m5-r2)/[M5-R10](reviews/M5.md#m5-r10), enforcement at the single choke point)
  recomputed for a budget the labels eat into. Guard: every re-tokenized window + its prefix stays under
  GLiNER's max.
- [x] **Wire into load + the model-swap canaries (S4).** Extend `load_onnx_ner`/`hf.rs` for the pinned repo
  (revision-pinned fetch; a community conversion — scoring against the corpus *is* the trust check). Fail
  closed under `NER_REQUIRED`, `FailOpen`-wrapped otherwise (the [M5-R7](reviews/M5.md#m5-r7) rule holds: a
  threshold may degrade GLiNER's own recall, never decide the caller's posture). Override `redetect` → empty
  **with the 0-loss recall measurement re-run for GLiNER** (S4's argument is per-model), and **re-run the
  `m5_r4` placeholder-inertness canary against the real GLiNER model** — the docs already flag "GLiNER
  especially" ([M5-R4](reviews/M5.md#m5-r4), [M7-R23](reviews/M7.md#m7-r23)); inertness is enforced by
  construction since S4, but the canary is how a filter-leaning idempotent model is caught.
- [x] **Docs + builder→reviewer (S5).** ARCHITECTURE (the span decode, the label config, the not-a-successor
  decision), TESTING (the smoke / eval / inertness-canary harness), READMEs (the new env + the detection
  matrix), DEVLOG. **Reviewer loop closed** (3 rounds, 7 findings, all closed — see the ledger below).

> **If the CPU latency misses the lean bar, that is the trigger for [M9](#m9), not a reason to ship slow.**
> GLiNER int8 is heavier than XLM-R int8; the escalation path
> ([`M2-NER-EVALUATION.md`](M2-NER-EVALUATION.md)) is explicit that a GPU EP is how a heavier model earns
> its place — so M8's eval is what may *pull M9 forward*.

### Review ledger — M8 → [`reviews/M8.md`](reviews/M8.md)
**Builder→reviewer loop CLOSED (2026-07-19) — 7 findings across 3 rounds (5 → 2 → 0), all closed; none a
leak, none a fail-open regression.** Findings recorded here as a compact ledger (id · title · sev ·
status); the full entry + fix + closure note lives in [`reviews/M8.md`](reviews/M8.md).

**Round 1 (2026-07-19): 5 findings — none a leak, none a fail-open regression.** Verified independently:
108 default / 132 onnx lib green, clippy + fmt clean, and all three gated GLiNER tests reproduced against
the real int8 model (Person 0.583 / Org 1.00 / Loc 0.909 — the DEVLOG numbers to the digit; the `markerV0`
tensor contract decodes correctly on the real export). The fixpoint is **provably safe** with GLiNER in the
composite (convergence rests on S4's `redetect → empty`, not on `keep_maskable`), a GLiNER `Phone` flows
soundly through the union-merge, and *addition, not successor* is honestly recorded. Findings are
hardening / test-coverage / docs.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M8-R1](reviews/M8.md#m8-r1) | GLiNER chunking has no choke-point ceiling guard — re-tokenized `seq` never checked vs `max_len` (the S3 "port M5's discipline" item, only half-delivered) | hardening | [x] |
| [M8-R2](reviews/M8.md#m8-r2) | GLiNER's multi-window chunking path has never run against the real model (all gated inputs are single-window) | test-cov | [x] |
| [M8-R3](reviews/M8.md#m8-r3) | ARCHITECTURE says a GLiNER guess "loses in overlap" — false for its `Phone`, which is `is_structured()` and is union-merged | docs | [x] |
| [M8-R4](reviews/M8.md#m8-r4) | `gliner_decode.rs` had 11 unit tests; a 12th was added to match DEVLOG/TESTING | docs | [x] |
| [M8-R5](reviews/M8.md#m8-r5) | `load_gliner` silently disables GLiNER on partial config / an out-of-range `GLINER_THRESHOLD` | hardening | [x] |

**Round 2 (2026-07-19): closure verification — all 5 hold.** Re-verified 109 default / 133 onnx lib green
(clean shell), clippy + fmt clean, all four gated tests reproduced on the real int8 model. The R1/R2 window
cap is **correct and recall-preserving** — independently confirmed with a throwaway real-model probe that a
name is detected at every offset in a capped window and mid-way through multi-window fields. Two new
findings, both low, neither a leak.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M8-R6](reviews/M8.md#m8-r6) | `required_ner_is_fatal_when_absent` is non-hermetic — ambient `GLINER_MODEL_PATH` (the M8 gated-test env) breaks the onnx lib suite | test-quality | [x] |
| [M8-R7](reviews/M8.md#m8-r7) | window-cap recall rationale mis-explains *why* it works (measured: bounded context, not overlap-near-start); the two knobs have different jobs | docs | [x] |

**Round 3 (2026-07-19): both closures verified — [M8 review-clean](reviews/M8.md#review-3).** Reproduced the
R6 repro then confirmed 133/0 with the gated env set; R7 rationale corrected. No new findings. All seven
M8 findings closed; no leak, fail-closed intact, fixpoint safe, window cap recall-preserving, decision
honest.

<a id="m81"></a>
### M8.1 — national phone recognizer (opt-in, per-locale) ✅

**Post-merge follow-up to M8 (2026-07-19). M8 pointed the ex-Backlog *"Locale phone national formats"* gap
at GLiNER's context as the clean path; a feasibility study found a *deterministic* path that is better on
every axis, so M8.1 takes it instead.** The un-anchored domestic phone (GB `020 7946 0958`, DE
`030 12345678`) has no `+CC` anchor and collides with order numbers / sort codes / national IDs, so the
deterministic layer deliberately didn't match it and XLM-R doesn't cover phone at all — it went upstream in
clear. The study (throwaway probe, then reproduced in-tree) showed the pure-Rust **`phonenumber`** crate
(libphonenumber port) closes it: a loose `0`-trunk regex proposes a candidate, and `is_valid()` accepts it
only if it is a **real, assigned number** for the region — the assigned-prefix + length check a
hand-written regex can never do, which is exactly what makes the un-anchored form FP-prone.

**Why deterministic beats GLiNER here (and doesn't make M8 wasted):** no second ML model to load, no
inference latency, no recall gap — and it runs in the **default** build. GLiNER keeps its real role
(names / orgs / free-form address — the open-label kinds no recognizer covers); only its *phone* motivation
is now better served here. Measured, faithful two-stage model (loose regex → `is_valid`) on an adversarial
corpus of 22 real GB/DE nationals + 22 phone-shaped non-phones (sequential digits, all-zeros, sort codes,
refs, unassigned prefixes, national IDs, dates, a Luhn card):

| locale | precision | recall | FP-rate | note |
|---|---|---|---|---|
| **GB** | **1.000** | 0.917 | **0.000** | one FN = the Ofcom fiction range `07700 900123` (libphonenumber rejects it — not a real number) |
| **DE** | 0.909 | 1.000 | 0.100 | one "FP" = `0049301234` = `00 49 30 1234`, literally an international dial to Berlin |

Numbers + the footprint analysis: **[DEVLOG 2026-07-19](DEVLOG.md) → *M8.1***.

### Scope
- [x] **Feasibility study first** — pure-Rust footprint (native-dep-free bar holds: no `*-sys`, nothing on
  `dependency_footprint`'s forbidden list) + precision on an adversarial mixed corpus, per locale. Decision:
  GO (deterministic, high precision, no second model).
- [x] **`phonenumber` in the DEFAULT build.** Pure Rust, so it does not breach the native-dep-free guarantee;
  accepted cost is ~3 MB embedded worldwide metadata + a few unmaintained transitive deps (`oncemutex`,
  `regex-cache`) — recorded as a tradeoff in ARCHITECTURE, guard stays green.
- [x] **GB/DE recognizers in `fp_prone_recognizers`, gated by `PII_LOCALES`.** A shared bounded-group
  `0`-trunk regex (compact + 2-/3-group forms, so a match can't swallow an adjacent number) with a
  per-locale `is_valid()` validator; `PiiKind::Phone`, `Scan::Overlapping` (length-bounded → linear).
- [x] **Adversarial tests** — detection across shapes, `PII_LOCALES` gating (off ⇒ not masked), validator
  rejects look-alikes, no adjacent-number swallowing, direct validator unit tests. 115 default lib green,
  clippy + fmt clean.
- [x] **Docs** — ARCHITECTURE (the recognizer + the accepted dep tradeoff + "the validator is not a locale
  discriminator: numbering plans overlap, which is privacy-safe"), TESTING (the new cases), READMEs
  (`PII_LOCALES` now enables GB/DE phone; the binary-size note), DEVLOG, this section.
- [x] **Builder→reviewer loop** — round 1 (2026-07-19): 1 finding (M8-R8, hardening, not a leak), closed;
  **review-clean**.

### Review ledger — M8.1 → [`reviews/M8.md`](reviews/M8.md)
**Round 4 (2026-07-19): 1 finding, not a leak.** Verified independently on both feature sets — 115 default /
139 onnx lib green, no warnings, `clippy`(+`-onnx -D warnings`)/`fmt` clean; native-dep-free bar holds
(`phonenumber` is pure Rust, unmaintained transitives honestly recorded); gating real, fail-closed/overlap
sound, locale non-discriminator reproduced. Fuzzed the swallow guard against the real `Vault::mask_all`:
adjacent/hyphen/mixed all split and round-trip. One blind spot found — **not a runtime leak** (the fixpoint
covers it, verified).

| ID | Title | Sev | Status |
|---|---|---|---|
| [M8-R8](reviews/M8.md#m8-r8) | Swallow guard blind spot: a longer *invalid* arm-1 match shadows the first of two adjacent numbers; only the **fixpoint** (not the bounded regex) prevents the single-pass miss — safety misattributed, latent if `redetect` is ever shortcut | hardening | [x] |

**Round 4 closed (2026-07-19).** Builder fixed source: the `national_phone_recognizer` doc no longer claims
the bounded groups prevent a cross-boundary match (they bound the *length*); the real backstop — the
`mask_all` fixpoint — is named, and a `mask_all`-level test (`adjacent_national_phones_are_both_masked_by_the_fixpoint`)
now pins it, so a future `redetect` shortcut fails there instead of leaking. Rule promoted to
ARCHITECTURE (next to `Scan`/fixpoint). **116 default lib green. M8.1 review-clean.**

<a id="m9"></a>
## M9 — GPU optimization

**Opened 2026-07-19** *(promoted from Backlog 2026-07-18; was slated for M4, deferred 2026-07-12)*.
Faster inference once the model is locked. The M2 model choice is **execution-provider-agnostic**
(standard ONNX, runs on any `ort` EP), so GPU constrains nothing upstream. This box has an **AMD DX12
iGPU**, which decides the backend by elimination: CUDA is NVIDIA-only, ONNX Runtime has no Vulkan EP,
so **DirectML is the only GPU path here** (and the only vendor-agnostic one on Windows, being D3D12).

- [x] GPU execution provider behind config — `NER_EXECUTION_PROVIDER` selects the backend for both
  the NER and GLiNER, with a logged **CPU fallback** and a homogeneous session pool
  (`onnx::build_session_pool`). Only **DirectML has been benchmarked** (a NO-GO on this iGPU) and
  **no accelerator has passed the determinism guard** — the measured/trusted split and what each
  platform actually gets: [ARCHITECTURE](ARCHITECTURE.md) → *Execution providers*.
  (DEVLOG 2026-07-19.)
- [x] Quantization tuning; benchmark GPU-fp16 vs the CPU-int8 baseline — **measured: NO-GO on this
  iGPU** (below). CPU-int8 stays the default.
- [x] `--bench-providers` — the binary measures the **model × provider** matrix on the operator's own
  machine and names the winner, because the answer is hardware-specific. (DEVLOG 2026-07-19.)
- [x] **Per-platform accelerator, wired per-target** — `--features onnx` ships the platform's GPU
  (DirectML on Windows both arches, CoreML on macOS, CUDA on `x86_64` Linux), so it needs no special
  build and a dev build cannot disagree with the release pipeline. Measured first: "compile in every
  backend" is impossible — one ONNX Runtime distribution carries one provider set (six `ep-*`
  features link, but five report unavailable at runtime). Which platforms
  were safe to wire came from `ort-sys`'s `resolve_dist`, not from guessing: DirectML/CoreML are not
  distribution keys (no-ops on the artifact), CUDA is — and has no arm64-Linux prebuilt, so that
  target is deliberately left alone. Provider discovery is therefore a **runtime** query
  (`is_available`), not a cargo-feature inference. (DEVLOG 2026-07-20.)

> **Verdict: DirectML is NO-GO on this hardware, and that is a completed milestone, not a failure.**
> DML-fp16 vs CPU-int8 measured **1.45× / 0.45× / 0.38×** at seq 128 / 256 / 512. The GPU wins only
> where latency is already invisible and loses ~2.6× at the **operating point** — fields are chunked
> to 480 tokens, so the latency-dominant inferences run at seq ~480–512. A shared-memory iGPU is
> bandwidth-bound; 12 CPU threads on int8 beat it. **This is a fact about this iGPU, not about GPUs:**
> a discrete GPU would very likely win, and the selector makes that a config flip. Full matrix and
> mechanism: DEVLOG 2026-07-19; design: [ARCHITECTURE](ARCHITECTURE.md) → *Execution providers*.

> **The measurement that nearly went wrong, now encoded in the tool.** Benchmarking the *shipped int8*
> model on DirectML showed 2–5× slower and looked like the verdict; it was a **false negative** (int8
> partitions badly onto GPU providers). At fp16 the same GPU went from 5× slower to 1.45× faster.
> Backend and quantization are **coupled**, so `--bench-providers` measures both axes and refuses to
> present an int8-only run as an answer to "is the GPU worth it?".

> **The M8 trigger did not fire as anticipated.** GLiNER was rejected as a successor on **recall**,
> not latency ([M8](#m8)), and shipped opt-in — so M9 is an **enabler** (make fp16-GLiNER's
> higher-recall path viable at speed; optionally cut the ~4.7 s XLM-R turn), not a latency rescue.
> XLM-R has no pre-shipped `model_fp16.onnx` (only `model.onnx` fp32 + int8); GLiNER's fp16 is on HF.

- [x] **Builder→reviewer loop CLOSED (2026-07-20) — 29 findings across 9 rounds, all closed; none a leak.**
  Round 1 (2026-07-20): 11 findings, **none a leak**. Verified
  independently: 143 lib + all integration tests green, `clippy-onnx` / `clippy-directml` clean, and the
  DirectML verdict **reproduced with the shipped tool** (CPU-int8 27.1/46.5/108.4 ms vs DML-int8
  101.1/346.1/599.3 ms). Round 2 ([reviews/M9.md](reviews/M9.md#review-2)): **all 11 closed and
  verified on the real binary** (the fallback line now reads `provider="cpu" requested="rocm"`);
  4 new findings on the per-target accelerator, **none a leak**. Round 3
  ([reviews/M9.md](reviews/M9.md#review-3)): R12–R15 closed, and the CUDA narrowing **verified against
  the resolver** — `cargo tree` per release triple confirms `directml` on both Windows arches, `cuda`
  on `x86_64` Linux, `coreml` on macOS, and **no accelerator on `aarch64-unknown-linux-gnu`**, exactly
  as claimed; R1/R2 re-verified un-regressed on the real binary. 4 new findings, **none a leak** —
  three are one stale description of the EP layer surviving in places the closures didn't grep.
  Round 4 ([reviews/M9.md](reviews/M9.md#review-4)): R16–R19 closed and the whole green bar
  reproduced (116 / 144 lib, zero warnings, both clippy legs clean); mask → inject → restore driven
  on the **real binary** against a mock upstream, with no raw PII in the logs; the rewritten
  `Cargo.toml` block's `ort-sys` claims re-derived from the crate source — three of four exact.
  3 new findings, **none a leak**: a guard that passes vacuously, the same stale sentence in two more
  documents, and one inherited factual claim the experiment behind it doesn't support. **The M9 code
  is finished; what remains is docs and one test.**
  Round 5 ([reviews/M9.md](reviews/M9.md#review-5)): R20–R22 closed at the sites they quoted, but two
  closures stopped short of their own prescribed site lists — 3 new findings, **none a leak**.
  Round 6 ([reviews/M9.md](reviews/M9.md#review-6)): R23–R25 closed, R25 fixed **structurally** (the
  messages moved onto the testable side of `cfg!`); green bar reproduced (116 / 147 lib, zero warnings,
  `fmt --check` and both clippy legs clean) and mask → inject → restore re-driven on the **real binary**
  with no raw PII in the logs. The `webgpu` sweep holds for every phrasing of the literal claim.
  2 new findings, **none a leak**: the one site R24 itself numbered "Sixth", and a guard whose
  distinctness assertion cannot see the failure its own comment cites — reproduced by mutation.
  Round 7 ([reviews/M9.md](reviews/M9.md#review-7)): **R26 and R27 closed — M9's own work is review-clean.**
  R27 re-verified by re-running the reviewer's mutation (BENCH-02 now fails it) plus a new one on the
  selection half (BENCH-03 fails an inverted Linux ordering); green bar reproduced (116 / **148** lib, zero
  warnings, `fmt --check` and both clippy legs clean); mask → inject → restore driven on the **real binary**
  at `RUST_LOG=debug` with reviewer-chosen values — byte-exact restore, no raw PII anywhere. The `obtainable`
  predicate grep comes back clean for the **first time**: all three build-config/design files now agree.
  1 new finding, **not a leak and not M9's** — a pre-existing race in the test log-capture helper
  ([M9-R28](reviews/M9.md#m9-r28)), reproduced at 11%, whose real cost is a sibling guard that goes vacuous.
  Round 8 ([reviews/M9.md](reviews/M9.md#review-8)): **R28 verified closed on both halves.** The builder's
  first fix (the `Mutex` this review had prescribed) was measured failing at 7/30 and replaced with a
  process-global subscriber — the right call, and the mechanism is confirmed in the vendored `tracing-core`,
  not merely inferred. Verified with a **validated** instrument: the flake reproduced on the pre-fix binary at
  **7.5%** (3/40), then **178 clean runs** on the fix across default, CPU-saturated, `onnx`, and eight
  `--test-threads` settings; and the de-vacuified guard confirmed by running the same dead-capture **mutation**
  against both revisions — the old test reports `ok`, the new one fails loudly. 1 new finding, **not a leak**:
  the helper's rustdoc still documents the abandoned lock ([M9-R29](reviews/M9.md#m9-r29)).
  Round 9 ([reviews/M9.md](reviews/M9.md#review-9)): **R29 closed, no new findings — M9 is review-clean and
  complete.** The rustdoc matches the code claim by claim, the abandoned design survives nowhere in `src/`,
  `tests/` or the design docs, and the closure commit is verified **docs-only** at the byte level, so round
  8's stress result stands. Green bar reproduced (116 lib + every suite, zero warnings, `fmt --check` and
  clippy clean); 20/20 clean lib runs. R29's rule promoted to TESTING.md — *re-read surviving prose against
  the new code, not the old defect* — the shape shared by R10, R17 and R19.
  Full record: [reviews/M9.md](reviews/M9.md).

| Finding | Title | Severity | Done |
|---|---|---|---|
| [M9-R1](reviews/M9.md#m9-r1) | The server logs the **requested** provider as if it were the **effective** one — `build_session_reporting` was wired to the benchmark, not to production | observability | [x] |
| [M9-R2](reviews/M9.md#m9-r2) | A session pool can end up **heterogeneous** (some sessions on GPU, some on CPU), making the backend a per-request variable | correctness | [x] |
| [M9-R3](reviews/M9.md#m9-r3) | `--bench-providers` ignores `NER_INTRA_THREADS` / `NER_POOL_SIZE`, so its CPU baseline isn't the CPU the server runs — M7-R1's class, in a new place | correctness | [x] |
| [M9-R4](reviews/M9.md#m9-r4) | An unrecognized CLI argument (`--bench-provider`, `--help`) silently **starts a live proxy** instead of failing | hardening | [x] |
| [M9-R5](reviews/M9.md#m9-r5) | `engaged()` / `ok` reports **registration**, not execution: ORT's per-node CPU fallback is counted as an accelerator measurement | docs | [x] |
| [M9-R6](reviews/M9.md#m9-r6) | ARCHITECTURE contradicts itself on the fallback's safety in adjacent paragraphs ("bit-identical on any backend" vs "subtly different logits") | docs | [x] |
| [M9-R7](reviews/M9.md#m9-r7) | The provider table marks DirectML "✅ tested" like CPU, while the prose says no EP is trusted until the determinism guard is re-run | docs | [x] |
| [M9-R8](reviews/M9.md#m9-r8) | `--bench-providers` cannot benchmark a BERT-family model — it never sends `token_type_ids` | correctness | [x] |
| [M9-R9](reviews/M9.md#m9-r9) | The README sample output is fabricated, and demonstrates the exact coupled-axes error the section warns against | docs | [x] |
| [M9-R10](reviews/M9.md#m9-r10) | Source doc drift: `onnx.rs` still says GPU "comes later (M4)"; `load_onnx_ner`'s doc block was orphaned onto `resolve_ner_model` | docs | [x] |
| [M9-R11](reviews/M9.md#m9-r11) | The CPU fallback is **initialization-only**; a mid-life EP failure degrades silently and never re-arms, while SETUP says "always safe to try" | docs | [x] |
| [M9-R12](reviews/M9.md#m9-r12) | The per-target accelerator now covers three platforms; ARCHITECTURE / DEVLOG / ROADMAP / both READMEs still say "Windows x64 only", and the design doc contradicts itself twelve lines apart | docs | [x] |
| [M9-R13](reviews/M9.md#m9-r13) | The three per-target wirings aren't equivalent: only Linux/CUDA changes the downloaded ONNX Runtime, and it doesn't apply to `aarch64-unknown-linux-gnu` — a shipped release target | correctness | [x] |
| [M9-R14](reviews/M9.md#m9-r14) | `--bench-providers` still advises rebuilding with an `ep-*` feature the per-target dependency already enables | diagnostics | [x] |
| [M9-R15](reviews/M9.md#m9-r15) | The table offers `ep-rocm` / `ep-openvino` as escape hatches, but `download-binaries` has no distribution for either — and on Linux `ep-rocm` now loses CUDA too | docs | [x] |
| [M9-R16](reviews/M9.md#m9-r16) | The non-`onnx` build still advises `cargo build --features ep-directml` — M9-R14's defect in the sibling branch, and on Linux/macOS the advice names a Windows-only backend | diagnostics | [x] |
| [M9-R17](reviews/M9.md#m9-r17) | `no_accelerator_guidance()` inherited the deleted function's rustdoc — it promises a tuple it no longer returns and the cargo feature it was written to stop naming | docs | [x] |
| [M9-R18](reviews/M9.md#m9-r18) | The pre-per-target "seven opt-in `ep-*` accelerators" framing survives in four places — `ExecutionProvider`'s rustdoc, `Cargo.toml`, ARCHITECTURE's opening paragraph, ROADMAP — contradicting M9-R12 and M9-R15 | docs | [x] |
| [M9-R19](reviews/M9.md#m9-r19) | ARCHITECTURE and ROADMAP name `onnx::build_session` as the "one home" of the fallback policy; M9-R1's own closure deleted that function | docs | [x] |
| [M9-R20](reviews/M9.md#m9-r20) | CLI-03 is **vacuous in the `onnx` build** — `--bench-providers` bails at the unconfigured-model check, so the guard never reaches the branch M9-R14 fixed | test-coverage | [x] |
| [M9-R21](reviews/M9.md#m9-r21) | The pre-per-target "`NER_EXECUTION_PROVIDER` + an `ep-*` feature" claim survives in ARCHITECTURE's design-principles bullet and its `--bench-providers` section, plus two DEVLOG sites — the design doc now contradicts CLI-03 | docs | [x] |
| [M9-R22](reviews/M9.md#m9-r22) | "`ep-webgpu` does not link on Windows" is generalized from a seven-feature build whose combined key resolved to `none`; `dist.txt` has an `x86_64-pc-windows-msvc+wgpu` row | docs | [x] |
| [M9-R23](reviews/M9.md#m9-r23) | M9-R20's own closure commit added a new ARCHITECTURE sentence crediting `CLI-03` with pinning the guidance "in either build" — the coverage M9-R20 disproved — and TESTING's CLI-03 entry was never narrowed | docs | [x] |
| [M9-R24](reviews/M9.md#m9-r24) | M9-R22's retraction reached ARCHITECTURE only: four of the six sites it named by `file:line` still say "`ep-webgpu` does not link", and `Cargo.toml` now files `webgpu` in the from-source ❌ tier that ARCHITECTURE moved it out of | docs | [x] |
| [M9-R25](reviews/M9.md#m9-r25) | `no_accelerator_guidance()` is a five-arm `cfg!` chain; BENCH-01 asserts only the arm compiled for the running platform, leaving three shipped arms guarded on no machine | test-coverage | [x] |
| [M9-R26](reviews/M9.md#m9-r26) | The site M9-R24 numbered "Sixth" is the one the sweep missed: `.cargo/config.toml` still files every non-default `ep-*` as "NOT obtainable", contradicting the `Cargo.toml` block rewritten beside it | docs | [x] |
| [M9-R27](reviews/M9.md#m9-r27) | BENCH-02's distinctness assertion catches a **collapsed** chain but not a **permuted** one, so the "Mac user told about DirectML" failure it names as its own reason passes it; the `cfg!`→`Platform` selection is now the untested half | test-coverage | [x] |
| [M9-R28](reviews/M9.md#m9-r28) | **Pre-existing, not M9:** `capture_debug_logs` races under the multi-threaded runner — the buffer can come back empty, failing FC-08's canary test (11% under load) and passing its absence-only sibling vacuously | test-integrity | [x] |
| [M9-R29](reviews/M9.md#m9-r29) | `capture_debug_logs`'s rustdoc documents the `Mutex` fix that was measured **not** to work — it calls a nonexistent lock "load-bearing", says the subscriber is scoped when it is process-global, and contradicts the body comment 35 lines below | docs | [x] |

<a id="m91"></a>
## M9.1 — one release binary per backend

**Opened 2026-07-20**, straight out of M9's own measurement. M9 established that **one ONNX Runtime
distribution carries one set of execution providers** — so no single build can contain them all, and
the choice has to move to *which artifact the operator downloads*. M9 shipped the selector and the
benchmark; this makes the release match what they imply.

**The partition THE RULE produces, and the mistake it corrects.** `ort-sys` keys the downloaded
prebuilt on `training` / `webgpu` / `cuda|tensorrt` / `nvrtx` / `rocm` — and nothing else. So:
DirectML and CoreML are **free** (not keys; already inside their platform's plain tarball) and stay
wired per-target, in everyone's standard binary at zero cost. CUDA and WebGPU **are** keys — M9 had
wired CUDA per-target for `x86_64` Linux, which handed a heavier CUDA runtime to **every** Linux x64
user, NVIDIA device or not. Key-ed accelerators now get **their own artifact** instead.

- [x] `release-build.yml`: a `(target, variant)` matrix — **10 legs**, every one a pair `dist.txt`
  actually has a prebuilt for. Standard (5, all targets) + `cuda` (2) + `webgpu` (3). arm64 Linux and
  arm64 Windows appear once because no second backend exists for them.
- [x] Standard artifacts keep their historical unsuffixed name; variants get `-cuda` / `-webgpu`, and
  the artifact *name* carries the variant too, or two legs sharing a target would overwrite each
  other's upload.
- [x] `release-build-publish.yml` flattens with a **collision check** — the suffixing that keeps
  basenames distinct lives in the *build* workflow, so a mistake there would land here as a silent
  overwrite that publishes fewer binaries than were built. It now fails loudly instead.
- [x] CUDA removed from the per-target block; only the free accelerators remain there.
- [x] README (+ `.it`) carries the operator-facing table: *your machine → which binary → what you get*.
- [x] **Verified on a real CI run** — `manual-build` on `f410c7b`, **10/10 legs green**
  ([run 29741374043](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/runs/29741374043)),
  including all five that had never been compiled anywhere: `cuda` on Windows x64 and Linux x64, and
  `webgpu` on Windows x64, Linux x64 and macOS arm64. The matrix was derived from `dist.txt` rather
  than guessed, and every derived pair built.
- [x] **The docs can no longer go silent about a release.** `v1.2.0` was once cut with the Status
  table saying nothing about it — and the fix for that was itself a trap: writing a tag before it
  exists is a false claim, leaving it out until it exists means the docs always lag. Resolved by
  making the *intent* expressible — `tag `vX.Y.Z` (planned)` — and by **enforcing the rest**:
  `release-build-publish.yml` refuses to publish a `v*` tag this table does not name as cut, so a
  release the documentation cannot explain simply cannot happen.
- [x] **A release is also checked against its own manifest.** The guard above reads only the tag
  *name*, so it cannot see a `v1.3.0` tag cut while `Cargo.toml` still says `1.2.0` — that would
  publish ten binaries reporting a version nobody released. A second step pins the two together, so
  the pair now covers both axes: *can the docs explain this release*, and *is it what it claims to
  be*. (Checkout happens at the tag's ref, so the manifest read is the tagged one — re-running an
  older release reads that release's version, not `main`'s.)

> **Tests deliberately run on the standard legs only.** Their purpose in this workflow is per-**arch**
> coverage (`ci.yml` only tests x86_64 Linux), and the five standard rows already cover every
> architecture shipped. A variant differs solely in which ONNX Runtime tarball is linked — backend
> selection is a *runtime* choice defaulting to `cpu`, so it adds no code path the suite exercises.
> What can break on a variant (a download resolving to nothing, a link failure) is caught by the
> build step, which every leg runs.

<a id="m10"></a>
## M10 — national phone coverage + release hygiene ✅

**Opened 2026-07-29, from a documentation pass that turned into a measurement.** Writing down what
`PII_LOCALES` does surfaced that it does less than everyone assumed. A throwaway probe drove
`StructuredRecognizers::with_locales` over real domestic numbers from nine countries; the matrix is
promoted into [`ARCHITECTURE.md`](ARCHITECTURE.md) → *Locale coverage*. What it showed:

1. **The shipped default `PII_LOCALES=it,us` masks no Italian domestic phone number.**
   `fp_prone_recognizers` matches only `gb` / `de`; `it` and `us` return an empty vec. So
   `06 69821234`, `011 5627111`, `347 1234567` all reach the provider in clear — while `+39 347
   1234567` is masked by the universal `+CC` arm, and `320 123 4567` only because its 3-3-4 grouping
   happens to match the universal *US* arm.
2. **`de` masks several IT landlines by accident** (Rome, Milan, Naples, Florence — but not Turin),
   because plans overlap per *number*, not per country. Accidental coverage is not coverage: a
   libphonenumber metadata update can move it without a line of our code changing.
3. **Nothing regressed — the gap was never filled.** [M4](#m4) introduced `PII_LOCALES` when the
   FP-prone tier was *empty*, choosing `it,us` as a placeholder naming the project's own locales;
   [M8.1](#m81) then added the tier's first two entries (`gb`, `de`) without revisiting that
   default. So the mismatch is a leftover of "the tier had nothing in it", not of lost coverage.

**Scope is every missing country, not IT alone.** GB and DE were where M8.1's evidence was, and
they are legitimately covered; the rest of Europe simply never got its turn.

**The library is not the constraint.** `phonenumber` 0.3.10 carries **245 regions** — every country
worth adding is already in `country::Id` (spot-checked: IT, FR, ES, PT, NL, BE, AT, CH, IE, PL, SE,
DK, NO, FI, GR, CZ, RO all present). Adding a region is an entry in our own enabled-region set, not
new metadata. **The two real constraints are ours:**

- **The candidate regex is `0`-trunk only** (`src/pii/recognizers.rs:346` — all three alternatives
  start with `0`). Countries that dropped the trunk prefix propose **no candidate at all**, so a
  match arm alone is a no-op for them: ES `91 123 45 67`, IT **mobiles** `347 1234567`. Generalizing
  the anchor is the actual work — and it widens the candidate set sharply, leaving `is_valid()` as
  the only filter. That is where the FP risk moves.
- **Cost scales with the number of enabled regions.** A candidate is validated per region until one
  accepts, so N regions mean up to N `parse().is_valid()` calls per candidate, on the deterministic
  path that is today the *fast* one (~20 ms/turn structured-only). "Turn on all 245" is not a design;
  a vetted set is.

**And the question that prompted this: does the gate still earn its keep?** It exists because
[M4-R1](reviews/M4.md#m4-r1) called an un-anchored national phone FP-prone — an objection
[M8.1](#m81) then **defused by measurement** (`is_valid()` against the real numbering plan: GB
precision 1.000 / FP-rate 0.000; DE 0.909, whose single "FP" `0049301234` is itself a real
international dial to Berlin). If the reason for opt-in is gone, an opt-in default is just a way to
ship less protection than the code can give. But precision was measured **per region**: enabling N
regions unions their accepted sets, so the union's FP-rate is ≥ the worst single one and grows with
N. That number decides the default, and we do not have it yet.

- [x] Generalize the candidate regex beyond the `0` trunk (ES, IT mobiles) **without** unbounding
      match length — `Scan::Overlapping` linearity (M4-R19) and the M8-R8 shadowing case must hold.
      Two families added (the French five-pair form; an un-anchored one), longest match ~18 chars,
      `tests/complexity.rs` green
- [x] Add the missing regions to the enabled set — **the dispatch shape is settled in step 4 and it
      came out (b): one shared regex per shape family, not one recognizer per region** — each with
      adversarial corpus cases, starting with **IT** (landlines across area-code lengths `02` / `06`
      / `011` / `055` / `081`, plus mobiles) since it is a declared locale of this project
- [x] Measure the **compound** FP-rate — each region alone, then the union — on an extended
      adversarial corpus of phone-shaped non-phones (order numbers, invoice refs, national IDs,
      dates), and the added latency per enabled region. `tests/phone_eval.rs`; numbers in
      [ARCHITECTURE → *Domestic phone coverage*](ARCHITECTURE.md) and DEVLOG 2026-07-29
- [x] **Tests that make "all countries are covered" checkable, not claimed.** Per enabled region:
      positive cases across the shapes a plan actually has — landline (every area-code length that
      country uses), mobile, toll-free/service — plus negatives from that country's own look-alikes
      (order numbers, VAT/tax IDs, dates, postcodes). Then the guard that keeps it honest: a test
      that **enumerates the regions the code enables and fails if any lacks corpus coverage**, so a
      region can never be switched on without its cases. A country added silently is the failure
      mode here, and it is the one a checklist alone does not catch
- [x] **Try the all-on default first, and only fall back if the numbers refuse it.** In order of
      preference: (a) every vetted region on by default and `PII_LOCALES` **removed**; (b) all-on by
      default with `PII_LOCALES` kept as an **override of that list** — set it and it replaces the
      default set, leave it unset and you get everything; (c) only the regions whose measured
      FP-rate is acceptable are on by default, with the excluded ones documented as opt-in and the
      number that excluded them written down. (a) is cleanest but drops a documented variable, which
      a patch must not do — so **(b) is the expected landing**: same protection out of the box, and
      an operator who set `PII_LOCALES` keeps exactly the behavior they asked for.
      **Landed (b).** Recall **1.000** for every region and for the union, and **zero `Phone`
      spans on a real 22 KiB agent turn** — those are the evidence. (c) was reconsidered against
      the *corrected* numbers in review round 1 and still not taken: demoting ES/PT/CN would fix
      the headline for `it` while re-creating the same defect for three other declared countries.
      The full per-category matrix lives in [ARCHITECTURE](ARCHITECTURE.md), re-measured after
      [M10-R3](reviews/M10.md#m10-r3) showed the first pool could not reach half the shapes —
      **do not quote FP figures from this file**, which is exactly how the superseded ones
      survived here for two rounds ([M10-R22](reviews/M10.md#m10-r22)). "Union-only false
      positives 0" is likewise **not** evidence: it is a structural identity of the dispatch
      ([M10-R6](reviews/M10.md#m10-r6))
- [x] **The floor, binding on every branch above: a freshly started proxy with no configuration
      MUST have national-phone recognizers active.** "Everything off unless you knew to switch it
      on" is the defect this milestone exists to end, not a fallback it may land back on — so the
      branches differ only in *how many* regions are on and whether the variable survives, never in
      *whether* the default detects anything. Pin it with a test that builds the detector from an
      **empty environment** and asserts a domestic number is masked; that test is what stops the
      regression from ever returning silently. **PHONE-NAT-06** does exactly that, for all nine
      regions at once. Promoted to [ARCHITECTURE](ARCHITECTURE.md) → *Decisions*: a default that
      detects nothing is a bug, not a conservative choice
- [x] **The over-mask guard, on text nobody curated.** The negative corpus only contains
      digit-shaped non-phones *we thought of*. This milestone widens the candidate set twice over —
      a non-trunk shape family means any plausible digit group is a candidate, and ~9 plans give
      each candidate nine chances to be somebody's valid number. Real agent traffic is full of
      digit runs no one writes test cases for: line numbers in diffs, ports, byte offsets,
      timestamps, PIDs, error codes, file sizes. So assert over the **M7 latency fixture** (a real
      22 KiB Claude Code turn already in the repo, not a synthetic string): with every region on,
      the `Phone` spans it yields must be exactly the ones we expect — a change in that count is a
      regression to explain, not a diff to accept. Cheap, automated, and it catches the gross case
      before anyone spends a live session on it. **PHONE-OM; the expected set is empty**, with a
      positive control so "found nothing" can never be "detector is dead"
- [x] `TESTING.md` catalogs the new cases; `ARCHITECTURE.md`'s matrix is **re-measured**, not
      re-asserted

### Also in this milestone — four rough edges of the shipped binary

All four surfaced on 2026-07-29, three while documenting how to run the released binary and the
fourth by running it. None is about detection, and each is the kind of defect that only shows up
when you use the shipped artifact rather than `cargo run` with the repo open next to you: **a
downloaded executable should be able to say what it is, what it accepts, and when things happened.**

- [x] **`--version`.** There is none, and because `main.rs` refuses unknown arguments (M9-R4),
      `llm-proxy-pii-rust --version` does not print a version — it **fails to start**. A release
      asset is a bare executable, so once it is saved next to an older copy nothing identifies it.
      The alternative considered and **rejected** was putting the version in the artifact filename:
      it was implemented, then rolled back, because a filename is a convention a rename can break
      while a binary reporting `CARGO_PKG_VERSION` cannot lie — and the filename approach also
      forfeits GitHub's stable `releases/latest/download/<name>` redirect. Print the version, the
      target triple, and whether the ML layer is compiled in (`--features onnx`), since "which
      build is this?" is the same question and `--bench-providers` already knows the answer.
- [x] **Default log level: `error` → `info`.** `EnvFilter::from_default_env()` falls back to
      **ERROR-only** when `RUST_LOG` is unset, so the shipped default prints **nothing at all** —
      no `listening on`, no `ONNX NER detector loaded`. Measured, not inferred: the binary was run
      both ways. That breaks the one check this project tells operators to make ("if the NER line
      is missing you are running structured-only"), because with no `RUST_LOG` *every* line is
      missing, and a silent process looks equally like a healthy one and a broken one. Fix is
      `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))`; `RUST_LOG`
      keeps overriding it. **This does not weaken the privacy bar** — logs carry kinds, counts and
      placeholders only, `Config`'s manual `Debug` redacts `upstream_api_key` (`src/config.rs:251`),
      and `tests/log_safety.rs` enforces it at every level; the masked-body dump stays `trace`-only.
      *Shipped as written, plus one thing the plan missed: an exported-but-**blank** `RUST_LOG`
      parses to no directives, which is ERROR-only — the same silent binary through a second door.
      It now counts as unset (LOG-02).*
- [x] **Timestamps in local time, with the offset shown.** Logs are UTC today
      (`2026-07-29T09:15:10.844780Z` — `tracing_subscriber::fmt`'s default), so an operator in
      CEST reads `10:37` for something that happened at `12:37` and has to do the arithmetic while
      debugging. UTC is the right default for a fleet of servers correlating logs; this is a
      *local* proxy running next to the person reading it. **The timestamp keeps every part it has
      today** — date, time, sub-second precision — and only shifts to local, with the zone marker
      changing from `Z` to a numeric offset:
      ```text
      before  2026-07-29T10:37:04.844780Z         INFO … listening on http://127.0.0.1:8080
      after   2026-07-29T12:37:04.844780+02:00    INFO … listening on http://127.0.0.1:8080
      ```
      **The offset is an addition, never a replacement**: what must not ship is a local time with
      no zone at all (`12:37:04`), which reads correctly on the author's machine and is ambiguous
      everywhere else. Looks trivial and is not — see step 1. *Shipped; the UTC fallback prints
      `+00:00` rather than `Z`, which keeps one format and one code path while still giving the
      reader an explicit offset.*
- [x] **`--help` must document the configuration, not just the two flags.** It exists (`main.rs`,
      and the unknown-argument refusal prints it too), but it lists only `--bench-providers` /
      `--help` and defers configuration to `README.md` / `docs/SETUP.md`. For a tool whose
      configuration is **entirely** environment variables and has no config file, that means the
      shipped binary cannot tell you what it accepts — you need the repo to run the program. List
      every variable with its default and a one-line purpose, grouped as the README table is
      (server · upstream · detection · NER · GLiNER · debug).
      **The risk is drift**, and a hand-written help text that silently falls behind `Config` is
      worse than a short one — so pin it the same way the region coverage is pinned: a test that
      **scans the source for every `env::var` / `env_flag` / `env_or` key and fails if one is
      missing from the help text**. Then a variable cannot be added without appearing in `--help`.

> **Why a patch (`v1.2.1`) and not a minor.** It closes a gap between what the tool claims and what
> it does — the declared `it` locale masking nothing — rather than adding a capability. The one thing
> that would make it a minor is option (a), removing `PII_LOCALES` outright; that is why (b) keeps
> the variable as an override rather than deleting it.

> **The `PII_LOCALES=gb,de` workaround is retired.** It was what the maintainer ran locally: GB and
> DE properly, plus whichever Italian landlines the German plan happened to accept, and nothing for
> Italian mobiles or Turin. Unset now means all nine regions; setting it still works and now
> *narrows* rather than being the only way to get anything.

> **The deliverable is the measurement.** If enabling every vetted region is safe, the tool should
> ship it on; if it isn't, the docs should say what it costs. Widening coverage without those numbers
> would trade a documented gap for an undocumented over-mask.
>
> **It was safe enough to ship on, and the docs say what it costs.** Recall 1.000 per region and
> for the union; zero `Phone` spans on a real 22 KiB agent turn; latency flat (0.30 → 0.32 ms/turn
> for 0 → 9 regions). The residual — dates and space-separated numeric tables, per region and per
> category — lives in [ARCHITECTURE → *Domestic phone coverage*](ARCHITECTURE.md) and **nowhere
> else**. That is deliberate: the figures once appeared here too, and when
> [M10-R3](reviews/M10.md#m10-r3) corrected them upward this copy stayed stale through two review
> rounds ([M10-R22](reviews/M10.md#m10-r22)). One home per number, like one home per finding.

### Implementation order

Written 2026-07-29 for the builder. **Steps 1–3 are independent of the phone work and should land
first** — they are small, they unblock the `RUST_LOG=info` workaround the READMEs currently
prescribe, and keeping them in their own commit keeps the risky detection change reviewable alone.

**1 · The subscriber: default level *and* local timestamps** (`src/main.rs`). Both changes edit the
same six lines, so they land together, in one commit, with one test file.

*Level.* `EnvFilter::from_default_env()` → `EnvFilter::try_from_default_env().unwrap_or_else(|_|
EnvFilter::new("info"))`. Test: the fallback directive is `info` when `RUST_LOG` is absent, and a set
`RUST_LOG` still wins — drive the filter builder directly, do **not** mutate process env in a
parallel test run. Then delete the `RUST_LOG=info` line and its warning box from both READMEs.

*Timestamps — and this is the part that is not trivial.* The obvious fix,
`tracing_subscriber::fmt::time::LocalTime`, **fails at runtime in this program**. It calls into the
`time` crate's local-offset lookup, which **refuses to answer once the process is multi-threaded**
(its guard against the `localtime_r`/`setenv` data race, CVE-2020-26235) — and `#[tokio::main]`
builds the runtime, worker threads and all, *before* the body of `main` runs, so by the time the
subscriber is initialized the answer is already unavailable. The failure is platform-split and
therefore a trap: Windows has a thread-safe path and usually works, Linux and macOS do not. Shipping
"it looked right on my box" is exactly the shape of bug this project keeps finding.

  So: **capture the offset while the process is still single-threaded**, then hand it to
  `OffsetTime::new(offset, …)`. That means dropping `#[tokio::main]` in favour of a plain `fn main()`
  that reads the offset first and then builds the runtime itself — a small, contained change, but a
  *structural* one, which is why this is not the one-liner it looks like.

  **If the offset cannot be determined, fall back to UTC and say so once at startup.** A silently
  wrong local time is worse than an honest `Z`: it is the kind of thing that costs an hour during an
  incident. Never guess an offset.

  Enabling this needs `tracing-subscriber`'s `local-time` feature and `time/local-offset`. `time`
  0.3.53 is already in `Cargo.lock` and is pure Rust (nothing on `dependency_footprint.rs`'s
  forbidden list), but confirm with `cargo tree` that it is in the **default** build's tree before
  assuming the cost is zero — that guard exists because this kind of "surely it's already there"
  has been wrong before.

  *Test what is deterministic.* Do not assert a wall-clock value — the result depends on the machine.
  Assert (i) the offset helper returns the same answer when called before threads exist, (ii) the
  fallback path yields UTC plus its warning, and (iii) in `binary_smoke.rs`, that an emitted line's
  timestamp carries an **explicit** offset (`Z` or `[+-]HH:MM`). (iii) is the one that catches the
  real regression — a bare local time with no offset, which reads fine on the author's box and is
  ambiguous everywhere else.

**2 · `--version`** (`src/main.rs`). Print `CARGO_PKG_VERSION`, the target triple, and whether the
`onnx` feature is compiled in. It must be handled in the same `match` as `--bench-providers`, before
`Config::from_env()`, so it works without a valid upstream config. Test in `tests/binary_smoke.rs`
alongside CLI-01/CLI-02, driving the **real binary**: the flag prints the manifest version and exits
0. Catalog it as CLI-04.

**3 · `--help` covers the configuration.** Extend `USAGE` with every environment variable, grouped
as the README table is. **The guard is the point** — a hand-written help text drifts. Add a test
that `include_str!`s `src/config.rs` and `src/server.rs`, regex-extracts every `env::var("X")` /
`env_flag("X")` / `env_or("X", …)` key, and asserts each appears in `USAGE`. That test is what makes
"documented" true a year from now, and it fails loudly the first time someone adds a variable.
CLI-02 already asserts `--help` exits 0 — extend rather than duplicate it, and catalog the new
coverage guard as CLI-05. Note the deliberate scope limit: it proves every key is **named**, not that
its description is accurate; say so in `TESTING.md` rather than letting the guard read stronger than
it is.

**4 · The validator shape — decided, do not re-open.** `Recognizer.validate` is
`Option<fn(&str) -> bool>` (`recognizers.rs:55`), a bare fn pointer that **cannot carry a region** —
which is why GB and DE each needed a hand-written wrapper today. Two ways out, and they trade scan
cost against validation cost:
  - **(a) One recognizer per region.** Zero type change: `fp_prone_recognizers` already returns a
    `Vec<Recognizer>`, so each region contributes its own regex + wrapper. Cost: N regexes scanned
    over every field, and `Scan::Overlapping` is O(n·L) *per recognizer*.
  - **(b) One recognizer per *shape family*, validator tries the enabled regions.** Scan count stays
    constant as N grows and validation only runs on candidates (rare). Cost: `validate` must become
    a closure — `Box<dyn Fn(&str) -> bool + Send + Sync>` — since detectors are held as
    `Box<dyn PiiDetector>` and run under `spawn_blocking`. Check what that breaks (any `Clone` or
    `Copy` derive on `Recognizer`) before committing to it.

  **Decided: (b), and it was measured rather than argued** (throwaway probe, 2026-07-29, deleted
  after reading; **release** profile, 22 KiB payload — the M7 realistic-turn size — 252 candidates,
  9 regions). **Measurement conditions, stated because this project has been burned by omitting
  them:** the reference box under load — a Teams call running, and mismatched memory banks (16 GB +
  8 GB, so not a clean dual-channel pair). **Treat the absolute milliseconds as this box's on a bad
  day, not as the product's.**

  | shape | per pass | what it isolates |
  |---|---|---|
  | (a) one recognizer per region | **12.28 ms** | 9 scans + 11,880 validations |
  | (b′) shared regex, validate every region | **6.02 ms** | 1 scan + the *same* 11,880 validations |
  | (b) shared regex, short-circuit on first accept | **2.40 ms** | 1 scan + 4,220 validations |

  **(a) → (b′) is 2.04× and is purely the scan count** — the two do byte-identical validation work,
  so the whole gap is 8 extra passes over the text, ~0.78 ms each per 22 KiB. That is the term that
  **grows linearly with every region added**: nine regions would put ~7 ms per field onto a
  structured-only path that costs ~20 ms for a whole turn. `.any()`'s early exit is worth a further
  2.51×, for **5.12× overall**. Note it came out *larger* in release than in debug (2.91×), i.e. the
  opposite of the "validation dominates once optimized" guess — which is the reason to measure.

  **The decision does not rest on the timings, which is why the noisy box does not threaten it.**
  (a) and (b′) run the *same* validations; (a) additionally scans the text N−1 more times. It is
  **strictly more work**, so (b′) cannot lose except to measurement noise — arithmetic, not a
  benchmark result. What the numbers add is only *how much*, and a quieter machine would move the
  size of the win, never its direction. Re-measure if a precise cost per region is ever needed
  (idle box, on AC); do not re-open the choice over it.

  So `validate` becomes a boxed closure. **Region granularity is not what is being traded away**:
  (b) still enables regions individually and still bounds the set by the step-5 principle — only the
  *dispatch* changes, so the "one region, one decision" property survives intact.

  And the M4-R19 bounded-length rule is **not** solved by either shape — it constrains the regex, so
  it applies identically to both. What (b) changes is that there is **one** pattern family to keep
  bounded instead of N, i.e. one place to get it wrong rather than nine. Whichever way, **GB and DE
  must keep the exact behaviour M8.1 measured**; their corpus cases are the regression guard.

**5 · Bound the region set by a principle, not by taste.** Cover **exactly the countries the tool
already claims** — the 10 national-ID countries and the XLM-R language set: IT, ES, FR, NL, PT, LV,
CN, on top of today's GB + DE. (US needs nothing: it has no trunk-`0` domestic form, and the
universal arm already covers `NNN NNN NNNN`.) That is defensible, finite, and leaves "why not
Belgium?" answered by the same rule that answers it everywhere else in the detector.

> **Considered and rejected: default to the *host's* locale** instead of an explicit set. It
> contradicts the rule this detector already runs on — [M4-R1](reviews/M4.md#m4-r1) made national IDs
> always-on *regardless of configuration* precisely because **what matters is the data that arrives,
> not where the proxy runs**. A proxy in a Frankfurt datacenter routinely carries Italian users'
> data, and an Italian developer on a US-locale Windows install would silently lose IT coverage —
> the exact failure this milestone exists to end. It also makes masking **machine-dependent**: the
> same request, two boxes, two results, with nothing in the logs to explain the difference. For a
> fail-closed tool that is a worse property than needing one explicit variable. The measurement in
> step 4 removes the motive anyway: under (b) an extra region is cheap, so "guess what this host
> needs" buys nothing that "cover what we claim to cover" doesn't already give.
>
> **What the host locale was really trying to fix is real** — that today's default detects *no*
> domestic number at all. The fix is turning the vetted set **on**, which is strictly more coverage
> than any single guessed locale: a host-locale default would still miss the Italian number arriving
> at a German-locale server. Reading the machine answers a question that stops being asked once
> nothing is off by default.

**6 · The candidate regex, per shape family.** Today's three arms all require a leading `0`
(`recognizers.rs:346`). Add a **non-trunk** family for IT mobiles (`347 1234567`) and ES
(`91 123 45 67`). **The M4-R19 constraint is absolute: match length must stay bounded** (~15 chars),
or `Scan::Overlapping` stops being linear and the complexity DoS is back. Do not write an open
`(?:[ -]?\d)+`. Re-run `tests/complexity.rs` — its wall-clock budgets are the guard — and keep the
M8-R8 shadowing case green (the recognizer must **never** override `redetect`; the `mask_all`
fixpoint is what un-shadows an adjacent number).

**7 · Corpus and the coverage guard.** `tests/corpus/pii_cases.json` already carries a per-case
`locale` field (`recognizers.phone`, today 5 positive / 1 negative — thin). Add, per region:
positives across every area-code length that country really uses, plus mobile and toll-free/service;
negatives from that country's own look-alikes (order numbers, VAT/tax IDs, dates, postcodes). Then
the guard from the scope list above: **a test that enumerates the regions the code enables and fails
if any has no corpus case**, so a region cannot be switched on silently. New IDs continue the
`PHONE-NN` scheme; catalog them in `TESTING.md`.

**8 · Measure, then choose the default.** An `#[ignore]`d harness in the shape of `ner_eval` that
prints precision / recall / FP-rate **per region and for the union**, plus the added latency per
enabled region. Only then take the (a)/(b)/(c) default decision from the scope list, and write the
numbers into `ARCHITECTURE.md`'s matrix — **re-measured, not re-asserted** — and into `DEVLOG.md`.

**9 · Close the loop.** `PII_LOCALES` stays accepted whatever is decided (a patch must not break an
operator who set it); the README's `PII_LOCALES` rows and the `it,us`-is-inert warnings come out
only once the code makes them false.

**10 · Which [CC scenarios](TESTING.md#cc-battery) this milestone actually needs — and why not all
nine.** The battery is expensive (a real Claude Code session, two runs each, human eyes on two
traces), so spend it where automated tests are structurally blind.

  > **It does *not* need a key configured on the proxy, and getting that wrong has now cost this
  > project real decisions — including one in this milestone.** The proxy holds **no credential**:
  > it forwards the client's own, which is why [M6](#m6)'s live run got a 200 on the first try with
  > nothing configured. What the battery needs is a **human with a working Claude Code**, pointed at
  > the proxy — the runbook is [`MANUAL_VERIFICATION.md`](MANUAL_VERIFICATION.md), which has said so
  > correctly all along. The error is a *summary* drifting from its source: "needs a live provider"
  > (true, and inherited from [M5](#m5), when the proxy was OpenAI-compat-only and Claude Code
  > **could not** route through it at all) got compressed into "needs a live `ANTHROPIC_API_KEY`"
  > (false since M6 shipped the native route). **When this file, `TESTING.md` and
  > `MANUAL_VERIFICATION.md` disagree about how to run something, the runbook wins** — it is the one
  > that was written while doing it. **They are blind to exactly one thing here:
over-masking real agent traffic, and the harm is functional rather than privacy.** A masked line
number or port inside `tool_use.input` hands the model `[PHONE_1]` where it needed `8080` — the
agent then does the wrong thing, and no corpus test will show you that because a corpus contains
only the false positives we imagined. Run:
  - **CC-04** (`tool_use.input` carries a value back) — the direct "over-mask corrupts a tool
    argument" case, and the highest-value single run.
  - **CC-09** (PII through an MCP SQL tool) — SQL result sets are the densest numeric payload the
    battery has: row IDs, counts, amounts.
  - **CC-03** (a file with PII → `tool_result`) — file contents are where line numbers and offsets
    arrive.
  - **CC-01** as the baseline that ordinary structured masking still round-trips.

  **Skip CC-02, CC-05, CC-06, CC-07, CC-08.** M10 touches the deterministic layer only: the NER,
  determinism, secrets, thinking blocks and streaming are all untouched, and re-running them would
  buy confidence in code this milestone never edits. Say so in the DEVLOG when reporting the run —
  *"five scenarios deliberately not run, because M10 cannot reach them"* is a result; silence about
  them reads as a full battery that passed.

  > **Run 2026-07-31 — four scenarios × two postures, all pass.** With the owner driving a real
  > Claude Code session on the hybrid (`NER_REQUIRED=1`, both green startup lines, no credential on
  > the proxy). 47 forwarded requests, **zero** fixpoint 400, **DBG-02 = 0** on all 16 raw values
  > across both logs. The masked bodies are **byte-identical between postures**, and the tokens the
  > client reads on ON (`[PERSON_4]`, `[EMAIL_2]`, `[IBAN_1]`) are the ones the OFF trace shows
  > leaving and restores — one round-trip seen from both ends. **The risk this subset exists for is
  > answered:** `Read`'s line numbers `1`…`5` passed untouched beside a masked phone column, the SQL
  > result's `"id":1` / `"row_count":1` passed untouched beside six masked fields, and a 199 KB real
  > turn yielded zero `Phone` spans — PHONE-OM's offline result with tool *results* included. CC-04
  > is the sharpest: identical `tool_use.input` in both runs, `first-contact.txt` holding
  > `bob@test.com` after OFF and `[EMAIL_2]` after ON. **Five scenarios deliberately not run**
  > (CC-02/05/06/07/08 — M10 never edits that code). Full account: DEVLOG 2026-07-31; the run also
  > corrected a two-milestone-old falsehood about CC-09's MCP setup, now
  > [four steps in the runbook](MANUAL_VERIFICATION.md#cc-09s-setup-which-is-not-one-line).

<a id="what-is-left-before-the-tag"></a>
### The road to the tag, and what it took
*(was "What is left before the tag" until the tag was cut — the old anchor still resolves, because
[reviews/M10.md](reviews/M10.md) refers to it by that name and the archive is history, not a draft.)*

Written 2026-07-29, rewritten 2026-07-30 after round 9's findings were closed, **and closed out
2026-07-31 when the CC battery landed and `v1.2.1` was cut.** The milestone's scope items are all
`[x]`, all **61** review findings across nine rounds are `[x]`, and the suite is green on both
feature sets (**221 default / 254 onnx**, zero warnings; `fmt`, `clippy -D warnings` clean;
`cargo doc` 15 warnings, all pre-existing). Nothing is left in the code, and **the one thing no
review round could produce — a live Claude Code session — was run before the tag, not after.**

> **The milestone is done reviewing.** Rounds 5–9 found **zero live defects in `src/`** — five
> consecutive rounds, with round 9 driving the real `.exe` end to end and mutating the tree to check
> its own guards. What kept producing findings was never the code: it was the loop between
> *measurement* and *prose*, and both halves of that loop now have a mechanical guard rather than a
> promise. `DOS-BUD` measures the rendering axis, three layouts and both column renderings, so the
> published band is read off the harness instead of typed in; **CAT-01** extracts every guard id a
> `#[test]` declares and asserts it is named in `TESTING.md`, which is the check M10-R55 prescribed,
> M10-R59 found unbuilt, and which caught four uncatalogued ids on its first run.
>
> The threshold was re-confirmed at **500,000** on the corrected numbers (see
> [M10-R56](reviews/M10.md#m10-r56)): ~62,500 phone numbers per request at the most expensive column
> rendering, an ordinary 5,000-row export at 8%, the M7 turn at 0.

1. ~~**The CC battery — CC-01 / CC-03 / CC-04 / CC-09**~~ — **done 2026-07-31**, all four in both
   postures, zero leaks and zero fixpoint 400 (step 10 above; DEVLOG for the full account). Nothing
   the tag waits on is left in this line.
2. ~~**Bump `Cargo.toml` to `1.2.1`**~~ — **done 2026-07-31**, together with dropping `(planned)`
   from the Status table, in one `chore(release):` commit. Until that commit the mismatch was
   *protective* rather than a gap: `release-build-publish.yml` refuses to publish a tag that
   disagrees with **either** the manifest or the table, and **both halves were simulated in round 9**
   and behaved as described. The two edits belong in the same commit precisely because either one
   alone leaves the guard armed against a release that is otherwise ready.
3. **A round 10 is optional and is not a gate.** If one is run, the thing to attack is
   [M10-R56](reviews/M10.md#m10-r56)'s band — the **sixth** version of those numbers, and the first
   five were wrong — plus whether `DOS-BUD`'s rendering set and `CAT-01`'s prefix list are themselves
   one-point grids. Run it *after* the tag as release verification rather than before it as a blocker.

<a id="m10-ledger"></a>
### Review ledger — M10 → [`reviews/M10.md`](reviews/M10.md)
**Round 1 (2026-07-29): 12 findings, all 12 closed.** The headline claim holds — verified through the real
binary that an unset `PII_LOCALES` masks all nine regions and that setting it narrows — and every published
precision figure reproduced exactly. The two sharp ones are [M10-R1](reviews/M10.md#m10-r1) (the un-anchored
family voids the M8-R8 fixpoint backstop, and both PROP-03 and PROP-04 are structurally blind to it) and
[M10-R2](reviews/M10.md#m10-r2) (a legal body now costs ~100 s of CPU, on a guard whose four cases produce
no phone candidate at all).

> **Two findings changed the product, not just the code.** [M10-R3](reviews/M10.md#m10-r3) showed the
> milestone's own deliverable measurement was reporting the *pool's* shape rather than the detector's
> precision — so the region table now declares **which renderings each country's numbers actually take**
> (`Trunk` · `TrunkPairs` · `Groups` · `LongBlock`) instead of one coarse trunk/non-trunk flag, and the
> published matrix in [ARCHITECTURE](ARCHITECTURE.md) is re-measured against a pool built from the families'
> own structure. [M10-R1](reviews/M10.md#m10-r1)'s fix — retry a rejected match one group shorter — is what
> the trunk anchor used to give for free. Both promoted to ARCHITECTURE; the numbers moved and are
> **worse**, which is the point of publishing them.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R1](reviews/M10.md#m10-r1) | The un-anchored family voids the M8-R8 fixpoint backstop — a real domestic number is left **partially in clear** at the shipped default | **leak** | [x] |
| [M10-R2](reviews/M10.md#m10-r2) | Per-candidate `phonenumber` validation costs ~100 s of CPU on a legal body; `complexity.rs` holds the character class constant and cannot see it | **BLOCKER** | [x] |
| [M10-R3](reviews/M10.md#m10-r3) | The generated negative pool cannot express the un-anchored families' shapes, so the published per-category zeros are a property of the pool (re-measured: 0.653) | measurement | [x] |
| [M10-R4](reviews/M10.md#m10-r4) | CLI-06 fails wherever `NO_COLOR` is unset — green only by accident of the harness shell, and CI does not set it | build | [x] |
| [M10-R5](reviews/M10.md#m10-r5) | ARCHITECTURE's supply-chain mitigation (`PII_LOCALES=` switches the tier off) turns it fully **on** | docs | [x] |
| [M10-R6](reviews/M10.md#m10-r6) | "union-only false positives: 0" is a structural identity of shape (b), not a measurement | measurement | [x] |
| [M10-R7](reviews/M10.md#m10-r7) | PHONE-OM's positive control only exercises the trunk family — the guard passes unchanged with both new families off | guard | [x] |
| [M10-R8](reviews/M10.md#m10-r8) | CLI-05's hand-listed source set already misses `HF_HUB_CACHE` / `HF_HOME`, documented as honored and absent from `--help` | guard | [x] |
| [M10-R9](reviews/M10.md#m10-r9) | DEP-01 is a six-name denylist, not the native-dep-free guarantee M10 cited it as | docs | [x] |
| [M10-R10](reviews/M10.md#m10-r10) | `Recognizer`'s doc comment absorbed into the new `Validator` alias; `[VETTED_PHONE_REGIONS]` names no item | docs | [x] |
| [M10-R11](reviews/M10.md#m10-r11) | The M9-R4 unknown-argument refusal is order-dependent: `--version --bogus` exits 0 | low | [x] |
| [M10-R12](reviews/M10.md#m10-r12) | The fixture-shape assertions the shared module points at are `#[ignore]`d and `onnx`-only | low | [x] |

**Round 2 (2026-07-29) — closure verification: all twelve hold**, each checked against the
**pre-fix tree** rather than the diff, and every `phone_eval` figure now published reproduced
exactly. Seven new findings, one of them the reason `v1.2.1` could not be cut yet:
**[M10-R13](reviews/M10.md#m10-r13) — M10-R2's fix *relocated* the leak it closed**, the M4
retrospective's signature move, landing on the guard that was supposed to make it impossible.
**All seven now closed.**

> **M10-R13 was closed by *deleting* the optimisation, not by repairing it** — measured with the
> gate forced open, it bought **nothing** (382→384 ms, 258→257, 145→145 on the very inputs it was
> added for), because the per-scan memoization was already doing all the work. A filter that costs
> recall and buys no speed has no defence. The rule it leaves behind is in
> [ARCHITECTURE](ARCHITECTURE.md): *a cheap filter in front of a validator must be derived from
> what the **validator** accepts, not from what the **metadata** describes — and proved
> differentially, never by a list of inputs the author expected it to allow.* Its old guard
> asserted the property over 30 hand-written literals, all ≤ 13 digits, while the defect lived at
> 14–15: **an assertion made only where it cannot fail is not an assertion.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R13](reviews/M10.md#m10-r13) | The digit-count gate rejects numbers `is_valid()` accepts, so M10-R2's fix re-creates M10-R1's truncation | **leak** | [x] |
| [M10-R14](reviews/M10.md#m10-r14) | `docs/SETUP.md` still documents the pre-M10 world, incl. "an Italian domestic number is not masked" | docs | [x] |
| [M10-R15](reviews/M10.md#m10-r15) | `shrink_to_a_valid_prefix`'s doc says it *is* applied to the trunk families; the code says the opposite | docs | [x] |
| [M10-R16](reviews/M10.md#m10-r16) | The same latency measurement is published as two different numbers; ARCHITECTURE's is the one that doesn't reproduce | measurement | [x] |
| [M10-R17](reviews/M10.md#m10-r17) | `every_declared_shape_is_needed_by_a_corpus_case` does not read the corpus | guard | [x] |
| [M10-R18](reviews/M10.md#m10-r18) | M10-R12's closure claims both comments were fixed; the shared module's still points at the `#[ignore]`d guard | low | [x] |
| [M10-R19](reviews/M10.md#m10-r19) | The M10-R10 fix left a new `cargo doc` warning of its own | low | [x] |

**Round 3 (2026-07-29) — closure verification: six of seven hold**, both behavioural ones attacked
directly against a gate-reinstated build in a throwaway worktree. Deleting the gate is defensible on
re-measurement, PHONE-NAT-10 really does catch the M10-R13 defect (0.94 s), and every `phone_eval`
figure reproduced exactly. [M10-R19](reviews/M10.md#m10-r19)'s closure does **not** hold — reopened
as [M10-R23](reviews/M10.md#m10-r23). Seven new findings, **all now closed.**

> **[M10-R20](reviews/M10.md#m10-r20) is closed by bounding the work, not by making it faster.**
> There is no faster validator to reach for (~6.5 µs per region, and the one cheap filter was
> M10-R13's leak), so a field's validator calls are capped and exceeding the cap is an **`Err` on
> the `try_detect` channel** — the request is blocked, never forwarded with a partial scan
> (M5-R7's rule). Measured: a legal **15 MiB body of distinct candidates goes from 64.5 s of CPU
> to 0.99 s and a refusal**; 1 MiB still scans, in 0.81 s; DOS-05's periodic shapes are unchanged.
> DOS-06 is the guard, and its generator is an **odometer rather than a modular hash** — the first
> draft used `(i * 7) % 9000`, silently repeated after its period, and reported the budget as never
> reached.

> **The one all three rounds shared.** Every DoS number this milestone published — M10-R2's table,
> its closure, round 2's verification, and M10-R13's "the gate bought nothing" — was measured on a
> **periodic** body, and so is DOS-05. The per-scan memoization that carries the whole fix is keyed
> on the matched bytes, so **its benefit is a property of the input's periodicity, not of the code**:
> hold the shape and size fixed and vary only the distinct-candidate count, and 4 MiB runs 207 ms →
> 17.0 s. A legal 15 MiB body of *distinct* digit groups answers **200 in 64.5 s** at the shipped
> default. That is [M10-R20](reviews/M10.md#m10-r20), and the rule it leaves behind is: *when the fix
> is a **cache**, the guard's input distribution is the thing under test — a case built with
> `unit.repeat(n)` measures the cache's best case and publishes it as the worst.*

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R20](reviews/M10.md#m10-r20) | The memoization's benefit is a property of input *periodicity*, so M10-R2's DoS is still reachable: a legal 15 MiB body of distinct digit groups costs 64.5 s | **BLOCKER** | [x] |
| [M10-R21](reviews/M10.md#m10-r21) | PHONE-NAT-10 catches the M10-R13 class with 2 of its 2,400 samples, and its non-vacuity floor is aggregate | guard | [x] |
| [M10-R22](reviews/M10.md#m10-r22) | ROADMAP's own M10 prose still publishes the superseded pre-M10-R3 measurement and cites "union-only FPs 0" as the evidence | measurement | [x] |
| [M10-R23](reviews/M10.md#m10-r23) | M10-R19's closure fixed a different link; the `cargo doc` warning it named still fires unchanged | low | [x] |
| [M10-R24](reviews/M10.md#m10-r24) | DOS-05, DEP-02 and CFG-01 are absent from `docs/TESTING.md`, and its complexity model is a milestone behind | docs | [x] |
| [M10-R25](reviews/M10.md#m10-r25) | ARCHITECTURE still names `every_declared_shape_is_needed_by_a_corpus_case`, which M10-R17's fix deleted | low | [x] |
| [M10-R26](reviews/M10.md#m10-r26) | The promoted invariant "a trunk anchor guarantees a candidate can only begin where a number begins" is false, and PHONE-NAT-04 is pinned to the one region set where it cannot fail | guard | [x] |
| [M10-R27](reviews/M10.md#m10-r27) | The M10-R20 budget refuses with an error its own client — an agent — cannot connect to a cause it controls, so it retries identically and wedges | usability | [x] |

> **[M10-R27](reviews/M10.md#m10-r27) came from the maintainer, not from a review round**, and is
> recorded the same way as the rest: *a finding's value does not depend on who noticed it.* The
> question that produced it — "how often can this happen, and does the agent just redo the query
> with a `LIMIT`?" — is one the builder had not asked, having priced the *probability* of the
> refusal without pricing the *failure it produces*. The rule it leaves: **a fail-closed threshold
> is only as good as the failure it produces**; when the party receiving the refusal is an agent,
> "can it act on this?" is a design question. Two alternative fixes (a configurable budget, a higher
> default) are written up in the record as **considered and not taken**, with the reasoning, so the
> next reader inherits the decision rather than re-deriving it.

**Round 4 (2026-07-30): 7 findings, all 7 closed — and six of eight round-3 closures held.** The
fail-open hunt the round was commissioned for came back **empty**: an exhausted budget is an `Err` on
every path (`detect` · `FailOpen` · `CompositeDetector` · `CachingDetector` · the `mask_all`
fixpoint · the response path), and the walk is written out in the record. Both feature sets were green
at the time (213 / 246, zero warnings), `cargo doc` 15 warnings all pre-existing, and every
`phone_eval` figure reproduced exactly. [M10-R20](reviews/M10.md#m10-r20)'s closure did **not** hold
and [M10-R26](reviews/M10.md#m10-r26)'s did not either; both were re-opened as new findings rather
than quietly re-scored.

> **[M10-R28](reviews/M10.md#m10-r28) was the tag blocker, and the fail-open hunt coming back empty is
> what makes it interesting.** The budget bounded a *field* while the body chooses its field count, so
> the DoS M10-R20 was raised for was reachable unchanged — measured against a **pre-fix build on the
> identical body**, HEAD and `9751847` were indistinguishable. Its fix is one `Budget` per request,
> threaded through a new `try_detect_within` seam. And the *reason* nothing failed open — nothing wraps
> the structured recognizers in `FailOpen` **today** — turned out to be the defect underneath: a
> property of the wiring, not of the code. `FailOpen` now swallows a failed *detector* and propagates
> an exhausted *request*.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R28](reviews/M10.md#m10-r28) | The budget bounds a **field**, not a request: the same 15.6 MiB split across 78 fields answers 200 in 57 s, unchanged by its own fix | **BLOCKER** | [x] |
| [M10-R29](reviews/M10.md#m10-r29) | `MAX_PHONE_VALIDATIONS_PER_FIELD` is spent by every validating recognizer, so the always-on national-ID tier refuses legal requests with the phone tier off | correctness | [x] |
| [M10-R30](reviews/M10.md#m10-r30) | The published bound is per *pass*, not per field — `mask_all` re-mints it up to five times, and a sub-budget field measures 2–4× the published 0.5 s | measurement | [x] |
| [M10-R31](reviews/M10.md#m10-r31) | M10-R26's closure corrected ARCHITECTURE and left the disproved sentence in the source, at the line the finding named | guard | [x] |
| [M10-R32](reviews/M10.md#m10-r32) | M10-R27 changed client-visible behaviour with no test; DOS-06's assertions pass unchanged on the message it replaced | guard | [x] |
| [M10-R33](reviews/M10.md#m10-r33) | Both READMEs' fail-closed list omits the 400 this milestone added, and the "linear under load" bullet beside it is contradicted by measurement | docs | [x] |
| [M10-R34](reviews/M10.md#m10-r34) | PHONE-NAT-10's country-code aim names five regions and carries four | low | [x] |

> **All seven closed (2026-07-30), in two passes and deliberately in that order.** R31 · R32 · R34
> went first as they needed no decision; R33 was **half**-closed and held at `[ ]` on its own
> instruction, because its linearity bullet claims a per-request CPU bound the code did not yet have.
> Then R29 (what the budget counts) and R28 (what unit it belongs to), which the maintainer settled —
> **R29 first, or R28's number gets measured against work the budget was never meant to count.** R30
> and R33's second half fell out of R28 as predicted: the seam is the same one.
>
> **The threshold moved because of a legal payload, not an adversarial one.** Charging per `parse()`
> instead of per candidate (R29) shrank the effective allowance ~9×, and at 50,000 units an ordinary
> **367 KB database tool result with one phone column** came back a `400`. Raised to **500,000** on
> the maintainer's call. *A fail-closed threshold whose refusal is a routine event is the wrong
> threshold.* Not an environment variable: a CPU bound an operator can raise is not a bound.
>
> *(The figures this block first carried — "0.19 s", "~1.4 s", "≈50,000 numbers" — were measured
> before round 5 found that only pass 0 was charged, and are corrected below and in
> [ARCHITECTURE](ARCHITECTURE.md). The threshold survived the correction; the numbers around it did
> not.)*
>
> Two things outlived the round, both promoted. *A budget scoped to a unit the client can multiply is
> a rate, not a bound* ([ARCHITECTURE](ARCHITECTURE.md)), and its testing companion — *when a guard's
> unit is smaller than the attack's, no number of axes inside its own unit will reach the attack*
> ([TESTING](TESTING.md#algorithmic-complexity-guards)). And `FailOpen` now distinguishes a failed
> **detector** from an exhausted **request** allowance: round 4 hunted for a fail-open path and found
> none, and the reason none existed — nothing wraps the structured recognizers in `FailOpen` *today* —
> was itself the defect.

**Round 5 (2026-07-30) — closure verification: four of seven hold.** Both feature sets green at the
time (**217 default / 250 onnx**, zero warnings; `fmt` and `clippy -D warnings` clean on both;
`cargo doc` 15 warnings, all pre-existing), DOS-BUD's unit counts reproduced exactly, and two closures
were verified by **mutating** the tree in a throwaway worktree rather than by reading it (E2E-05 fails
when the actionable clause is stripped; M10-R34's modulus really is what moved the floor). Seven new
findings, one of them the reason `v1.2.1` still cannot be cut:
[M10-R28](reviews/M10.md#m10-r28)'s closure and [M10-R30](reviews/M10.md#m10-r30)'s do **not** hold,
and [M10-R33](reviews/M10.md#m10-r33)'s no longer does either. All three were re-opened as new
findings rather than re-scored.

> **[M10-R35](reviews/M10.md#m10-r35) is the tag blocker, and it is one missing method.** Round 4 made
> `CompositeDetector`, `CachingDetector` and `FailOpen` forward the budget explicitly, and warned in
> the trait's own doc that inheriting the default would hand the detector underneath a fresh
> allowance. `StructuredRecognizers` — the one detector whose cost the budget exists to bound, and the
> only place in the tree that *mints* an allowance — overrides `try_detect_within` and **not**
> `redetect_within`, so every fixpoint pass after the first starts from a full 500,000.
> `redetect_within(200 KB of groups, Budget::new(1))` returns `Ok` with **0 spent**. Measured on the
> real `.exe` at the shipped default: a legal **15.63 MiB body answers `200` in 17.2 s**, against
> 0.36 s with the tier off and a published per-request ceiling of **~1.4 s**. The one-line fix makes it
> a `400` in 2.2 s — and leaves the whole suite green on 217, which is why nothing saw it. *The
> obligation to carry a budget is not the wrapper's; it is every implementor's, and the sharpest case
> is the leaf, where the default does not ignore the budget but replaces it.*

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R35](reviews/M10.md#m10-r35) | The budget bounds **pass 0** of a request, not a request: `StructuredRecognizers` never overrides `redetect_within`, so a legal 15.63 MiB body answers 200 in 17.2 s | **BLOCKER** | [x] |
| [M10-R36](reviews/M10.md#m10-r36) | The headline "refused in 0.19 s" does not reproduce and contradicts the row above it in its own table — DOS-BUD prints 3.07 s | measurement | [x] |
| [M10-R37](reviews/M10.md#m10-r37) | Both READMEs' fail-closed bullet still describes the pre-M10-R28 unit: "a **single field** … exhausts the budget", remedy "shrink the field" | docs | [x] |
| [M10-R38](reviews/M10.md#m10-r38) | The budget constant's own SQL table publishes a row DOS-BUD refutes, and the "≈50,000 numbers per request" figure resting on it is ~1.7× pessimistic | measurement | [x] |
| [M10-R39](reviews/M10.md#m10-r39) | `mask_all_within`'s doc says `mask_all` passes `Budget::unlimited`; twenty lines above, `mask_all` passes a real allowance and says so | low | [x] |
| [M10-R40](reviews/M10.md#m10-r40) | E2E-05's digit-run check exempts every run shorter than four digits, while the record claims every run is checked | low | [x] |
| [M10-R41](reviews/M10.md#m10-r41) | `FailOpen` identifies a budget refusal by a global side-condition, not by the error; correct today only via an invariant nothing states | low | [x] |

> **All seven closed (2026-07-30), and R35 was closed by deleting the seam rather than filling it.**
> The maintainer chose the structural fix over the one-line override: `try_detect` and `redetect`
> now **take** a `&Budget`, the `_within` pair is gone, and `Vault::mask_all` lost its budget-less
> convenience for the same reason. *An obligation a trait default can satisfy is not carried by the
> type system — and "every test passes" is the signature of that, not evidence against it.*
> **DOS-08** is the guard whose absence let it through. R41 was closed the same way — `DetectError`
> carries the distinction, so `FailOpen` reads the error instead of correlating with a global.
>
> *(Round 6 found both closures had claimed more than they delivered: DOS-08 quantified over the
> **leaf type**, not the chain, so the same defect in a wrapper stayed green —
> [M10-R42](reviews/M10.md#m10-r42); and `try_detect`'s own default still routed to a minting,
> swallowing `detect` — [M10-R44](reviews/M10.md#m10-r44). Both closed; see the Round 6 block.)*
>
> **The threshold survived; the numbers around it did not.** Charging the fixpoint's later passes
> roughly **doubles** what a masking body spends, so the refusal line for a phone-bearing database
> result moved from ~90,000 rows to **~25,000–35,000**. 500,000 stays: 5,000 rows — the ordinary
> `tool_result` — spends 100,010 of it, and a real 22 KiB Claude Code turn still spends **0**.
> Re-measured and republished in full, from DOS-BUD's own rows (R36 · R38): the 15.6 MiB / 78-field
> body is **refused in 2.24 s** (was `200` in 57 s), and the honest ceiling is *~1.6 s of validation
> plus work linear in the body* — the slowest legal body measured is **5.2 s**, not the 1.4 s this
> ledger claimed a round ago. **218 default / 251 onnx green**, `fmt`, `clippy -D warnings` and
> `cargo doc`'s 15-warning baseline all clean.

**Round 6 (2026-07-30) — closure verification: six of seven hold.** Both feature sets green (**218
default / 251 onnx**, zero warnings; `fmt` and `clippy -D warnings` clean on both; `cargo doc` 15
warnings, all pre-existing). [M10-R35](reviews/M10.md#m10-r35)'s repro was re-run on the **real
release `.exe`** — 15.63 MiB, shipped default, counting mock upstream: **`400` in 1.0 s**, upstream
never contacted, against `200` in 17.2 s a commit ago. DOS-BUD's unit counts reproduce **exactly, row
for row**. Five new findings, none of them a live leak or a live DoS:

> **The fix is right; two of the claims made *about* it are not, and both are M10-R35's own shape one
> level over.** DOS-08 reds when the defect is reintroduced in the leaf — verified by mutation, so it
> is not decoration — but it constructs a bare `StructuredRecognizers`, so the identical one-line
> defect in a **shipped wrapper** (`CachingDetector::redetect`) leaves all 218 tests green while the
> request path under-charges by **21×** and forwards what it must refuse
> ([M10-R42](reviews/M10.md#m10-r42)). And *"no method remains that a default could route to which
> would mint another"* is false as written: `try_detect`'s **own** default routes to `detect`, which
> mints *and* swallows the refusal — a five-line `detect`-only wrapper forwards DOS-07's body with a
> partial scan reported as clean ([M10-R44](reviews/M10.md#m10-r44)). Neither is reachable in the
> shipped tree today; neither was M10-R35's, until the leaf did not override.
>
> **And the milestone's other recurring failure closed one of its two homes.**
> [M10-R38](reviews/M10.md#m10-r38) re-measured every figure into ARCHITECTURE and never touched the
> constant the finding was named after: `MAX_PHONE_VALIDATIONS_PER_REQUEST` still publishes the
> pre-R35 SQL table and *"≈50,000 phone numbers per request"*, now **2× optimistic in the reassuring
> direction** ([M10-R43](reviews/M10.md#m10-r43)). *A closure that re-measures a figure must fix
> every site the finding listed — and the finding's own title names the first one.*

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R42](reviews/M10.md#m10-r42) | DOS-08 quantifies over `StructuredRecognizers`, not the chain: the same defect in a **wrapper** leaves all 218 tests green while the request under-charges 21× | guard | [x] |
| [M10-R43](reviews/M10.md#m10-r43) | The budget constant's own doc still publishes the pre-M10-R35 table and `≈50,000 numbers per request` — M10-R38's closure updated ARCHITECTURE and not the artefact it named | measurement | [x] |
| [M10-R44](reviews/M10.md#m10-r44) | `try_detect`'s own default routes to `detect`, which mints **and** swallows: a five-line `detect`-only wrapper forwards DOS-07's body with a partial scan reported as clean | guard | [x] |
| [M10-R45](reviews/M10.md#m10-r45) | `DetectError`'s fields are `pub` and the struct is not `#[non_exhaustive]`, so a literal bypasses both constructors — *"a new error site has to choose"* is a convention | low | [x] |
| [M10-R46](reviews/M10.md#m10-r46) | Two **present-tense** comments still name the deleted `try_detect_within`, both in the file that deleted it | low | [x] |

**Round 7 (2026-07-30) — closure verification: all five of round 6's hold**, and the two that needed a
mutated tree got one. With `CachingDetector::redetect` minting again, **DOS-08 and DOS-09 are the only
two failures in the entire default suite** (217 / 2, `--no-fail-fast`) — R42's guard is not decoration.
A `detect`-only implementor is **`error[E0046]`**, so R44's inversion is carried by the compiler. Every
unit count the constant and ARCHITECTURE publish reproduces **exactly** on two DOS-BUD runs. The real
release `.exe` was driven end to end on OS-assigned ports: a PII turn is masked upstream
(`[EMAIL_1] [PHONE_1] [IBAN_1]`), augmented, and **restored** to the client, and a 15.63 MiB body is a
**`400` in 1.36 s** with the upstream never contacted. Both feature sets green (**219 / 252**, zero
warnings; `fmt`, `clippy -D warnings`, `cargo doc`'s 15-warning baseline all clean). Six new findings,
none a live leak, none a live DoS, **none changing product behaviour**.

> **The one line nothing asserts.** `FailOpen` swallows a failed *detector* and propagates an exhausted
> *request* — the distinction [M10-R41](reviews/M10.md#m10-r41) moved onto the error so the wrapper
> could read it. **Delete that line and the suite is 219/219 green**, and DOS-07's own body driven
> through `Caching(Composite([FailOpen(Structured)])))` is **forwarded**: a partially scanned body with
> a clean bill of health, which is the failure R41 exists to prevent. R41's own *"test that would have
> caught it"* prescribed exactly that guard; the closure took the fix and left the guard
> ([M10-R47](reviews/M10.md#m10-r47)). The reason no guard sees it is
> [M10-R48](reviews/M10.md#m10-r48): `shipped_chains()` claims to be *"every arrangement `AppState::new`
> can build"* and is a hand-written four that includes a shape the wiring never builds (`bare`) and
> omits the one it does (`FailOpen`) — **R42's own defect one level up: the guard was aimed at a type,
> so it was widened to a list, and the list is the new instance.** TESTING's axis row records it with
> the `—` that same finding declared is never the honest answer.

> **And the numbers moved again — this time the harness disagrees with the prose.** The refusal line is
> **not** 25,000–35,000 rows: DOS-BUD prints no row between 20,000 and 50,000, and running them shows
> **40,000 rows masked at 490,010 units**, so the line is ≈41,000. Its *"one phone column"* is eleven
> digits where an Italian mobile is ten, so **zero phones are masked** in the payload the threshold was
> chosen against — a column that really validates spends 284,500 units at 40,000 rows and refuses near
> 70,000 ([M10-R49](reviews/M10.md#m10-r49)). And the **5.2 s** worst case both READMEs publish measures
> **1.46 s** here on an idle box, is contradicted by the same table's 3 MiB row at the same 500,000
> units, and cannot be derived from the two-term model printed beside it — the mechanism is contention,
> reproduced directly (the same command printed 3.64 s with a build running and 1.31 s alone)
> ([M10-R50](reviews/M10.md#m10-r50)). *Wall clocks are machine- and load-dependent; the units are not,
> and only one of the two columns says so.*

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R47](reviews/M10.md#m10-r47) | `FailOpen`'s "never swallow a budget refusal" is asserted by no test: delete the line and the suite is 219/219 green while a `FailOpen(structured)` chain forwards DOS-07's body | guard | [x] |
| [M10-R48](reviews/M10.md#m10-r48) | `shipped_chains()` says *"every arrangement `AppState::new` can build"* and is a hand-written four — wrong in both directions, with nothing tying it to `build_detector` | guard | [x] |
| [M10-R49](reviews/M10.md#m10-r49) | The refusal line is ~41,000 rows, not 25,000–35,000 — and DOS-BUD's *"phone column"* is 11 digits, so it masks **zero** phones at every size | measurement | [x] |
| [M10-R50](reviews/M10.md#m10-r50) | The **5.2 s** worst case in both READMEs measures 1.46 s idle, contradicts the two-term model beside it, and is attributed to masking its row never did | measurement | [x] |
| [M10-R51](reviews/M10.md#m10-r51) | Two round-6 closures each left a site their own finding named, both in `TESTING.md`: DOS-08's entry still claims what M10-R42 disproved, E2E-05 is still *1 MiB* | docs | [x] |
| [M10-R52](reviews/M10.md#m10-r52) | *"The two constructors are the only way to build a `DetectError`"* is false in every module below `pii` — which is all four error sites | low | [x] |

> **All six closed (2026-07-30), and two of them were numbers this project had published.** R47 is the
> one that mattered: `FailOpen`'s *"never swallow a budget refusal"* was asserted by **nothing** for
> two rounds — delete the line and the whole suite stayed green while a body the request path must
> refuse was forwarded. **FAILOPEN-BUD** now asserts both sides of the distinction, verified by
> mutation. R48 put the three `FailOpen` positions into `shipped_chains()` and, where the list still
> cannot be derived from the wiring, says so instead of implying otherwise.
>
> **R49 is the one to remember.** DOS-BUD's "phone column" was **eleven digits**, and no Italian plan
> accepts eleven — so the harness that exists to answer *"can real traffic reach the budget?"* masked
> **nothing at any size**, and every unit it published was the cost of *rejecting* non-numbers. Its
> verdict column printed `0 left`, which is what a correctly-masked column prints too: an instrument
> that could not tell success from vacancy. With a real column the answer inverts — `.any()`
> short-circuits on **accept**, so a valid number costs ~1 unit against ~9 for a rejected one, and the
> largest phone-bearing request the proxy accepts (**16 MiB, 221,941 numbers**) spends **221,941 of
> 500,000**. *A legal phone-bearing body cannot reach the allowance at all*; `MAX_BODY_BYTES` binds
> first. What reaches it is text that **fails** validation — the adversarial shape, which is the right
> thing for a fail-closed bound to be reachable by.
>
> R50 retired the **5.2 s** worst case: it was measured under load *and* on the broken column. Idle,
> the slowest legal body is that 16 MiB result at **2.56 s** and every refusal lands at ~1.4–1.9 s.
> Third time a contended measurement reached a product doc — *a published number is measured on an
> idle box or it is not measured.* R51 fixed two sites round 6's own closures had named (fourth and
> fifth of that class), and R52 recorded that a private field reaches a module's **descendants**, so
> the `DetectError` literal still compiles inside `pii::*` — the claim was weaker than it looked, and
> the record says so rather than the code pretending otherwise.
>
> **220 default / 253 onnx green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc`
> baseline all clean.

**Round 8 (2026-07-30) — closure verification: all six round-7 closures hold as code**, two of them
by mutation. Deleting R47's guard clause reds **FAILOPEN-BUD *and* DOS-08** — R48's `FailOpen`
positions put the wrapper inside `shipped_chains()`, so the two closures reinforce each other — and
disabling the refusal outright reds **six** guards, every one that exists for the fail-closed budget.
DOS-BUD's unit counts reproduce exactly on an idle box. **Three new findings, none needing a code
change, and the first one blocks the tag's docs.**

> **[M10-R53](reviews/M10.md#m10-r53) is the fourth publication of these numbers and the fourth to be
> wrong — this time in the *optimistic* direction, which is new.** *"A legal phone-bearing body cannot
> reach the allowance at all"* rests on `.any()` short-circuiting on accept, and that reasoning holds
> only for DOS-BUD's own rendering. `347 XXXXXXX` is `LongBlock`, the **only** shape family with a
> single applicable region, and the only rendering no other family's regex can match inside: 1 unit.
> Every other legal rendering costs 2–29, because the overlapping rescan proposes sub-candidates that
> each pay their family's whole region list. (And *"pays for all nine"* is wrong too — the validator
> loops the regions of one **shape family**, at most six.) Measured: five of six legal 16 MiB
> phone-bearing payloads are **refused**, including DOS-BUD's own table in the grouped rendering — and
> through the real `.exe` a **2.6 MB** `SELECT name, phone` export is a `400`. Nothing leaks and
> nothing is forwarded; the fail-closed path is correct. What is wrong is the availability envelope
> four documents publish. The rule to carry: **the cost of a phone candidate is a property of its
> *rendering*, not of its validity** — M10-R43's "a rate measured at one scale does not transfer"
> with `scale` replaced by `shape`.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R53](reviews/M10.md#m10-r53) | *"A legal phone-bearing body cannot reach the allowance at all"* is false in four documents: five of six legal 16 MiB payloads are refused, and the binary refuses a **2.6 MB** contact export | measurement | [x] |
| [M10-R54](reviews/M10.md#m10-r54) | DOS-BUD holds the phone **rendering** constant, so M10-R49 gave it a non-vacuity assertion and left it a single-point grid | guard | [x] |
| [M10-R55](reviews/M10.md#m10-r55) | Round 7 left three more doc sites its own findings named — and M10-R48's closure note asserts a TESTING edit the diff does not contain | docs | [x] |

> **All three closed (2026-07-30). The threshold stays at 500,000 — the maintainer's call — and the
> documents now say what the product does.** [M10-R53](reviews/M10.md#m10-r53) is the sharpest finding
> of the milestone that is not a defect: *"a legal phone-bearing body cannot reach the allowance"* was
> published in four places on the strength of one measurement, and the measurement was of
> `347 1234567` — the **global minimum**. `LongBlock` is the only single-region family and the only
> shape no other family's regex matches inside, so it costs **1** unit where `320 123 4567` costs 12,
> `612 34 56 78` costs 26 and `01 23 45 67 89` costs **29**: `Scan::Overlapping` makes every other
> rendering propose sub-candidates from inside itself, each rejected and each paying its family's whole
> region list. Measured through the real `.exe`: a **2.6 MB** `SELECT name, phone` export is a `400`.
> Nothing leaks and nothing is forwarded — the refusal is the fail-closed path working — but the
> availability envelope was wrong in the **optimistic** direction, which is a first for this milestone.
>
> **The rule: *a conclusion drawn from one point of a grid is a fact about that point.*** DOS-BUD now
> varies the rendering (R54) and its axis row names what it holds constant; the per-rendering unit
> table and a two-rendering `MAX_BODY_BYTES` row are in the harness, so the band — refusals start
> around **2.6 MB** grouped and **6 MB** at the cheapest rendering — is re-runnable rather than
> quoted. R55 closed the 7th–9th instance of *"a closure skipped a site its own finding named"*, and
> `FAILOPEN-BUD` is now in the test catalogue where R47 said it belonged.
>
> **Writing R54's second rendering reproduced the milestone's oldest trap a sixth time**: the first
> generator had a 10,000-row period, the memo served 95% of a 16 MiB dump for free, and the grouped
> rendering measured *cheaper* than the cheapest — the exact opposite of the finding. It is an odometer
> now. *A modular generator looks varied at every call site; only the aggregate shows it is not.*
>
> **220 default / 253 onnx green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc`
> baseline all clean.

**Round 9 (2026-07-31) — closure verification: all four checked closures hold as *edits*; two do not
hold as *claims*.** R47's guard clause is load-bearing by mutation (deleting it reds FAILOPEN-BUD
**and** DOS-08, and nothing else); R54's odometer is genuinely distinct at 16 MiB in both renderings
(219,096/219,096, counted, and the grouped column really masks — 8.00 units/row, not an R49-style
rejection cost); R55's three sites are filled. Both feature sets green (**220 / 253**, zero warnings;
`fmt`, `clippy -D warnings`, the 15-warning `cargo doc` baseline all clean), and the release
workflow's two guards were *simulated* rather than assumed — `v1.2.1` is correctly blocked on both
axes today. Six new findings. **None is a live defect in `src/`:** rounds 5–9 have now found zero, and
the fail-closed path was driven end to end on the real `.exe` again — masked upstream, restored to the
client, refused bodies never forwarded, the refusal message carrying no input-derived bytes.

> **The replacement numbers are wrong in the same optimistic direction, for the third consecutive
> round — and this time the refutation is unit-exact, not a wall clock.** Four documents say the
> reachable band starts *"around **2.6 MB** (a dense grouped column)"*, and both READMEs name the
> rendering. Measured with DOS-BUD's **own** generator that column costs **8.00 units per row**, so a
> one-column export is refused at **62,500 rows / 793 KB** — 3.3× below the published bound — and a
> `SELECT name, phone` export at **1.88 MB**. Confirmed through the real binary: 799 KB → `400`,
> upstream never contacted ([M10-R56](reviews/M10.md#m10-r56)). The per-candidate table has the same
> defect one level in: *"1 to 29 units"* is one sample per family, and six ordinary French renderings
> cost **46** while masking whole ([M10-R57](reviews/M10.md#m10-r57)). The reason none of this was
> caught is that [M10-R54](reviews/M10.md#m10-r54)'s fix item 2 — *"the one-column density row, **which
> is where the 2.6 MB / 6.0 MB refusal line lives**"* — was not built, so the band is still an
> **off-harness number** ([M10-R58](reviews/M10.md#m10-r58)): the tenth *"a closure skipped a step its
> own finding named"*, in the commit that closed the ninth.
>
> **And the stopping rule's premise does not hold.** *"Both classes now have a mechanical guard"* names
> FAILOPEN-BUD, which guards fail-open behaviour rather than catalogue drift, and *"the catalogue
> entries"*, which are entries — R55's own ~15-line id-extraction guard was never written
> ([M10-R59](reviews/M10.md#m10-r59)). **The code has converged; the instrumentation has not.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M10-R56](reviews/M10.md#m10-r56) | The replacement band is wrong in the same optimistic direction: the grouped column both READMEs name is refused at **793 KB**, not 2.6 MB — and *"multi-megabyte, rare"* is what the threshold decision rests on | measurement | [x] |
| [M10-R57](reviews/M10.md#m10-r57) | *"1 to 29 units"* is one sample per family and the French sample is the cheapest of its family: six ordinary French renderings cost **46**, the overall maximum is **65** | measurement | [x] |
| [M10-R58](reviews/M10.md#m10-r58) | M10-R54's fix item 2 — the density row *"where the refusal line lives"* — was not built, so the published band is still an off-harness number nothing re-runs | guard | [x] |
| [M10-R59](reviews/M10.md#m10-r59) | The tag's stopping rule rests on *"both classes now have a mechanical guard"*; the catalogue guard R55 prescribed was never built, and DOS-BUD's axis is aimed at the instance | guard | [x] |
| [M10-R60](reviews/M10.md#m10-r60) | M10-R53's closure narrowed *"four live sites and two narrative ones"* to *"all four live sites"* — the round-7 ledger block still asserts the refuted claim in the present tense | docs | [x] |
| [M10-R61](reviews/M10.md#m10-r61) | Two mechanical defects introduced by `e704ce6`: a duplicated sentence in ROADMAP's pre-tag list (already gone — that block was rewritten this round) and seven doubled apostrophes in the review record | low | [x] |

<a id="m11"></a>
## M11 — deterministic coverage · NER thread base 🔨

**Three tracks that share a milestone and nothing else.** [A](#m11-a) is the coverage gap the
milestone opened on; [B](#m11-b) and [C](#m11-c) were added 2026-09-02 and both touch the ML
layer — B changed *how many threads* one inference gets, C was to change *which export* runs. They
were listed apart because they fail apart, and they did: **B shipped, and C closed without work**
once the search showed there is no newer export to move to. **What remains of M11 is Track A.**

**M11 is heading for `v1.3.0`** (set 2026-09-02, written `(planned)` in the Status table until the
tag exists). **Minor, not patch, and Track B alone decides that:** the default per-session thread
count changes on every SMT machine with no config change — a behaviour change, not a fix — and the
same is true of Track A's new recognizers. A patch release that
silently halves a thread count is the kind of version number [M10](#m10) spent a milestone
learning not to publish.

<a id="m11-a"></a>
### Track A — deterministic coverage: VAT numbers ✅

**Opened 2026-07-31, from a coverage comparison rather than a defect.** Set against a
document-level anonymizer built on a fine-tuned multilingual encoder over **22** entity types,
this proxy's **10** looks thin — and the useful answer was not *"get to 22"*. The two counts
measure different things: five of those 22 are one address split into
street / number / postcode / city / province where this emits a single `[LOCATION_1]`, which is
resolution rather than reach, and several more are categories this proxy must **not** mask
(below). What the comparison did surface is a real, cheap gap: **identifiers with a verifiable
form that we simply have not written a recognizer for.** That is this track, and nothing else.

**Scope — the deterministic tier only.**
- [x] **Italian Partita IVA** — 11 digits, mod-10 checksum with position doubling. Structurally the
  twin of the Codice Fiscale recognizer that already ships; the work is the corpus, not the code.
  Emits the **new `[TAXID_n]`**, always-on (decision 1 and 2 below).
- [x] **EU VAT numbers (VIES)** — per-country format + checksum, for the countries already covered
  by the national-ID tier. Same family, same always-on posture, one recognizer per country.
  **Five shipped — 🇮🇹 🇩🇪 🇬🇧 🇳🇱 🇵🇹; three did not, and are named rather than silently missing:**
  🇪🇸 (the legal-entity CIF control character), 🇫🇷 (the key over the SIREN) and 🇱🇻 (the
  legal-entity checksum, a different algorithm from the personal code already shipped) could not be
  confirmed against trustworthy real pairs, and an unmeasured recognizer does not ship. `VAT-04`
  asserts their absence so the gap stays a decision rather than a discovery. GB ships despite not
  being VIES since Brexit: it is in the national-ID tier this track takes its country list from, and
  its number is checksum-verifiable — the tier's actual criterion.
- [x] Corpora + adversarial negatives per recognizer, on the `PHONE-NAT` model: a positive set of
  real renderings and a negative set of things that merely look like one. **A category ships when
  it is measured** — the rule that produced the nine phone regions, applied unchanged. Anchored on
  **six real published P.IVAs** plus the German administration's own documented vector; two rates
  measured rather than asserted — the bare-form over-mask cost **0.100** (`VAT-09`) and the
  P.IVA/national-ID naming collision **0.0998** (`VAT-10`).
- [x] **The collision the plan did not foresee, found by a guard from a closed milestone.** A bare
  P.IVA is `\d{11}` — and so is the compact domestic phone shape M10 measured, where
  `02079460958` (a real London number) satisfies the P.IVA mod-10. Ranking `TaxId` above `Phone`
  relabelled **every compact GB and DE number** as `[TAXID_n]`: no leak, but a fidelity regression
  on a measured capability, caught by PHONE-NAT-01. `TaxId` now sits below both `NationalId`
  (conservatism about personhood) and `Phone` (a numbering-plan lookup beats a mod-10 check), and
  `VAT-14` pins it from the side a reintroducing change would be written.
- [x] Catalogue every new guard id in `TESTING.md` (`CAT-01` enforces this) and re-publish the
  coverage tables in both READMEs and `ARCHITECTURE.md`.

**A third decision, taken 2026-09-03 — the published over-mask rate needs its real-traffic number,
and the bar is set here BEFORE that number exists.** The bare Partita IVA recognizer is
`\b\d{11}\b` + mod-10 with **no context required**, and it alone carries the whole measured
**0.100** over-mask cost — the four prefixed schemes (`IT`/`DE`/`GB`/`PT`/`NL`) are effectively
false-positive-free. That rate was shipped on [M4-R6](reviews/M4.md#m4-r6)'s grounds, which is half
the precedent this project actually set: **[M10](#m10) accepted a synthetic rate for the domestic
phone tier *and then proved zero hits on real traffic*** (`tests/phone_overmask.rs`, over the real
~22 KiB Claude Code turn in `tests/common/m7_turn.rs`). The VAT tier has the synthetic half only.

- **The gap is not theoretical.** ASCII word boundaries keep the match to runs of *exactly* eleven
  digits, which excludes both common timestamp widths (10 = Unix seconds, 13 = milliseconds) — so
  what is left exposed is **database ids and order numbers of exactly eleven digits**, and the CC
  battery already guards that a SQL result's `id` survives a round trip intact.
- [x] **The bar, declared before the run:** **zero** bare-form `TaxId` spans over that same 22 KiB
  turn. Clear it and `0.100` stands as a published *synthetic worst case* and the tag proceeds. Miss
  it and this is the **same class as the phone collision** — a fidelity regression on a measured
  capability — and it is fixed before `v1.3.0`, not documented around.
  **Cleared 2026-09-03: 0 `TaxId` spans over 22 823 bytes** (`VAT-15`/`VAT-16`,
  `tests/vat_overmask.rs`), so `0.100` stands as the published synthetic worst case. Two numbers
  qualify it, and both live in [`TESTING.md`](TESTING.md) → `VAT-15` where they get read: the guard
  also asserts **no masked span of any kind is exactly 11 digits** — `TaxId` ranks below
  `NationalId` and `Phone`, so a label filter would have measured the naming rule rather than the
  over-mask — and the **denominator is zero**: the turn holds no 11-digit token at all (longest
  all-digit run: 5). The bar is met, and what it proves is that *this* traffic offers the bare
  P.IVA nothing to bite, not that the recognizer is precise on traffic that does.
- Rejected: requiring a keyword anchor (it would lose the P.IVA sitting alone in a `partita_iva`
  column, which is the primary case, and adds a context-window heuristic this codebase has nowhere
  else) and dropping the bare form (a P.IVA written the way Italians write it would go upstream in
  clear — precision bought by gutting the feature for its primary locale).

**A fourth decision, taken 2026-09-03 — the bare-11-digit collision keeps `Phone` on top, and
this is the one place the milestone knowingly does not deliver decision 1.** Raised by
[M11-R2](reviews/M11.md#m11-r2), which measured what nobody had: **0.775** of *issuable* bare
Partita IVAs reach the model as `[PHONE_n]` rather than `[TAXID_n]` under the shipped default
(`VAT-17`; `00xx` 0.033, `0[1-9]xx` 0.859).

**Why it is a decision and not a defect.** `02079460958` is simultaneously a really-assigned London
number and a mod-10-valid P.IVA. **No rule can separate them** — the same eleven bytes satisfy both
tiers — so one of the two labels is wrong for that string whichever way the order runs, and the
choice is which population eats the error. Nothing leaks under either: both are masked, and the
vault restores byte-identically.

**The order stands, on two grounds.**
1. **Strength of evidence.** `phonenumber::is_valid()` confirms an *assigned* number in a real
   numbering plan. A mod-10 check accepts about one arbitrary 11-digit number in ten (`VAT-09`).
   Where two claims collide, the stronger one names the span.
2. **Direction of regression.** M10's domestic-phone fidelity is *shipped and measured*; the VAT
   tier is new. Trading a measured capability for a new one is the worse direction, and it is the
   precise trade M11 already refused once when `VAT-14` was written.

**What it costs, stated plainly rather than softened:** for roughly three quarters of the bare
form's issuable space, [decision 1](#m11-a)'s entire purpose — a token that tells a consumer
*business* rather than *person* — is **not delivered**. The prefixed forms (`IT…`, `DE…`, `GB…`,
`PT…`, `NL…`) are unaffected, and so is the `00…`-leading bare space, which is where every real
P.IVA in this repo's corpus lives.

**Rejected, and why:**
- **Yield the separator-free arm to `TaxId`** (keep `Phone` ahead on separated renderings). This
  reads like a refinement and is not one: **the two strings `VAT-14` pins — `02079460958` and
  `03012345678` — are themselves separator-free**, so the rule would relabel exactly the numbers
  the guard exists to protect. It undoes `VAT-14` rather than narrowing it, and would require
  re-measuring M10's phone fidelity before the tag.
- **A config variable choosing the winner.** Nobody loses, but it is a third setting with subtle
  semantics in both READMEs and a second shipped configuration whose guards must stay green —
  [M10](#m10) spent a milestone learning what a coverage-changing variable costs forever, and
  `PERF-M7-05` is the standing example of what two shipped shapes cost in guard maintenance.

`VAT-17` pins the number so that a change to this trade has to edit it to land, and
[`ARCHITECTURE.md`](ARCHITECTURE.md)'s priority bullet carries the same figures where the ordering
is explained.

**Two decisions, taken by the maintainer 2026-09-02** (Track A's; B and C carry their own). Each
changes product-visible behaviour, so none was the builder's. They are recorded with their cost and
with what was rejected, because that is what a future reader needs — the verdict alone explains
nothing:

1. **A VAT number gets a new `PiiKind::TaxId` → `[TAXID_n]`**, not a reused `[NATID_n]`. A P.IVA is a
   *business* identifier that is personal data only when it identifies a sole trader, and a token
   unable to distinguish it from a Codice Fiscale destroys that distinction for every consumer
   downstream, permanently. The cost is paid once and in full: a new enum variant (the matches are
   exhaustive), a new slot in the overlap priority order, an eleventh row in the coverage tables of
   both READMEs and `ARCHITECTURE.md`, the prompt-augmentation text — **which is cached (M7.1), so
   its key moves with it** — and every client that pattern-matches placeholders. **Reusing `NATID`
   now and adding `TaxId` later was rejected on purpose:** it converts a free choice today into a
   breaking change to the placeholder vocabulary tomorrow, where the same input silently starts
   emitting a different token.
2. **The VAT tier is always-on**, like the national IDs it joins — not gated by `PII_LOCALES`. It is
   checksum-verified, so a false positive is about as likely as one on an IBAN, and the residual
   cost is an over-mask on a company VAT that is not personal data at all: the **right** side to err
   on, since the vault restores it on the response path and nothing leaves in clear either way.
   Gating it would also have inherited `PII_LOCALES`'s **narrowing** semantics — the one M10 had to
   set in bold in the changelog, where setting the variable yields *less* coverage than leaving it
   unset — and a second tier carrying that subtlety doubles what the README has to explain.

**What Track A deliberately excludes, and why it is not an omission.**
- **Vehicle plates — dropped from this milestone 2026-09-02, and dropped from the deterministic tier
  for good.** A plate has **no checksum in any country**: the only defence is its layout, which is
  what M4-R1 named FP-prone and what would have forced a precision floor, an adversarial corpus and
  a per-country gate to be argued into existence *before the first line of it was worth writing*.
  That cost is the symptom, not the problem. **The problem is that a plate is not the kind of thing
  this tier recognizes.** The deterministic tier exists for identifiers arithmetic can confirm —
  mod-10, mod-97, Luhn — where a match is a *proof* and a miss is impossible. A plate offers no
  proof, so it belongs with the things only a model can judge: it moves to [M12](#m12), alongside
  address.
- **Free-form address, age, gender** — no rule can confirm them; they need a model. Deferred to
  [M12](#m12) — the model swap — rather than solved by switching on a second engine.
- **Dates, times, amounts — excluded on purpose, permanently, as a default.** This is the sharp
  one. A document anonymizer can mask a date: an over-mask is visible to the human holding the
  document, and costs nothing. **This proxy sits on live agent traffic**, where `[DATE_1]` in place
  of `2026-07-31` inside a `tool_use.input` corrupts the call and the agent then does the wrong
  thing quietly. That is precisely the harm [M10](#m10) spent nine review rounds bounding, and the
  reason the CC battery checks that `Read`'s line numbers and a SQL result's `id` survive intact.
  If these are ever wanted, they are opt-in, model-side, and measured for over-mask first.

### Review ledger — M11 → [`reviews/M11.md`](reviews/M11.md)

**This milestone reached code-complete with zero review rounds** — the code landed 2026-09-02, the
builder→reviewer loop rules landed 2026-09-03 (`01fbadd`). The loop opened after the fact, and R0 is the
builder's own pass rather than a round. **Round 1 (2026-09-03) is the first independent one: no leak,
no fail-open, no over-mask regression — five findings, all about guards that cannot go red.**

**Round 2 (2026-09-03) drove the product instead: the native Anthropic walk, the Anthropic SSE
rewriter, `MAX_BODY_BYTES`, a VAT-dense DoS body and both wire schemas through the real `.exe`.
The masking path came back clean — no leak, no fail-open, no over-mask regression, no raw value
in a log. Four findings, all on the net, and two of them are Round 1's own closures failing to hold.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M11-R0](reviews/M11.md#m11-r0) | `cargo fmt --check` red on `main` across four files the M11 commits touched — CI gates on it, so M11 as committed would fail on push | build | [x] |
| [M11-R1](reviews/M11.md#m11-r1) | `VAT-04` asserts the absence of an ES VAT number in a form no ES recognizer could match — a live ES stub keeps the suite green | guard | [x] |
| [M11-R2](reviews/M11.md#m11-r2) | The priority fix created the mirror collision and nothing measures it: 73% of issuable 0-leading P.IVAs are named `[PHONE_n]` under the shipped default, while `VAT-10`'s sweep excludes that shape | fidelity | [x] |
| [M11-R3](reviews/M11.md#m11-r3) | A twelfth `PiiKind` ships with the suite green — `AUG-01` and `from_label` are hand-kept lists the enum does not force you to revisit | guard | [x] |
| [M11-R4](reviews/M11.md#m11-r4) | `CAT-01`'s non-vacuity floor is 20 against 54 declared ids, so a whole guard family can go invisible — and `VAT-OM` is already uncatalogued | guard | [x] |
| [M11-R5](reviews/M11.md#m11-r5) | `VAT-09`'s band cannot go red: `0.100` is structural over a contiguous sweep, not a measurement of the checksum | guard | [x] |
| [M11-R6](reviews/M11.md#m11-r6) | `KIND-01`'s successor chain forces an *arm*, not a place in the walk — a twelfth `PiiKind` still ships green, missing from `ALL` and from `from_label` | guard | [ ] |
| [M11-R7](reviews/M11.md#m11-r7) | `CAT-01`'s floor was justified by a count that was never measured (73 vs 90), and the `//!` mutation its closure cites as proof is green at HEAD | guard | [ ] |
| [M11-R8](reviews/M11.md#m11-r8) | Decision 4 is still written as an open question in `TESTING.md` — VAT-17 and in `recognizers.rs`'s VAT-17 doc comment | docs | [ ] |
| [M11-R9](reviews/M11.md#m11-r9) | `TESTING.md` states `CAT-02`'s matrix as 18 against 15; the matrix holds 18 against 17 | docs | [ ] |

<a id="m11-b"></a>
### Track B — the intra-op thread base: physical cores, not logical threads ✅

**Decided 2026-09-02 — recorded as a decision, not left open as a question.** Until this track,
`onnx::default_intra_threads` divided `available_parallelism()`, the **logical** count, so on the
reference box (6 cores / 12 threads) the shipped default `NER_POOL_SIZE=1` yielded `intra = 12`:
both SMT siblings of every core running the same int8 GEMM, contending for one core's L1d/L2 and
one set of vector units. The base is now the **physical core count**, and it is one rule at every
pool size:

```
base  = min(physical_cores, available_parallelism())
intra = max(1, base / NER_POOL_SIZE)
```

M7's invariant is restated on the new base, keeping the **bounded** form [M7-R4](reviews/M7.md#m7-r4)
insisted on: `pool × intra ≤ base` **while `pool ≤ base`**, and `intra == 1` beyond it, where the
derivation is out of moves.

**Why the same base above `pool = 1` and not only at the default** — the half of the question that
was open, decided the same way. The reason to cap at physical cores is sibling contention for one
core's cache and vector units, and that reason does not weaken when a second session appears: it
applies to *both* sessions. Dividing the logical count from `pool = 2` up would put `2 × 6 = 12`
ONNX threads back on 6 physical cores — reintroducing at `pool = 2` exactly what the change removes
at `pool = 1`, and making the process's total NER thread count **double** between `pool = 1` and
`pool = 2` under a formula whose entire purpose is that the product fits the box. One base, one
rule, no discontinuity. The cost is named and accepted: at `pool = 2` on a 6/12 box the default
falls from 6 to 3 threads per session, leaving the siblings to the runtime's own work (tokio, TLS,
JSON) — which is latency-bound and *does* profit from SMT, unlike the GEMM.

**`min(physical, available_parallelism())`, never `physical` alone — this is the trap.**
`available_parallelism()` honours cgroup quota, CPU affinity masks and Windows job objects; a
physical-core count does **not**, it reports the silicon. Take it bare and a proxy in a 2-CPU
container on a 32-core host derives `intra = 32` — the oversubscription this derivation exists to
prevent, arriving through the fix for it. The `min` also settles the case that raised the question,
**CPUs whose thread count is no longer 2× the core count**: on a hybrid P+E part (say 14 cores /
20 threads) it returns 14 — every core once, no sibling doubling — and with SMT disabled in firmware
the two counts are equal and the `min` is a no-op.

**This settles the rule by decision and mechanism, not by this box's timings — and the milestone
must say so.** [M7-R2](reviews/M7.md#m7-r2) recorded the SMT question as **unresolved** after four
runs whose sign flipped and whose same-configuration spread was ~40%. Track B does not claim to
have resolved it. It adopts the conventional base — physical cores is the standard intra-op
recommendation for GEMM-bound inference — and stops paying for a knob no measurement on this
hardware can read. The sweep below *records* the change; it does not justify it.

**Scope.**
- [x] A **pure** `derive_thread_base(logical, physical: Option<usize>) -> usize` beside
  `derive_intra_threads` in `src/pii/onnx.rs`, so the runner's own box cannot decide whether it is
  correct — THREAD-01's standing rule. `None` (the platform won't say) falls back to `logical`,
  which is today's behaviour exactly: a platform that cannot answer loses nothing.
- [x] Physical detection behind the **`onnx` feature only** (where `available_cores` already lives),
  so the default build's footprint is untouched. `num_cpus::get_physical()` is the candidate — it is
  neither a `*-sys` crate nor a `cc`/`cmake` user, so `tests/dependency_footprint.rs` should stay
  green. **Verify that per target rather than assert it here**, which is the whole lesson of that
  file's 2026-07-31 rewrite. **Verified before anything else was written** (`num_cpus` 1.17.0,
  Windows/MSVC): it builds, reports **6** on the 6/12 reference box, DEP-01 and DEP-02 stay green
  across the whole release matrix, and `cargo tree` finds it in the `onnx` tree only — 0 hits in the
  default one.
- [x] `resolve_pool_and_intra` keeps its single home and its `0`-is-unset symmetry (M7-R1, M7-R5);
  only the base it divides changes. **`GLINER_POOL_SIZE`/`GLINER_INTRA_THREADS` inherit it for
  free** — same function, which is the point of it having one home.
- [x] **The startup log must stay reproducible from its own inputs.** It prints
  `pool_size=… intra_threads=…`, and with a base that is no longer the core count the operator's
  task manager shows, the arithmetic stops being redoable. Log the base, and whether it was the
  physical count or the `min` cap. M7-R5 rejected `pool_size=0, intra_threads=12` for exactly this
  reason: a derived value the logged inputs cannot explain.
- [x] **Extend THREAD-01** along the new axis, as a function of `(logical, physical)`: SMT box
  (12, 6) → 6 · SMT off (6, 6) → 6 · hybrid P+E (20, 14) → 14 · container/affinity (2, 32) → **2**,
  the `min` case · detection unavailable (12, `None`) → 12 · plus the invariant re-asserted in both
  regimes on the new base, still split by regime and never hidden inside a `max`.
- [x] **NER-THREAD-01 must NOT be narrowed to the new base.** It sweeps
  `intra ∈ {1, 2, 4, 6, available_cores()}` to prove thread count changes speed and never detection;
  more partitions is strictly more coverage, so it keeps testing up to the **logical** count even
  though nothing ships there any more.
- [x] Re-run the M7 latency sweep on the new default (PERF-M7-03/05 — **≥3 reps, min + median +
  spread**, non-negotiable per M7-R2) and re-publish the figures the docs quote. **Single-request
  latency is a wash at the default** (`1×6` 2.48× vs `1×12` 2.42× over the pre-M7 shape — inside the
  noise, exactly as M7-R2 predicted this harness could and could not resolve), and **throughput
  improved on both shipped shapes**: `1×6` 0.485 turns/s vs `1×12` 0.419 (+16%), `2×3` 0.664 vs
  `2×6` 0.558 (+19%), the new centralized shape being the fastest row measured. Tables: `TESTING.md`
  → PERF-M7-03/04, DEVLOG 2026-09-02.
- [x] **PERF-M7-05's floor is now per-shape — a consequence the plan did not foresee.** The
  centralized shape lost half its per-session threads (`2×6` → `2×3`) and measured **1.46 / 1.56 /
  1.56 / 1.69×** against the shared **1.5×** floor: a guard straddling its threshold on correct
  code, i.e. intermittently red. Maintainer's call, taken 2026-09-02: **keep both shipped shapes
  asserted, with their own floors** — default ≥1.5×, centralized ≥1.3×. Dropping the centralized
  shape would have re-opened [M7-R1](reviews/M7.md#m7-r1)'s class (a shipped config unwatched);
  lowering both would have thrown away the default's real ~1.9× headroom.
- [x] Docs: `ARCHITECTURE.md` → *NER threading* (formula, invariant, the `min` reason), the
  `NER_INTRA_THREADS` rows in both READMEs (they read `max(1, cores / NER_POOL_SIZE)` today),
  `--help` in `src/main.rs`, `TESTING.md` (THREAD-01), `DEVLOG.md`. **[M7](#m7)'s boxes in this file
  and DEVLOG's S1 get a superseded-by-M11 pointer, not a rewrite** — they are the record of what M7
  shipped and why.

**Product-visible, and it belongs in the release notes:** on every SMT machine the default
per-session thread count **halves** with no config change. The old shape stays one env var away —
`NER_INTRA_THREADS` is an explicit override and already wins over the derivation.

<a id="m11-c"></a>
### Track C — refresh the pinned XLM-R export ✅ *closed 2026-09-02: there is nothing to refresh*

**Closed by the answer, not by the work — and recorded rather than deleted**, because the next
person to ask *"shouldn't we be updating the model?"* deserves the answer without repeating the
search.

**The premise was false.** Track C was written to bump `NER_MODEL_REVISION` off `478a2a3`
(`src/server.rs`) to a newer export of the same base checkpoint. Measured 2026-09-02:

- **`478a2a3` is the head** of `jiting/xlm-roberta-base-ner-hrl_onnx`, unchanged since **2024-10-09**.
- The base checkpoint it exports, `Davlan/xlm-roberta-base-ner-hrl`, has not moved since
  **2023-08-14** (nor has its `large` sibling).

So the pin is already at the head of the export of the model's **final** version. The scope item
*"identify the candidate revision and record what changed in it"* has no possible answer, and the
track closes on that.

**The consequence worth keeping: maintaining this pin is not recurring work.** This model will not
be updated. When something does change it will be a **different model**, which is [M12](#m12) — not
a revision bump wearing its clothes.

> **Where we download the same weights from — a real choice, deliberately left open and not
> urgent.** One other ONNX export of the same base checkpoint exists —
> `tjruesch/xlm-roberta-base-ner-hrl-onnx` — and it matches on everything the guards check:
> identical `id2label` **and order**, same `XLMRobertaForTokenClassification` /
> `max_position_embeddings` 514, same tokenizer, same default file name. It would be a pin change,
> not a model change.
>
> **It is not a newer model.** Same 2023 weights, re-exported, with a slightly different int8
> quantization (278.3 MB vs 278.7 MB, from a byte-identical 1110.1 MB fp32). A recall *improvement*
> is therefore unavailable **by construction** — parity is the best case, and even that must be
> measured, because a different quantization moves the logits.
>
> **The argument to move is supply chain, not accuracy.** Today's source is a personal repo with
> **21 downloads**, untouched for two years, one deletion away from breaking `NER_MODEL_REPO` for
> every operator; the alternative has an organisation behind it and ~100× the traction. **The
> arguments against are concrete too:** it ships no `quantize_config.json` and no `ort_config.json`,
> so *how* it was quantized would stop being recorded — a real loss for a project that pins
> revisions in order to be reproducible — and it declares **MIT** while the base checkpoint is
> **afl-3.0**, which a re-export cannot change.
>
> **Not urgent, because the exposure is already bounded:** `NER_MODEL_PATH` is the
> explicit-local-files route and depends on no repository at all. **If this is ever taken, the bar
> is non-inferiority** — recall must not regress, threshold declared before the numbers are read —
> because the reason to move is provenance, so accuracy only has to hold. The original
> "moves only on a measured improvement" rule cannot apply here: it demands something this family
> cannot produce.

> **Known, and deliberately not this track:** an ONNX export of the **`large`** sibling exists with
> the same nine labels in the same order and the same 514 window, at roughly **2× the int8 size**.
> That is the cheapest real recall upgrade available — no chunking constant to re-derive, no new
> class to map, no export to produce ourselves — but it is a **different checkpoint**, so it is a
> model change sized between here and [M12](#m12). Recorded so it is not re-discovered; not
> scheduled.


<a id="m12"></a>
## M12 — one model for everything not provable 📋

**Scheduled 2026-09-02, out of the backlog.** The way to reach contextual PII — address above all —
is a **better single model**, not XLM-R and GLiNER running side by side, and the target is explicit:
**both** current engines retire in favour of one, which then carries every judgement call the
deterministic tier cannot prove.

**The line this milestone draws, and it is the whole architecture in a sentence.** Not "one model
instead of two", but a rule about *which* work is a model's at all: **what arithmetic can confirm
stays deterministic — what it cannot goes to a single model.** A checksum is a proof (mod-10,
mod-97, Luhn: a match cannot be wrong and a miss cannot happen), and no model should be asked to
re-decide it. Everything else — names, organizations, free-form address, age, gender, vehicle
plates — is a judgement, and judgements belong to one engine rather than two. That rule is why
[M11](#m11)'s Track A keeps VAT numbers and dropped plates: the first are provable, the second are
not.

**Stacking is already known to be a bad trade here, from this project's own numbers.** GLiNER int8's
Person recall is **0.58** against XLM-R's **0.83** ([M8](#m8)), so the second engine is *worse* at
the job the first one exists for; it costs **~510 MB on top** (≈1.07 GB for the pair) and a second
inference pass, on a path where [M7](#m7) measured the NER as **~100% of masking latency**. Two
models is paying twice for a partial upgrade — and the same numbers are the standing warning about
the candidate: a model whose published figures looked strong measured 0.58 here. **Any published
figure is somebody else's measurement on their own distribution until it is re-measured on this
project's corpus.** The shape to aim at is known to exist — encoders under half a gigabyte cover
several times this project's entity count on CPU — so *more coverage for less RAM than the current
pair* is not speculative. Which model it is, is [open](#m12-open).

**Scope.**
- [ ] **Get the candidate into ONNX, or it cannot ship at all.** The runtime is `ort` and nothing
  else, so an ONNX export is a **precondition, not a detail**. If upstream has not published one by
  the time this opens, **we export and quantize it ourselves** (maintainer's call, 2026-09-02) — and
  that decision carries a consequence worth naming up front: we then *own* the artifact, so
  `NER_MODEL_REPO` + `NER_MODEL_REVISION` have no upstream revision to pin unless we publish our own
  export, and the explicit-local-path route (`NER_MODEL_PATH`) becomes the only reproducible one.
- [ ] **Map the classes this proxy refuses to nothing.** A candidate trained for document
  anonymization will carry **dates, times and amounts** — which [M11](#m11) excludes permanently,
  because `[DATE_1]` inside a `tool_use.input` corrupts an agent's call. The mechanism exists
  (`DATE` already maps to `None`); the risk is a *new* class nobody mapped.
- [ ] **Re-measure everything the swap invalidates**, before it is a default: the recall corpus, the
  per-kind figures both READMEs publish, the RAM and latency numbers, and the chunking constants —
  `MODEL_MAX_TOKENS`, `MAX_WINDOW_TOKENS` and the **measured** cut-edge drift are tuned to the
  current export and are not portable to a different architecture or context window.
- [ ] **Re-run the determinism guard.** `ARCHITECTURE.md` → *Execution providers* says outright that
  a model swap must: intra-op thread-count inertness w.r.t. detection is **empirical** on the
  current export, not promised by the runtime (M7-R3).
- [ ] **Retire GLiNER** — its config surface (`GLINER_*`), its decode path and its guards — only
  once the replacement measures at least as well on what GLiNER was added for. Removing it first
  would be a coverage regression taken on faith.
- [ ] Re-publish the coverage tables in both READMEs and `ARCHITECTURE.md`, and say plainly what
  `[PERSON_n]`/`[ORG_n]`/`[LOCATION_n]` mean afterwards — **operators already depend on those in
  live traffic**, which is what makes this a milestone of its own rather than an upgrade dropped
  into a patch.

<a id="m12-open"></a>
**Open, and the maintainer's.** *Which* model. Its evaluation stays out of this repository by
standing instruction — record here only the design consequences, never a name or a link.

<a id="backlog"></a>
## Backlog — documented, not scheduled

### One model with more kinds — *scheduled as [M12](#m12)*
Moved out of the backlog on **2026-09-02**: the direction stopped being "documented, not
scheduled" the moment a concrete route to it existed. Its reasoning, its preconditions and
what it re-opens all live in [M12](#m12) — one home, so this section cannot drift from it.

### Option B — native provider adapters
The heavy alternative to M3's Option A: support a provider's **native** API instead of its OpenAI-compat
endpoint, via a **per-provider, schema-aware masking adapter** — **higher leak risk, because a missed schema
field is a leak.** The rule holds: unscheduled until a concrete **client with a real user** needs a native
schema that Option A can't serve.

**The Anthropic slice is done — [M6](#m6) shipped native `POST /v1/messages` for Claude Code, and it is the
blueprint** for any future adapter ("native-in, mask-in-place": walk the native schema directly, no OpenAI
round-trip). What remains here is narrower than "every provider", because **Option A already covers any
client that speaks OpenAI-compat** — OpenAI, Azure OpenAI, Copilot, and the OpenAI-compat endpoints
Gemini / Mistral / Cohere all expose. A *native* adapter earns its leak risk only for a client that speaks a
provider's **native** protocol:
- **Google Gemini native** (`generateContent`: `contents[].parts[]`, `systemInstruction`,
  `functionCall` / `functionResponse`, `tools[].functionDeclarations`) — **the one strong analogue to the
  M6 trigger**: a native **Gemini CLI** is to Gemini what Claude Code is to Anthropic. If a real user wants
  to front it, this is the next native adapter, following the M6 blueprint.
- **Bedrock / Vertex** hosting endpoints — possible but heavier: the hard part is **native auth** (SigV4 /
  GCP), not the schema. Already named out-of-scope by [M6](#m6).

**Recommendation: keep, don't delete.** It records *why* Option A was chosen and names the single realistic
next trigger (Gemini) — deleting it would just re-raise both questions later. Not deleted, not scheduled: it
waits for a real user behind a native client, exactly as the Anthropic slice waited for Claude Code.

### Request-level provider routing *(moved out of M3)*
Provider selection is **per-instance** today (`UPSTREAM_PROVIDER`, chosen at startup), which already covers
Copilot + Anthropic: run **one proxy instance per provider** and point each client at the right port — no
code needed. A *single* instance choosing per **request** needs a provider **map** in `Config` (N base URLs
/ keys / presets) + a selection rule, and doing it well shades into Option B territory. **If pursued,
prefer routing by the request's `model`** — no client changes, no custom headers, composes with the
existing presets; keep it opt-in. **Not a privacy change** — masking runs *before* routing, so a mis-route
is a wrong-provider error, never a leak.

### Should `cargo doc --no-deps` join the "green with no warnings" bar? *(raised by [M10-R10](reviews/M10.md#m10-r10) / [M10-R19](reviews/M10.md#m10-r19))*
It does not today, and the consequence was measured rather than argued: M10 added two intra-doc-link
warnings and **nobody noticed**, because ~14 already existed. Almost all are one mechanical class — a
public doc comment linking a private item — so the cleanup is small and the question is really about
the bar, not the work.

**Deliberately not decided inside M10.** It affects code that milestone never touched, and a
project-wide quality bar set as a side effect of a patch release is a bar nobody agreed to. The
argument for it is the one this repo already makes about `clippy`: *a warning channel with 14
standing entries cannot surface the fifteenth*, and these dense doc comments are exactly where this
codebase's reasoning lives.

### Other later items
Auth & rate-limiting stages · TLS (or running behind a TLS terminator) · config-file support & container
deployment · additional providers · metrics/observability.

> **Sharpened 2026-07-29.** Running the *released binary* is now documented in README → Quick start
> as plain commands. Two things were built and deleted rather than shipped: a `deploy/` env-file +
> launcher-script pair (machinery around a config model that is already one line per variable), and
> a systemd unit — because **a hand-rolled unit per init system is the wrong shape, and a published
> OCI image is the answer that generalizes**. So "run it as a service" is deliberately parked here
> rather than half-answered elsewhere. The other half — the binary reading a config file itself —
> stays deliberately not done.

> The **never-log-raw-PII** rule is **not** a backlog item — it is an enforced quality bar *today*
> (kind/placeholder-only logging, guarded by `tests/log_safety.rs`, DBG-02). A dedicated structured
> **audit-trail** feature could be added here only if a compliance need arises.

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
| [M8.1 — national phone recognizer (opt-in)](#m81) | ✅ complete |
| [M9 — GPU optimization](#m9) | 🔨 code-complete |

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
flip driven through the real model — an operator setting nothing now resolves to `pool=1 intra=12` (the
whole box for a lone request, half the RAM), `NER_POOL_SIZE=2` gives the pooled `2×6`, both clearing the
`≥1.5×` floor. No detection code changed; no leak, fail-open, over-mask or determinism impact — pool is a
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
  the NER and GLiNER; **all seven `ort` EPs wired, DirectML tested, the rest opt-in + untested**,
  each with a logged **CPU fallback** (`onnx::build_session`). Design + tested/untested table:
  [ARCHITECTURE](ARCHITECTURE.md) → *Execution providers*. (DEVLOG 2026-07-19.)
- [x] Quantization tuning; benchmark GPU-fp16 vs the CPU-int8 baseline — **measured: NO-GO on this
  iGPU** (below). CPU-int8 stays the default.
- [x] `--bench-providers` — the binary measures the **model × provider** matrix on the operator's own
  machine and names the winner, because the answer is hardware-specific. (DEVLOG 2026-07-19.)

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

- [ ] **Builder→reviewer loop** — round 1 (2026-07-20): 11 findings, **none a leak**. Verified
  independently: 143 lib + all integration tests green, `clippy-onnx` / `clippy-directml` clean, and the
  DirectML verdict **reproduced with the shipped tool** (CPU-int8 27.1/46.5/108.4 ms vs DML-int8
  101.1/346.1/599.3 ms). Full record: [reviews/M9.md](reviews/M9.md).

| Finding | Title | Severity | Done |
|---|---|---|---|
| [M9-R1](reviews/M9.md#m9-r1) | The server logs the **requested** provider as if it were the **effective** one — `build_session_reporting` was wired to the benchmark, not to production | observability | [ ] |
| [M9-R2](reviews/M9.md#m9-r2) | A session pool can end up **heterogeneous** (some sessions on GPU, some on CPU), making the backend a per-request variable | correctness | [ ] |
| [M9-R3](reviews/M9.md#m9-r3) | `--bench-providers` ignores `NER_INTRA_THREADS` / `NER_POOL_SIZE`, so its CPU baseline isn't the CPU the server runs — M7-R1's class, in a new place | correctness | [ ] |
| [M9-R4](reviews/M9.md#m9-r4) | An unrecognized CLI argument (`--bench-provider`, `--help`) silently **starts a live proxy** instead of failing | hardening | [ ] |
| [M9-R5](reviews/M9.md#m9-r5) | `engaged()` / `ok` reports **registration**, not execution: ORT's per-node CPU fallback is counted as an accelerator measurement | docs | [ ] |
| [M9-R6](reviews/M9.md#m9-r6) | ARCHITECTURE contradicts itself on the fallback's safety in adjacent paragraphs ("bit-identical on any backend" vs "subtly different logits") | docs | [ ] |
| [M9-R7](reviews/M9.md#m9-r7) | The provider table marks DirectML "✅ tested" like CPU, while the prose says no EP is trusted until the determinism guard is re-run | docs | [ ] |
| [M9-R8](reviews/M9.md#m9-r8) | `--bench-providers` cannot benchmark a BERT-family model — it never sends `token_type_ids` | correctness | [ ] |
| [M9-R9](reviews/M9.md#m9-r9) | The README sample output is fabricated, and demonstrates the exact coupled-axes error the section warns against | docs | [ ] |
| [M9-R10](reviews/M9.md#m9-r10) | Source doc drift: `onnx.rs` still says GPU "comes later (M4)"; `load_onnx_ner`'s doc block was orphaned onto `resolve_ner_model` | docs | [ ] |
| [M9-R11](reviews/M9.md#m9-r11) | The CPU fallback is **initialization-only**; a mid-life EP failure degrades silently and never re-arms, while SETUP says "always safe to try" | docs | [ ] |

<a id="backlog"></a>
## Backlog — documented, not scheduled

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

### Other later items
Auth & rate-limiting stages · TLS (or running behind a TLS terminator) · config-file support & container
deployment · additional providers · metrics/observability.

> The **never-log-raw-PII** rule is **not** a backlog item — it is an enforced quality bar *today*
> (kind/placeholder-only logging, guarded by `tests/log_safety.rs`, DBG-02). A dedicated structured
> **audit-trail** feature could be added here only if a compliance need arises.

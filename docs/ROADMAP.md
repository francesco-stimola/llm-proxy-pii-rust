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

This table is the whole backlog at a glance — each row links to its section below. It tracks **only
completion**; findings, counts and closure notes live in each milestone's section and in
[`reviews/`](reviews/), so this table can't drift out of sync with them.

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
| [M5 — Integration & performance testing](#m5) | ✅ complete |
| [**M6 — Native Anthropic `/v1/messages`**](#m6) | ✅ **code-complete**, and **verified live**: a real Claude Code session round-trips through the proxy (2026-07-16) |
| [**M7 — NER latency**](#m7) | 🔨 **active** — the hybrid is ~1 s/KB, i.e. 20–40 s per Claude Code turn. **Now what gates `1.0.0`** |
| [First tagged release `1.0.0`](#m6) | ⬜ not started — gated on [M7](#m7): the product we advertise must be usable, not just correct |

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
  "which countries"** — and today it is a documented **no-op** (the FP-prone seam is wired and empty).

- [x] Locale-parametrized recognizer architecture — universal / national-ID / FP-prone tiers (`with_locales`, `fp_prone_recognizers`)
- [x] **National-ID packs for all XLM-R-aligned countries**, always-on, each checksum- or rule-gated: US SSN (keeps `PiiKind::Ssn`) · IT Codice Fiscale · GB NINO · ES DNI/NIE · FR NIR · DE Steuer-ID · NL BSN · PT NIF · LV personal code · zh Resident ID. **`ar` gets no pack** — the language spans ~20 countries with different ID schemes, so there is no single "Arabic" national ID; Arabic names/locations stay covered by the NER
- [x] **IBAN per-country length validation** — `Verified` only when mod-97 **and** the ISO 13616 length both hold; a wrong-length IBAN of a known country is still masked, flagged `Structural`
- [x] **Validate the NER across its declared domain** — scored XLM-R int8 on all **10** languages: Person 0.83 / Org 1.00 / Loc 0.91 (per-language notes in DEVLOG 2026-07-13)
- [x] Extend the corpus with multi-language cases (`multilingual_preview`) **and non-ASCII structured cases** (`non_ascii_scripts` — see M4-R13)
- [x] Provider-agnostic verification — `e2e_masking_is_provider_agnostic`: `openai` vs `anthropic` presets → a byte-identical masked body upstream
- **Locale phone national formats** → moved to Backlog (the FP-prone tier's first recognizer; the `+CC` arm already covers the unambiguous case)

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
      binaries as **throwaway artifacts** (30-day). No publish job, so it *cannot* cut a release. With push CI
      gone, this is **the way to confirm every target still compiles before you tag**.
    - **`ci.yml` is disabled** (renamed `ci.yml.disabled`) — a deliberate change of approach: nothing runs on
      push/PR anymore. **`cargo test` moved into `release-build.yml`** (so the suite still runs — on every
      target's native runner — but at `manual-build` / tag time, not per push; a release that fails its tests
      never publishes). **`fmt` / `clippy` / `msrv` are now local-only** gates (the CLAUDE.md "green before
      done" bar), so a lint break surfaces at build time or locally, not on a PR. *(Partly reversed later,
      when Dependabot was enabled: `security.yml` (cargo-deny) and a **trimmed `ci.yml`** (fmt/clippy/test on
      push + PR) were added so dependency-bump PRs self-verify. The all-target cross-compile stays tag/manual
      only. See DEVLOG 2026-07-15 / ARCHITECTURE → Supply-chain.)*
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
- [x] **Verification.** A native-schema coverage / fail-closed suite against a mock upstream (10 e2e +
  13 unit) — round-trip, tool defs (incl. a content-source document), fail-closed on an unknown block/document
  source, 404-when-not-anthropic, the full auth matrix, and the streaming split-placeholder. **Still open (the
  `1.0.0` gate):** the opt-in real Claude Code
  smoke — point a Claude Code session at the proxy and compare the M5 dual-run Run A / Run B against live
  Anthropic. Needs a human with Claude Code + credentials; not runnable in this environment.
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

- [ ] **Use more than 1 core of 12.** `with_intra_threads(1)` (`src/pii/onnx.rs`) is deliberate — a
  *pool* of single-threaded sessions optimizes **concurrent throughput** and is exactly wrong for
  **single-request latency**: ~18 chunks run sequentially, each on one core. Measure the real
  trade-off (latency vs throughput under concurrent load) rather than picking by intuition — both
  matter, and the current choice was made when a "field" was a sentence.

  > **The two knobs multiply — that is the trap.** `NER_POOL_SIZE × intra_threads` is the thread
  > count. Today it is `2 × 1 = 2`. Naively setting `intra = 12` while `pool = 2` gives **24 threads
  > on 12 cores** — oversubscription, and plausibly *slower* than now. The invariant is that the
  > **product** fits the box, not that either factor saturates it. So the default should be
  > **derived**, not fixed: `intra = max(1, available_parallelism() / NER_POOL_SIZE)`, overridable by
  > env like `NER_POOL_SIZE` already is. A fixed number is wrong on a 2-core VM and on a 64-core
  > server alike.
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
- [ ] **Stop paying twice for the fixpoint.** A no-PII 34.7 KB field (one pass) masks in 9.9 s
  = 286 ms/KB; a *smaller* 29.4 KB field with PII (two passes) takes 26.6 s = 903 ms/KB — **2.7×
  slower while 15% shorter**. M4-R21 accepted that cost when the NER saw sentences. The lead:
  passes ≥2 exist to catch masking **exposing** PII (a masked phone splits a digit run, revealing a
  card) — a **deterministic-recognizer** phenomenon. Masking a name to `[PERSON_1]` does not reveal
  a new name. So re-run only the structured layer on later passes. **Needs a real argument about NER
  recall at the seams before it ships** — `[PERSON_1]-Jones` is exactly the shape that would decide it.
- [ ] **Don't re-scan an unchanged system prompt every turn.** It is byte-identical across turns and
  dominates the payload. Any answer here (content-keyed cache of the *entities*, leaving the
  per-request vault to mint placeholders as it does now) adds state, and state on the masking path
  needs its own threat argument — which is why this is listed last, not first.
- [ ] **Rewrite the CC prompts as natural agent tasks** — a **verification debt from M6**, tracked
  here because M7 owns the re-run. The battery said *"reply with exactly this sentence: contact
  jane.doe@example.com, IBAN …"*, and the model **refused it as an injection attempt** — correctly.
  Claude Code is an *agent with a repo context*, not a completion endpoint, and it inherited that
  context precisely because the fixture lives **inside** the repo. So CC-01/CC-02/CC-08 never ran.
  > **The design question underneath, worth answering once:** *how do you test PII masking with a
  > client that refuses suspicious prompts?* The answer is not to argue the agent out of its
  > judgement (a `CLAUDE.md` telling it to comply would work and would be the wrong fix — it makes
  > the test a special case of itself). It is to use prompts that are **plausible work**: read this
  > file and summarise it, format this contact as JSON, run this query. Which is *also* what real
  > Claude Code traffic looks like — so the rewrite makes the battery both runnable **and** more
  > representative. CC-03/04/06/09 are already this shape; the chat-only ones are not.
- [ ] **Re-run the CC battery** once the prompts are fixed and the numbers move — 9 scenarios × 2
  runs is not practical at 27 s/turn, which is a second reason the latency gates the verification.
  Record the per-turn latency as a product figure next to the RAM ones in the READMEs.

> **Do not "fix" this with a build profile.** Measured: `release` (fat LTO, opt-level 3) buys **3%**
> — 27,728 ms → 26,863 ms on 29 KB. The cost is inside ONNX Runtime, a prebuilt native library that
> is already optimized; compiling *our* Rust harder changes nothing.

<a id="backlog"></a>
## Backlog — documented, not scheduled

### Evaluate GLiNER — contextual-PII detector / potential XLM-R successor
The path for **ambiguous, anchor-less PII** (a bare national phone, a free-form postal address) that the
deterministic layer can't disambiguate and the current XLM-R (PER/ORG/LOC only) doesn't cover. It is also
the clean **precision** path for [M4-R6](reviews/M4.md#m4-r6)'s accepted over-mask.

**GLiNER ≠ Piiranha.** Piiranha (a *fixed-label* mDeBERTa token-classifier) was **measured and rejected**
at M2 (~0 recall on natural sentences). GLiNER is a *zero-shot, **open-label*** span extractor — you pass
labels like `"phone number"` at inference and it matches by **context** — and is **not yet evaluated**. It
could be a single more-capable **successor** to XLM-R (named entities *and* contextual PII in one model).

Needs a **measured eval** (score `gliner_multi_pii-v1` int8 through the hybrid on the corpus; CPU latency /
RAM against the lean bar) and a **separate detector + span decode** (it is *not* token-classification, so
not a drop-in). Detail in [`M2-NER-EVALUATION.md`](M2-NER-EVALUATION.md).

> **Why it's backlog and not done:** GLiNER was set aside for *first-version scaffolding simplicity* — a
> separate detector was more integration than a first pass warranted. **Not** for any capability doubt.
> That is the **opposite** of Piiranha, which is a *measured* dead-end on prose. **When this is picked up,
> the evolution to evaluate is GLiNER, not Piiranha.**

### Locale phone national formats *(the FP-prone tier — moved out of M4)*
National phone numbers with no `+CC` anchor (UK `020 …`, DE `030 …`) collide with ordinary number
sequences, so they're false-positive-prone. The `fp_prone_recognizers(code)` seam is wired and empty, gated
by `PII_LOCALES`. The universal `+CC` arm already catches the unambiguous case. **The clean way to catch
the ambiguous form is *context*, not regex** — so this really belongs to the GLiNER escalation above.

### GPU optimization *(was M4 — deferred 2026-07-12)*
Faster inference once the model is locked. **Deferred on purpose:** the M2 model choice is
**execution-provider-agnostic** (standard ONNX, runs on any `ort` EP), so GPU constrains nothing upstream —
no reason to spend on it until real latency/load demands it. On this Windows/no-admin box the natural EP is
**DirectML** (any DX12 GPU, no CUDA/admin); going to GPU is mostly a config change (swap the EP; int8 →
the pre-shipped `model_fp16.onnx`).
- [ ] GPU execution provider (CUDA / DirectML) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline

### Option B — native provider adapters
The heavy alternative to M3's Option A: support each provider's **native** API (e.g. Anthropic's
`POST /v1/messages`) instead of its OpenAI-compat endpoint. Needs a **per-provider, schema-aware masking
adapter** (Anthropic uses `system`, content blocks, `tool_use`/`tool_result`, `tools[].input_schema`), plus
native auth and paths. **Higher leak risk — a missed schema field is a leak** — so it stays unscheduled
until a concrete need outweighs the OpenAI-compat path. **The Anthropic slice now has that need and is
promoted to [M6](#m6)** (Claude Code passthrough); native adapters for *other* providers remain here.

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

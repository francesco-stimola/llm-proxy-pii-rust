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

**M0 – M4 complete, and M4's ledger is clean — all 24 findings closed.** **M5 (integration &
performance testing) is code-complete** — real integration tests, a performance/load harness,
NER chunking (a real bug found and fixed along the way), the README rewrite, and CI/release
workflows are all in place. **Review round 1's 6 findings are all closed; round 2 verified those
closures and opened 3 more** ([M5-R7…R9](#m5-ledger)) — the M5-R2 fix checked the token bound correctly
but made the overflow *clamp and continue*, which returns `Ok` where `NER_REQUIRED` promises a block.
**One box is also deliberately still open**: the
manual dual-run procedure needs a human with a real `ANTHROPIC_API_KEY` to actually execute it, which
no session can do on its own — see the M5 section below. 132 tests green (default) / 145 + 6
`#[ignore]`d (`--features onnx`), no warnings; `cargo fmt` + `clippy` clean on both feature sets.
**MSRV 1.86** (default) / **1.89** (`--features onnx`) — *measured*, and now built by CI.
`v0.4.0`, AGPL-3.0-or-later. The product masks structured PII (universal + national-ID packs for
10 countries) and unstructured entities (ONNX NER, XLM-R int8, now chunked for large fields),
streams, and fronts OpenAI / Copilot / Anthropic via their OpenAI-compatible endpoints.

### Open work — everything not yet done

| What | Where | Status |
|---|---|---|
| **M5 review round 2** — 3 findings open | [ledger](#m5-ledger) | [ ] [M5-R7](reviews/M5.md#m5-r7) fail-closed · [M5-R8](reviews/M5.md#m5-r8) docs · [M5-R9](reviews/M5.md#m5-r9) guard |
| **M5** — integration & performance testing | [below](#m5) | [~] one box open — the manual live-provider check needs a human + a real key |
| **First tagged release (`1.0.0`)** | [below](#m5) | [ ] gated on a real green CI run (the workflows have never actually run) |

**M5 is unblocked.** Its prerequisite was a clean M4 ledger, and the last blocker was the DoS class — which
took **two** fixes, not one, because the masking path has **two** size axes: field *size*
([M4-R19](reviews/M4.md#m4-r19), the O(n²) candidate rescan) and entity *count*
([M4-R24](reviews/M4.md#m4-r24), the O(n²) mask splice). Both are now linear and guarded — DOS-01…03 vary
the size, **DOS-04 varies the count** — so a perf harness now measures the product, not a bug. Measured:
the 13.4 MiB many-entity body that took ~7 min of CPU masks in **1.8 s**.

> **The perf item found while closing M4-R24 — measured, and it needed a fix, not just a number.**
> The `onnx` NER fed the **whole field as one sequence**; the real failure mode was not the suspected
> quadratic self-attention slowdown but something more abrupt: past the model's `max_position_embeddings`
> (514), the ONNX graph's position-embedding lookup went **out of range** — an outright `Expand` op error
> on any field over ~500 tokens (~2 KB of prose), silently downgrading NER by default or **blocking every
> such request** under `NER_REQUIRED`. Fixed with overlapping-window chunking; measured **linear** afterward
> (448 ms / 2.07 s / 7.53 s at 64×/256×/1024× a repeated sentence). Detail in
> [`ARCHITECTURE.md`](ARCHITECTURE.md) → *NER chunking*.

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
> real floors — **1.86** default, **1.89** onnx — are measured and now **built** by CI.)*

- [x] **Real integration tests** — full mask → forward → (stream) → de-mask round-trips, tool-call
  round-trips, multi-turn determinism, and the fail-closed paths. **Mock upstreams cover all three preset
  shapes** (OpenAI / Copilot / Anthropic) — no accounts needed. The **real-provider smoke is
  Anthropic-only** (the only provider we have; opt-in, needs a key, never in CI without one) —
  `tests/anthropic_smoke.rs`, written and gated, **not yet run against a live key** (no credentials in this
  environment).
  - [x] Implement the two cataloged-but-missing e2e cases **E2E-02** (CSV `tool_result`) and **E2E-04**
    (`SELECT … FROM DUAL`) against a mock.
- [ ] **Manual "does the whole structure hold?" procedure** (real Anthropic + `RUST_LOG=…=trace`). The
  procedure itself is written — `docs/MANUAL_VERIFICATION.md` — but **running it needs a human with a real
  `ANTHROPIC_API_KEY`**, so this box stays open until someone actually executes it. Run the same PII prompt
  twice:
  - **Run A** (`PII_DEBUG_SKIP_DEMASK=1`) — the client gets the **placeholders** → proof the request left
    masked and the round-trip is wired.
  - **Run B** (normal) — the client gets the **restored values** → proof of the full round-trip.

  Comparing A vs B on the same input shows the chain holds end-to-end against a real provider. Trace
  logging also exercises the never-log-raw-PII rule (DBG-02) on **real** data.
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
- [x] **Update the root `README.md`** (+ `README.it.md`) to describe the shipped product — what it does,
  three-tier detection + NER, streaming, multi-provider usage, config/env, status. Rewritten from the
  "early development" placeholder to describe the working system (feature list, quick start, full env-var
  reference, current status).
- [x] **CI + release binaries (GitHub Actions).** `.github/workflows/ci.yml`: `fmt` (once) + `clippy` +
  `cargo test`, one job for the default build and one for `--features onnx` (matrix) — on push to `main`
  and on every PR. **All three are already green on both feature sets** (2026-07-13) — `fmt` and `clippy`
  had never been run, and `clippy` was in fact *erroring*, so the CI would have failed on its first run;
  that is cleared, and the MSRV is declared ([M4-R22](reviews/M4.md#m4-r22)). `.github/workflows/release.yml`:
  on a `v*.*.*` tag, cross-compiles the full `--features onnx` product for Linux (x86_64), macOS (x86_64 +
  arm64), and Windows (x86_64-msvc), and attaches the binaries to a GitHub Release. **Not yet exercised
  live** — no tag has been pushed and no PR has run the new CI yet; both are standard, unremarkable
  GitHub Actions shapes but are only proven once a real push/PR/tag runs them. **The first tagged release
  is `1.0.0`** (bump from `0.4.0`) — cut when a real CI run is green and, ideally, the manual verification
  procedure below has actually been run once against a live provider.

<a id="m5-ledger"></a>
### Review ledger — M5 → [`reviews/M5.md`](reviews/M5.md)
**Round 1 (2026-07-14): 6 findings — all closed. No leak, no fail-open, no over-mask regression.** The
chunking fix was verified against the **pre-fix** commit (the `Expand` error reproduces) and driven
through the real binary end-to-end with NER on an oversized field.

**M5-R5 turned out to be the sharp one.** Chasing it revealed the declared MSRV was **fiction**: `1.82`
cannot even *parse* the dependency tree. Measured, the real floors are **1.86** (default) and **1.89**
(`--features onnx`) — and they *differ per feature set*, which a single `rust-version` cannot express. Both
are now pinned in CI. This is exactly the failure M4-R22 tried to prevent and structurally could not:
`rust-version` makes cargo refuse a too-**old** toolchain; it cannot notice the crate drifting **past** its
own declared floor. **Only a job that builds on the MSRV can.**

| ID | Title | Sev | Status |
|---|---|---|---|
| [M5-R1](reviews/M5.md#m5-r1) | TESTING.md still says the NER is unchunked and quadratic — the claim M5 disproved *and* fixed | docs | [x] |
| [M5-R2](reviews/M5.md#m5-r2) | `MAX_SEQUENCE_TOKENS` didn't bound what reaches the model — chunks re-tokenize to 481–483 vs a usable limit of 512 → the ceiling is now **enforced** at the one choke point, and the drift is **asserted** | hardening | [x] |
| [M5-R3](reviews/M5.md#m5-r3) | The chunk slice hard-indexed tokenizer offsets — the one place on the masking path that could panic on attacker input | hardening | [x] |
| [M5-R4](reviews/M5.md#m5-r4) | The fixpoint's convergence proof covers the recognizers, not the NER — placeholder inertness is **empirical** for the model, and a **model swap must re-check it** (GLiNER especially) | invariant | [x] |
| [M5-R5](reviews/M5.md#m5-r5) | CI never exercised the declared MSRV → **and the declared MSRV was wrong**: measured 1.86 (default) / 1.89 (onnx), both now built in CI | low → med | [x] |
| [M5-R6](reviews/M5.md#m5-r6) | The READMEs' Status was self-referentially stale and understated the `1.0.0` gate | docs | [x] |

**Round 2 (2026-07-14) — closure verification: 5 of 6 hold; M5-R2's *valve* does not.** Re-measured both
MSRV floors + both negative controls, re-ran the four live NER guards (they reproduce the 481–483 drift),
confirmed `run_and_decode` really is the single session choke point and that clamping cannot corrupt a
span — then found that the clamp returns `Ok`, which is precisely what `NER_REQUIRED` exists not to get.

| ID | Title | Sev | Status |
|---|---|---|---|
| [M5-R7](reviews/M5.md#m5-r7) | The clamp trades `NER_REQUIRED`'s fail-closed block for a silent partial scan — the M5-R2 fix *relocated* the failure | fail-closed | [ ] |
| [M5-R8](reviews/M5.md#m5-r8) | ARCHITECTURE's *NER chunking* still names the constant M5-R2 deleted — in the file M5-R1 just made the single home | docs | [ ] |
| [M5-R9](reviews/M5.md#m5-r9) | The M5-R2 guard hand-copies the one constant `chunk_char_ranges` was made `pub` to avoid hand-copying | guard | [ ] |

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
until a concrete need outweighs the OpenAI-compat path.

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

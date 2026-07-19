# Development log

Newest first. One entry per meaningful change — note *what* and *why*, not just
*what*. This is the running history so context is never lost between sessions.

## 2026-07-19 — M8 GLiNER implemented, measured, and shipped **opt-in** (not a successor on int8)

**Built the whole M8 slice and validated it end-to-end against the real model**
(`onnx-community/gliner_multi_pii-v1`, int8 `model_quantized.onnx`, 349 MB, pinned). New:
`src/pii/gliner_decode.rs` (model-independent: the regex word splitter, the span decode, the
label→`PiiKind` map — 12 unit tests), `src/pii/gliner.rs` (`GLiNerDetector`: prompt + tensor build,
the session pool, word-window chunking — 5 unit tests), the `load_gliner` opt-in wiring in
`server.rs`, and `tests/gliner_eval.rs` (smoke + eval + inertness canary, `#[ignore]`d, gated on
`GLINER_MODEL_PATH`). 132 onnx / 108 default lib tests green, `clippy` clean on both feature sets.

**S0 — the ONNX I/O contract, verified from the real export** (not guessed). GLiNER span-mode
`markerV0`: inputs `input_ids` / `attention_mask` / `words_mask` (i64 `[1,L]`), `text_lengths`
(i64 `[1,1]`), `span_idx` (i64 `[1,S,2]`), `span_mask` (**bool** `[1,S]`); output `logits`
(f32 `[1, num_words, max_width, num_types]`). `S = num_words·max_width`, word-major, so the flat
logit index is `(word·max_width + width)·num_types + type` — the decode reads it directly.

**Two bugs the smoke test caught on the real model** (the reason S0 must be *run*, not reasoned):
1. **Word splitter.** GLiNER's `whitespace` splitter is not "split on spaces" — it is the regex
   `\w+(?:[-_]\w+)*|\S` (verified against the reference `gliner` lib), which **separates trailing
   punctuation**: `"Milano."` → `["Milano", "."]`. A pure-whitespace split kept `"Milano."` whole
   and measurably lowered scores. Fixed in `split_words`.
2. **Threshold.** The int8 model's confidences run low; correct entities cluster at **0.15–0.6**, so
   the nominal 0.5 missed most of them. Default set to **0.15** by a measured sweep (below).

**The measured decision (S2) — score through the hybrid on `ner_cases.json`, int8, threshold sweep:**

| threshold | Person R/P | Org R/P | Location R/P |
|---|---|---|---|
| 0.30 | 0.58 / 0.88 | 1.00 / 1.00 | 0.27 / 1.00 |
| 0.20 | 0.58 / 0.78 | 1.00 / 1.00 | 0.64 / 1.00 |
| **0.15** | **0.58 / 0.78** | **1.00 / 1.00** | **0.91 / 1.00** |
| 0.10 | 0.58 / 0.64 | 1.00 / 1.00 | 0.91 / 1.00 |

At the 0.15 optimum GLiNER **matches XLM-R on Location (0.91) and Organization (1.00)** but its
**Person recall is stuck at 0.583 at every threshold** — single-word / CJK / Arabic names (Tizio,
Caia, 张伟, محمد أحمد) it never scores, where XLM-R's floor is **0.83**. **So int8 GLiNER is *not* a
successor** — replacing XLM-R would regress name recall, i.e. more leaks, on the most important kind.
**Decision: ship it opt-in, off by default.** XLM-R stays the default NER; GLiNER is enabled via
`GLINER_MODEL_PATH` (+ `_TOKENIZER_PATH` / `_CONFIG_PATH`), adding what XLM-R *can't* do — contextual,
open-label kinds like a **bare national phone** (`"020 7946 0958"`, no `+CC`, the M8 recall gap) and a
free-form address. This is the "measure first, the milestone may say no [to successor]" gate doing its
job, exactly like Piiranha at M2 and the stop-at-the-bar call at M7.

**Quantization sweep (2026-07-19, follow-up) — is int8 the reason, or the model?** The low int8
confidences prompted the obvious question: does *less aggressive* quantization clear the successor bar?
Downloaded and scored all three variants through the hybrid on `ner_cases.json`:

| variant | size | Person R/P | Org | Location R/P | note |
|---|---|---|---|---|---|
| **int8** | 349 MB | 0.583 / 0.778 (@0.15) | 1.00 | 0.909 / 1.00 | the lean default; highest precision |
| **fp16** | 580 MB | **0.667** / 0.667 (@0.3) | 1.00 | 0.909 / 0.909 | +Caia, +Amsterdam; confidences run higher (Location already 0.909 at the nominal 0.5) |
| **fp32** | 1.16 GB | **0.667** / 0.667 | 1.00 | 0.909 | **identical to fp16** |

**Two conclusions.** (1) Less aggressive quantization *does* help — Person recall **0.58 → 0.67** and the
scores calibrate better — so int8 was hiding some of GLiNER's ability. (2) But **the verdict holds at full
precision**: even fp32's Person recall (0.667) is below XLM-R's 0.83. Quantization explains ~half the int8
gap; the rest is the **model** — single-word / CJK / Arabic names (Tizio, 张伟, محمد أحمد) it doesn't score
at *any* precision. The gain also costs ~0.11 precision (pronoun false positives — `She`/`I`/`me` → Person).
**fp32 ≡ fp16 because ORT up-casts fp16→fp32 on CPU**, so fp16 already delivers fp32 accuracy: **fp16 is the
higher-recall GLiNER option, and fp32 is pointless on CPU** (2× the RAM for the same result). int8 stays the
lean default; an operator wanting max GLiNER recall uses **fp16** (`GLINER_MODEL_PATH=…/model_fp16.onnx`,
~580 MB). "Not a successor" is now measured across the whole quantization spread, not just int8.

**The inertness canary — "GLiNER especially" (M5-R4) confirmed, and safe.** Run directly on
placeholder-dense text, GLiNER **does** tag our own `[PERSON_1]` / `[ORG_1]` tokens as entities (int8
XLM-R does not — that is why the docs singled GLiNER out). It is safe regardless: `keep_maskable`
drops an *exact* `[KIND_N]` hit by construction (CC-08) and **S4** keeps GLiNER off every pass after
the first, so `mask_all` on placeholder-dense input **converges with the text unchanged** (verified —
`gliner_placeholder_inertness_canary`), never a fixpoint 400. `redetect → empty` (idempotent after
pass 0) carries to GLiNER for the same reason it does to XLM-R.

**Integration shape.** `GLiNerDetector` implements `PiiDetector`, so it drops into `CompositeDetector`
next to the structured recognizers and XLM-R; the overlap resolver dedups a GLiNER guess against a
deterministic match (checksum wins), and a GLiNER false positive is an over-mask, never a leak. GLiNER
maps `"phone number" → Phone` / `"address" → Location` deliberately (email stays with the deterministic
layer). `NER_REQUIRED` now means "≥1 ML detector (XLM-R and/or GLiNER) must load and run unwrapped";
`GLINER_LABELS` / `GLINER_THRESHOLD` / `GLINER_POOL_SIZE` / `GLINER_INTRA_THREADS` tune it. Explicit
local paths only for now (the airtight-privacy path); an `hf-hub` auto-download parity with the NER is
a documented future addition.

**Reviewer round 1 (2026-07-19) — 5 findings, none a leak; all closed. One of them was load-bearing.**
The reviewer independently reproduced the eval to the digit and confirmed no leak / no fail-open, then
flagged that the **chunking path had never run against the real model** (M8-R2) and lacked a `max_len`
choke-point guard (M8-R1). Acting on that pair exposed a real recall bug the pure-function unit test had
hidden: a window filled to the `max_len` budget makes the model return **all-low logits at seq ≈ 384** (a
313-word window scored *zero* on a clear name), and GLiNER int8's confidence **dilutes with context** well
before that (a name at a window's start keeps ≳0.2 while the window stays ≲100 text tokens, ~0.15 by ~130).
**Fix:** cap the window at `MAX_WINDOW_TEXT_TOKENS = 100` (far below the `max_len` budget), which bounds
the context every span is scored against — the dilution is a function of window *size*, not the entity's
position in it (M8-R7), so a small window scores an entity at any offset; an 8-word overlap keeps a
boundary-crossing entity whole; plus the M5-R7 choke-point guard as the hard safety net. Long-field recall stays weaker than short-field (a documented model property;
the default XLM-R covers long system prompts). The other three: the overlap invariant for a GLiNER `Phone`
(`is_structured` → union-merged, not "loses") promoted to ARCHITECTURE (M8-R3), a 12th decode test
(determinism, M8-R4), and `load_gliner` hardened to **fail loud** on partial config / a bad threshold
rather than silently disabling an opt-in feature (M8-R5). Full record: `docs/reviews/M8.md`.

## 2026-07-18 — M8/M9 promoted from Backlog; the M8 GLiNER implementation plan

**Post-`1.0.0` planning.** With `1.0.0` tagged and the ROADMAP's scheduled work all closed (the only open
checkboxes left were the two GPU-optimization backlog items), the next two directions are promoted to
numbered milestones: **M8 — GLiNER** (contextual / open-label PII) and **M9 — GPU optimization**. M9 keeps
its existing rationale (EP-agnostic model choice, DirectML on this box); M8 gets the plan below. The ROADMAP
M8 section carries the scope checkboxes — this entry carries the *how*.

### Why GLiNER is not a drop-in (the thing the plan is shaped around)

The current NER (`OnnxNerDetector`) is **token-classification**: `input_ids` + `attention_mask` → per-token
logits → argmax → BIO decode (`ner_decode`). GLiNER is a different architecture, and every stage below
exists because of one of these differences:
- **Open-label input.** The entity types are fed to the model *as text* — prepended to the input, framed by
  special entity/separator tokens — so the label set is chosen at inference (`"person"`, `"phone number"`,
  `"address"`, …), not baked into the head. This is the whole reason to want it: contextual, anchor-less PII
  (a bare national phone, a free-form address) the deterministic layer can't disambiguate.
- **Span output, not per-token.** It emits scores per *(span, label)* pair, decoded by threshold + greedy
  non-overlapping selection — a `gliner_decode` module, the span×label analogue of BIO `ner_decode`.
- **Word-level spans.** It scores spans over *words*, so the detector must map words → sub-word token ranges
  (via the tokenizer's word ids / offsets) — more bookkeeping than argmax-per-token.
- **The labels share the sequence budget.** Because the labels are prepended to *every* window, the usable
  text budget is `model_max − prefix − specials − drift`, so M5's chunking math is recomputed against a
  budget the labels eat into.

Artifacts: `onnx-community/gliner_multi_pii-v1` (base `urchade/gliner_multi-v2.1`, PII-tuned, 6 languages
incl. IT), int8 `model_quantized.onnx` (~349 MB) + `tokenizer.json` (mDeBERTa-v3 SentencePiece) + config,
**revision-pinned**. Community conversion → scoring against the corpus *is* the trust check.

### Stages

- **S0 — Verify the ONNX I/O contract from the real export.** Download the pinned model; inspect the
  graph's actual input/output names and shapes. The key unknown: does the export **enumerate spans inside
  the graph**, or expect `span_idx` / `span_mask` as inputs? GLiNER ONNX exports differ here, and the decode
  design forks on the answer. Deliverable: the documented contract. **Everything downstream is built against
  this, not a guess.**
- **S1 — `GLiNerDetector` (`src/pii/gliner.rs`) + `gliner_decode`.** The detector: tokenize with the label
  prefix, build the word→token map, enumerate candidate spans (or feed span inputs per S0), run the session,
  pull span logits. The decode (model-independent, unit-tested without a model like `ner_decode`):
  (span, label, score) → per-label threshold → greedy non-overlap → `Vec<PiiEntity>`. Behind `onnx`; slots
  into `CompositeDetector` behind the `PiiDetector` trait, so the pipeline is untouched. `GLINER_LABELS`
  (natural-language types) → `PiiKind` map; start mapping to **existing** kinds (phone number → `Phone`,
  address → `Location`) so placeholders and de-mask are unchanged — a new `PiiKind::Address` is a
  deliberate, eval-justified addition (it ripples through `label`/`from_label`/`priority`/`is_structured`).
- **S2 — The eval harness + the measured decision (the gate).** `tests/gliner_eval.rs` (`#[ignore]`,
  `--features onnx`) scores GLiNER int8 **through the hybrid resolver** on an extended `ner_cases.json`
  (add bare national phones, free-form addresses, single-word names) — P/R/F1 per type + CPU latency / RAM /
  size vs XLM-R int8. **Recall is metric #1.** The decision: **successor** (GLiNER replaces XLM-R in the
  composite — the clean win, one model; requires ≥ XLM-R's M4 floor Person 0.83 / Org 1.00 / Loc 0.91),
  **addition** (both NERs run — GLiNER only for the contextual kinds; 2× model RAM + latency, only if it
  underperforms XLM-R on PER/ORG/LOC but adds real contextual value), or **rejected** (doesn't clear the
  lean bar — a legitimate outcome, exactly as M7 stopped at its bar and Piiranha was rejected at M2).
  Numbers → DEVLOG. **Recommended: evaluate as a successor first.**
- **S3 — Chunking against the shared label-prefix budget.** Port M5's discipline — the compile-time headroom
  invariant (M5-R2/M5-R10), enforcement at the single choke point, the posture-is-the-caller's rule (M5-R7)
  — recomputed so the window budget subtracts the label prefix, which rides every window. Guard: every
  re-tokenized window **plus its prefix** stays under GLiNER's usable max.
- **S4 — Wire into load + `redetect` idempotence + the model-swap canaries.** Extend `load_onnx_ner`/`hf.rs`
  (or a parallel `load_gliner`) for the pinned repo. Fail closed under `NER_REQUIRED`, `FailOpen`-wrapped
  otherwise. Override `redetect` → empty **with the 0-loss recall measurement re-run for GLiNER** (the S4
  argument is per-model: masking a name never reveals a new one). **Re-run the `m5_r4` placeholder-inertness
  canary against the real GLiNER model** — inertness is enforced by construction since S4's `keep_maskable`,
  but the canary is how a *filter-leaning idempotent* model (the GLiNER case the docs flag, M5-R4 / M7-R23)
  is caught. Wire successor-vs-addition per S2.
- **S5 — Docs + builder→reviewer.** ARCHITECTURE (span decode, shared-budget chunking, label config),
  TESTING (eval + corpus cases + re-run canaries), READMEs (env + detection matrix), DEVLOG. Reviewer loop
  until clean.

### Invariants any NER swap must re-check (carried from the reviews)
- **Placeholder inertness (M5-R4 → the `m5_r4` canary).** Enforced by construction since S4, but the docs
  single out "GLiNER especially" — re-run the canary against the real model.
- **The fixpoint / `redetect` (S4).** GLiNER overrides `redetect` → empty on the same argument as XLM-R; the
  0-loss recall claim is per-model and must be re-measured.
- **Sequence-budget headroom (M5-R2/R10).** Recomputed for GLiNER's label-shared budget; a tokenizer swap
  re-opens the drift number.
- **Fail-closed posture (M5-R7).** Any GLiNER threshold may degrade its own recall but must never decide the
  caller's posture — errors flow through `try_detect` / `FailOpen` exactly as the NER's do.

### Cross-link to M9
If GLiNER int8's CPU latency misses the lean bar, that is the trigger to pull **M9 (GPU)** forward, not to
ship slow: the escalation path in `M2-NER-EVALUATION.md` is explicit that a GPU EP + `model_fp16.onnx` is
how a heavier model earns its place (on CPU, fp16 is up-cast to fp32 — no speedup, so fp16 only pays off on
GPU).

## 2026-07-18 — CC battery CLOSED on the S4 binary: the two 400s converge, zero leak

**The last box on the road to `1.0.0`.** Re-ran the CC battery through the proxy against real Anthropic on
the **S4 binary** (cache S3 on), with the owner driving Claude Code:
- **CC-05** (chat "what's the email?") — 400'd pre-S4; now **converges**, client gets the real email (OFF).
- **CC-09** (MCP SQL `SELECT *`) — 400'd pre-S4; now **converges**, all six real values restored (OFF).
- **CC-08** (the original 400 — reminder list ×30) — **both postures**: OFF restores the three real emails
  across all 30 lines; ON shows `[EMAIL_2/3/4]` (exactly what the provider saw). 30 restorations of values
  the model never held.

**Zero fixpoint 400 across the entire S4 run; DBG-02 = 0 on every value, every scenario.** S4 is validated
live, end-to-end: the fragmentation non-convergence that blocked three scenarios is gone. CC-03/04/06/07
were leak-clean pre-S4 and S4 only ever masks *more* (it runs the full NER on pass 0 unchanged; it drops the
NER only on *later* passes), and the de-mask code S4 doesn't touch, so their earlier OFF results carry.
CC-01/02 were already both postures. **The battery is closed; the privacy property held on every turn,
including the fail-closed blocks.** `1.0.0` is unblocked — all that's left is the mechanical PR → merge → tag.

## 2026-07-18 — S3: content-keyed detection cache (M7.1 complete)

**The other M7.1 lead, landed.** Claude Code re-sends 20–40 KB of **byte-identical** system prompt + tool
schemas every turn; detecting PII in it (the NER above all) dominates the masking latency. `CachingDetector`
(`src/pii/cache.rs`) wraps the composite and memoizes `try_detect` **keyed on the exact field bytes**, so
turn 2+ skips the scan. The per-request vault still mints the placeholders, so numbering is unchanged — the
cache stores *what/where*, never a mask.

**The threat argument the ROADMAP demanded, discharged.** A cache hit must never mask *less* than a fresh
scan. `try_detect` is a pure function of its input (stateless regex; NER inference on the input alone) and
the key is the *whole* input, so a hit returns exactly what a fresh scan would — it cannot mask less. Only
`Ok` results are cached (an error still fails closed); the cache is bounded (a dependency-free two-generation
map, ~`2 × PII_CACHE_ENTRIES` live entries, only fields 256 B–128 KiB, hot keys promoted on read); and
`redetect` (S4's later passes, on per-request masked text) is never cached. `PII_CACHE_ENTRIES` (default 16,
`0` disables) is the one knob. Default **on** — it is sound, and the latency win is the whole point.

Tested: 5 unit tests (hit==fresh, error-not-cached, redetect-uncached, small-skipped, bounded-LRU) + an e2e
(`proxy_e2e.rs::e2e_cache_on_a_repeated_large_field_still_masks_both_times`, a repeated PII-bearing large
field masks on both requests). 116 onnx lib tests green, clippy clean. **M7.1 is complete** (S3 + S4);
ARCHITECTURE has both invariants. Left before `1.0.0`: the CC battery re-run on the S4 binary, with the user.

## 2026-07-18 — the *real* non-convergence cause, found live: NER sub-word fragmentation (CC-05) → S4 is the fix

**The instrumentation paid off.** Running the CC battery's **Run OFF** half on the diagnostic binary, **CC-05
(a plain "what's the email?" turn) hit the fail-closed 400** — and this time the value-free diagnostic named
the cause: `per_pass=[[ORG 6, PER 2],[ORG 2],[PER 1],[PER 1]]`, `remaining=[ORG 2]`,
**`placeholder_tags_suppressed=0`**. So it is **not** placeholder re-tagging (the earlier hardening was valid
but not the cause) — it is the **NER tagging sub-word fragments** of dense product/org names. The masked
bodies show it plainly: `S[ORG_6]` (Slack → "lack" tagged, "S" left), `Git[ORG_4]` (GitHub), `[PERSON_2] Code`
(Claude). Each mask splits the word; the next pass re-tags the leftover; the Claude Code **system prompt** is
dense with these, so it needs **> `MAX_MASK_PASSES`** and 400s. This is [M7-R7](reviews/M7.md#m7-r7)'s
fragment over-mask — which M7-R7 called "a latency cost, not a correctness one." **Live CC-05 proves that
wrong:** past 4 passes it is a fail-closed availability failure on real Claude Code traffic. It never
reproduced synthetically before because no synthetic input carried a real system prompt's density.

**Investigated three fixes offline (production composite, real XLM-R):**
- **Word-boundary snap** (extend a fragment span to the whole word) — **rejected**: it makes convergence
  *worse* (8 passes vs 4 on the same dense text). Counterintuitive but measured.
- **Bump `MAX_MASK_PASSES`** — **rejected**: PLAIN passes **grow with the dense text**, 6 (5 KB) → 11 (15 KB)
  → 13 (30 KB), unbounded; no safe fixed value, and each pass is a full NER scan.
- **S4 (NER on pass 0 only, structured recognizers after)** — **converges in 1 pass at every size.** This is
  the M7.1 "stop paying twice for the fixpoint" lead, now promoted from optimisation to *the fix*.

**S4's recall risk — the argument M7-R7 demanded — is answered.** Dropping the NER after pass 0 could in
principle miss a name a later pass would expose at a seam (`[PERSON_1]-Jones`). Measured: **0** losses — S4
masks every expected entity PLAIN does across the labelled corpus (25 NER entities / 15 cases), and **0** raw
PII survives when real names/emails/IBAN are injected into fragmenting dense text. Masking only ever *reduces*
NER context, so later passes surface no genuinely-new names — only the fragments they created.

**Decision (owner):** record all of this in M7.1's S4 spec (done, ROADMAP), keep running the OFF battery on
the current binary to collect any further 400s, and do the S4 change as a proper builder→reviewer cycle. S4
now conditionally gates `1.0.0`: fail-closed never leaks, but a proxy that 400s a fraction of real turns is
not "usable". Investigation harness was throwaway (`tests/cc05_investigate.rs`, deleted); the numbers live
here and the regression tests ship with S4.

**Implemented the same day.** A `redetect` method on `PiiDetector` (default `try_detect`; `OnnxNerDetector`
overrides it to return nothing — idempotent after pass 0; `CompositeDetector`/`FailOpen` delegate).
`Vault::mask_all` runs the whole detector on pass 0 and `redetect` on every later pass *and the fixpoint
confirm*, so the NER's fragments can't chain. Verified live: `ner_perf.rs::
m7_s4_dense_org_names_converge_instead_of_400` — dense system-prompt text that 400'd now converges — plus
deterministic `Fragmenter` unit tests (FC-09, bug→fix). 111 onnx lib tests green, `clippy` clean. Invariant
in `ARCHITECTURE.md` (*Masking must run to a fixpoint*); S4 closed in M7.1. S3 (the cache) is next.

## 2026-07-18 — CC-08 resolved: placeholder inertness *by construction* + a value-free block diagnostic

**The finding, recapped.** CC-08 (a long reminder-list turn) returned a fail-closed **400** — masking
could not confirm a fixpoint in `MAX_MASK_PASSES=4`. The guard fired **correctly**: blocked before
forwarding, **zero leak**. But a 400 on ordinary work is a real *availability* defect, and the 400 carried
no *reason* — the failing content isn't logged (by design, fail-closed blocks before the forward-trace).

**Diagnosis: the prime suspect was wrong.** The suspected mechanism was the NER tagging one of our own
`[KIND_N]` placeholders, so each pass re-masks it and the text never shrinks (`anonymizer.rs:75-80`, the
latent path the code documents). Rebuilt an instrumented proxy and had the owner re-run CC-08: it
**converged** (fresh session, less accumulated context). Then reproduced offline against the **production
composite** (structured it+us + the real ONNX NER) over every CC-08-like shape — placeholder-dense fields
with two-digit indices and mixed kinds in Italian context, the raw `contacts.csv`, and a chunk-triggering
long field. **All converge in ≤1 pass; the official `m5_r4` inertness test passes.** Across the live re-run
and ~10 synthetic reconstructions, the 400 **never reproduced**. Conclusion: placeholders are **empirically
inert for this model** — the suspected cause is *not* what fired. The real trigger is content-specific to
that one session (an unseen system-prompt/context field, or deep structured nesting > 4 passes), which
fail-closed handles correctly by design.

**Resolution (owner's call: instrument + harden, don't chase an unpinnable trigger).**
- **Placeholder inertness is now enforced *by construction*, for every engine.** `Vault::mask_all` runs
  detection through a new `detect_maskable`, which **drops any detection that is exactly one of our own
  `[KIND_N]` tokens** (`is_placeholder_token`) before masking — a real value can never take that shape, so
  this never drops genuine PII. Every surviving detection is real PII, masking real PII strictly shrinks the
  raw text, so the fixpoint converges **regardless of the NER**. This upgrades M5-R4 from an *empirical
  model property* to an *algorithm property*; the `m5_r4` NER test stays as **belt-and-braces + a model-swap
  canary** (it still tells us *whether* a future model — e.g. GLiNER — leans on the filter).
- **The fail-closed branch now explains itself, value-free.** On non-convergence it logs the **per-pass kind
  tally** (shrinking = a deep nest that would clear with more passes; stalled = genuine), the **residue's
  kinds**, and `placeholder_tags_suppressed` (the canary). Kinds and counts only — never the text, which
  fail-closed never forwards or logs. This turns any *future* recurrence into a pinned cause, which is worth
  more than a synthetic guess at this one.
- **Tests:** `mask_all_converges_even_if_a_detector_tags_its_own_placeholders` (a `TagsPlaceholders`
  detector that re-tags every placeholder — the exact pathology — and `mask_all` still converges),
  `is_placeholder_token_matches_only_our_own_tokens` (the filter's boundary: our tokens incl. tolerant
  corruptions in, foreign `[TODO_1]` / partials / real PII out), and `kind_histogram_…is_value_free`.
  107 onnx lib tests green, `clippy` clean. Invariant promoted to `ARCHITECTURE.md` → *Masking must run to a
  fixpoint*; catalog in `TESTING.md` (FC-07, NER-INERT-01 reworded).

**Why not a live re-run to confirm?** The fix only *adds* a filter that can never make convergence worse,
and CC-08 already converged live before it. The privacy property is untouched; the change is covered by
tests + the reviewer. A live re-run would only re-show convergence.

## 2026-07-18 — CC battery live run: 8/9 leak-clean, CC-08 the one finding

**Manual verification, the M7 → `1.0.0` gate.** Ran a real Claude Code session through the proxy against
real Anthropic — hybrid, `NER_REQUIRED=1`, at the new **`pool=1` default** (exercised live throughout,
startup logs `pool_size=1 intra_threads=12`). Each turn verified from the trace: only placeholders
leave, and a DBG-02 grep of every fixture's raw values returns **0**. Across ~41 forwarded requests,
DBG-02 stayed 0 for every raw value and every PII pattern.

**Leak-clean (7):**
- **CC-01** (contacts.csv → JSON) — *both* postures. Run ON: client saw `[EMAIL_2]`/`[PERSON_4]`; Run
  OFF: the restored real values. (Index ≠ 1 is *correct*, not drift: the NER numbers entities across
  the whole body in encounter order, and the boilerplate's entities precede the user's — so the user's
  email is `[EMAIL_2]`, not `_1`. Confirmed by the owner running from a fresh `/clear`.)
- **CC-02** (release note thanking a Person at an Org in a City) — *both* postures; Mario Rossi / Acme /
  Milano → `[PERSON]/[ORG]/[LOCATION]`, all NER.
- **CC-03** (read the whole CSV) — every category masked: `[EMAIL]/[PERSON]/[IBAN]/[PHONE]/[SSN]`.
- **CC-04** (write the first email to a scratch file) — the file on disk holds **`[EMAIL_2]`**, not the
  real email: proof the client acted only on the placeholder.
- **CC-05** (asked point-blank "what's the email?") — the model answered **`[EMAIL_2]`**. It genuinely
  does not hold the real value; masking is not just in/out, the model itself operates on placeholders.
- **CC-06** (deploy-config.env) — the SECRET test the old ML-only proxy failed: all three keys
  (`sk-ant-…`, `sk-…`, `AKIA…`) → `[SECRET_1/2/3]`, plus email/phone. Zero secret-shaped tokens out.
- **CC-07** ("which IBAN is German?") — the model **cannot tell**, because the `DE` prefix was masked
  before it arrived: the privacy/utility trade working as designed. Also showed vault consistency — the
  Italian IBAN repeated in two rows got the **same** `[IBAN_1]` both times.

**CC-08 — a real finding (availability, not privacy).** The long-reminder-list scenario returned
**HTTP 400 "vault detector failed: masking did not reach a fixpoint in 4 passes"**. This is the
**fail-closed guard firing correctly** — the request was **blocked before forwarding** (no `forwarding`
trace line, zero leak), exactly the posture a privacy proxy must have when it cannot confirm a field is
clean (`anonymizer.rs::mask_all`, `MAX_MASK_PASSES=4`, M4-R20). But masking that *cannot converge* on an
ordinary task is a real defect: the code itself flagged this as a **latent path** ("no input has ever
been shown to need more than 2 passes"), and CC-08 is the first input to trigger it. Prime suspect
(`anonymizer.rs:75-80`): the NER tagging a placeholder as an entity in the pathological
repeated-placeholder context, so each pass re-masks it and the text never shrinks.
- **Not yet reproduced.** The failing content isn't logged (fail-closed blocks *before* the forward-log,
  and we never log raw PII). Three synthetic reproductions — the placeholder reminder-list, the raw CSV,
  and the CSV×3 — all **converged** (reached upstream). So the trigger is more specific than "repeated
  placeholders"; pinning it needs instrumentation.
- **Next:** add a *value-free* per-pass kind/count log to `mask_all` (kinds only, never values), re-run
  CC-08 to capture what stays detectable on pass 4, then fix (likely: protect existing placeholders from
  re-detection) + a regression test. Tracked in ROADMAP.

**CC-09 — leak-clean, after fixing a self-sabotaging fixture.** The original `customer-lookup.sql`
carried the PII as **literals in the query text** (`SELECT 'bob@test.com'… FROM DUAL`): to run it the
agent reads the file → the proxy masks the literals **on read** → the agent runs a query that already
says `[EMAIL_1]` → the result has nothing new to mask, and the `tool_result` path the scenario exists
for is never exercised. **Fix (2026-07-18):** put the PII in a **table**, not the text. A synthetic
`cc09_customers` (one row: email/phone/ssn/card/iban/secret) is created out-of-band by
`fixtures/cc09-setup.sql`, and the agent runs the **PII-free** `SELECT * FROM cc09_customers`
(`customer-lookup.sql` is now exactly that). The available SQL MCP servers pointed only at real
corporate Oracle DBs (we did **not** query real tables — real-PII risk), so the owner stood up a
throwaway **SQLite** DB for the `python-sql` server (absolute-path connection, so the setup session and
the CC session hit the same file). **Run ON, verified:** the client saw
`[EMAIL_2]/[PHONE_1]/[SSN_1]/[CARD_1]/[IBAN_1]/[SECRET_1]`, and DBG-02 on the outbound log returned
**0** for all six raw values and every PII pattern. So an **MCP tool result** — a path the proxy never
sees coming — is masked like any other. (The raw row does appear in Claude Code's *local* tool-output
pane; that is the MCP tool's local return, which never transits the proxy — only the re-send to the
model does, and that left masked.) TESTING / MANUAL_VERIFICATION / the two fixtures updated to match.

**Bottom line for `1.0.0`:** the privacy property held on every turn that ran, including the fail-closed
block and the MCP tool-result path. **8/9 leak-clean; CC-08's non-convergence resolved the same day** —
see the entry above (placeholder inertness by construction + a value-free block diagnostic).

## 2026-07-17 — `NER_POOL_SIZE` default flips 2 → 1 (the personal shape becomes the default)

**Source + docs.** `DEFAULT_POOL_SIZE` is now **1**. The dominant deployment is a personal proxy in
front of a single client (Claude Code, concurrency ≈ 1), and a single request only ever occupies one
session (S1a — the field walk holds `&mut Vault`, `infer_chunked` loops its windows, only then does
the session run), so a second pooled session buys a lone request **nothing** while holding a second
copy of the model. So the lean default is one session: the whole box for the in-flight request, and
**less RAM** (how much — measured — is its own paragraph below; the earlier "half" was arithmetic and
wrong). That is `CLAUDE.md`'s *low-RAM* bar applied to the case almost everyone runs. M7 had already
identified `(1, 12)` as the personal shape and documented it — it just left the
*default* on the pooled `(2, 6)`; this flips which of the two documented shapes an operator gets by
setting nothing.

**What it is NOT: a latency win.** `intra = cores` vs `cores/2` is inside this box's noise — the SMT
question (`1×6` vs `1×12`) is UNRESOLVED (M7-R2/S1), its sign flips run to run. Latency between the
two shapes is a wash. Anyone reading this flip as "~2× faster single request" is re-reading the noise
M7 spent three rounds learning not to.

**The cost, named (not papered over).** `pool=1` measured **−23% throughput** under concurrent load
— two independent measurements plus a mechanism: intra-op scaling is sublinear, so N sessions ×
cores/N threads aggregate better than one × cores (2026-07-16 entry below, PERF-M7-04). The flip is
scoped around exactly that: it targets the **personal** case, which has no concurrency to lose the
23% on, while the RAM it saves is real. A **centralizing** operator serving concurrent clients sets
`NER_POOL_SIZE=N` to reclaim the throughput — which is why the pool stays an override rather than the
default's job.

**RAM, measured — and the "half" was wrong.** The pre-flip docs said `pool=1` "halves the RAM",
reasoning `834 MB / 2 sessions ≈ 400 MB each`. Measured properly (idle resident, same debug build,
2026-07-17): **`pool=1` = 563 MB private (585 MB working set), `pool=2` = 834 MB** — and the `pool=2`
figure reproduces the README's prior 834 MB exactly, so the method matches. So it is **~290 MB of
shared base + ~270 MB per session** (`pool=N` ≈ 290 + N×270 MB) — *not* a clean doubling, because the
ONNX runtime and the first session's arenas don't duplicate. Dropping the second session saves
~270 MB (**about a third**, not half); `pool=6` is ~1.9 GB, not the ~2.5 GB the `834/2` split
projected. That `834/2 ≈ 400-per-session` arithmetic assumed **zero base** — the same class of error
this milestone keeps naming, a number never checked against what the product does. README /
ARCHITECTURE / the config knobs now carry the measured scaling; this is the number to quote.

**Semantics that moved with it (M7-R13 / M7.1).** Under `pool=2`, `intra` floored at 1 below 4 cores,
so the derived default *was* `PRE_M7_SHAPE (2,1)` and M7's ratio was 1.0 by construction — nothing to
deliver on a small box. Under `pool=1` the derivation is `intra = cores`, so that identity now holds
**only at 1 core**; from two cores up the default already adds threads a lone request can use. The
latency harness still skips its ratio guard below 4 cores, but for a **different** reason now — the
few-thread shapes there are too thread-poor to clear the 1.5× floor reliably (untested on a small
box), not because the ratio is 1.0 by construction. The unit test pinning the derivation is renamed
`the_default_gives_one_session_the_whole_box`; `bar_shapes` in `m7_latency.rs` now guards the default
`(1,12)` **and** the centralized `(2,6)`.

**Exercised live the same day, leak-clean in both postures.** The hybrid ran a real Claude Code
session through the proxy against real Anthropic. **Run OFF** (old default `pool=2`) on a
name/org/location turn: the client saw the **restored real values**, the outbound trace carried only
`[PERSON_1]/[ORG_1]/[LOCATION_1]`, and a DBG-02 grep for the three raw values returned **0 hits**.
**Run ON** (`PII_DEBUG_SKIP_DEMASK=1`, new default `pool=1` → `intra=12`) on a JSON-extraction turn:
the client saw the **placeholders** (`[PERSON_4]/[EMAIL_2]/[IBAN_1]`), the outbound body carried only
placeholders, and a pattern scan found **no** real email or IBAN. Both postures held independently.
The one thing still open for a *formal* CC-battery closure is the strict **same-prompt** OFF/ON
pairing (the runbook's "same round-trip, two halves" proof) — here the two runs used different
prompts.

Files: `src/pii/onnx.rs` (default + `thread_tests`), `tests/m7_latency.rs` (`bar_shapes`,
`MIN_CORES_FOR_A_MEANINGFUL_RATIO` rationale, S1 docs), `docs/ARCHITECTURE.md`, `docs/TESTING.md`,
`docs/MANUAL_VERIFICATION.md`, `README.md` + `README.it.md`.

## 2026-07-16 — The onnx build gets its own target dir (the clobber footgun, closed)

**Infra, no source change.** The default and `onnx` builds both wrote
`target/debug/llm-proxy-pii-rust.exe`, so *any* default-features command — `cargo test`, `cargo
clippy --all-targets`, a plain `cargo build` — silently replaced the hybrid binary with a
structured-only one. Not hypothetical: it is what made the first live M6 run test half the product
(entry below), and `MANUAL_VERIFICATION.md` had been carrying it as a **warning box** — i.e. as a
discipline to remember, which is the weakest kind of guard this repo accepts anywhere else.

**Why not just rename the binary** (the first instinct, and the ask): **Cargo cannot name a binary
per feature.** Features are additive and crate-wide; `required-features` gates *whether* a bin
builds, never what it is called; and "not onnx" is inexpressible. A second `[[bin]]` with
`required-features = ["onnx"]` *would* give a distinct name — but `cargo build --features onnx`
would still also build the ambiguous `llm-proxy-pii-rust.exe` (a wasted link, ~5 min under fat
LTO), so the trap binary survives next to the safe one and you must now *know which to point at*.
That relocates the confusion instead of removing it.

**So the split is by directory, not by name** — `.cargo/config.toml` aliases add `--features onnx
--target-dir target/onnx`:

| Path | Contains |
|---|---|
| `target/onnx/debug/llm-proxy-pii-rust.exe` | **the hybrid — always.** Only the aliases write here |
| `target/debug/llm-proxy-pii-rust.exe` | whatever the last default-features command left; structured-only *by convention* |

**Only the first row is a guarantee** — an explicit `cargo build --features onnx` still writes a
hybrid to `target/debug/`, and seven milestones of docs trained exactly that habit. The asymmetry
is fine because it runs in the safe direction: the *dangerous* case (structured-only masquerading
as the hybrid) is the one `NER_REQUIRED=1` makes fatal. The shipped artifact name is untouched.
`cargo build-onnx` / `run-onnx` / `test-onnx` / `clippy-onnx`; extra args append
(`cargo build-onnx --release`). Run them from the repo root — `--target-dir` is cwd-relative, so
from a subdirectory you get a stray `<subdir>/target/onnx/` that a root `cargo clean` won't find.

**Verified, not assumed** (this box exists because the last one wasn't): both suites run green
through the aliases — **85 lib tests default, 97 with `cargo test-onnx`**, zero warnings, `fmt`
clean, `clippy-onnx -- -D warnings` clean, and all five workflow YAMLs parse. `cargo build-onnx`
then `cargo build` left **both** binaries alive — 54.9 MB (links ONNX Runtime) vs 13.2 MB, the
default command finishing in 2.1 s without touching the hybrid. `cargo run-onnx` rebuilds and
launches `target/onnx/debug/`. And the two binaries are genuinely different builds, proved by the
backstop they each hit under `NER_REQUIRED=1`:

```text
target\debug\…       Error: NER_REQUIRED is set but this binary was built without the `onnx` feature
target\onnx\debug\…  Error: NER_REQUIRED is set but the NER is not configured (set NER_MODEL_PATH / …)
```

**`NER_REQUIRED=1` stays mandatory for the CC battery.** The aliases make the right build
automatic; the flag makes the wrong one fatal. Two guards, and the cheap one is not a reason to
drop the other — the flag is what *caught* this in the first place.

**What the split does NOT close — and the review caught me claiming otherwise.** The clobber was
**loud**: it destroyed the binary, so `NER_REQUIRED` turned it into a fatal error. A binary in its
own directory is never destroyed — it goes **stale**, and a stale hybrid loads the NER and prints
both green startup lines. `NER_REQUIRED` sees a *missing feature*, never *old code*. My first draft
dropped MANUAL_VERIFICATION's old imperative ("always rebuild immediately before starting") on the
grounds that the trap was now closed by construction — which is the M4-retrospective move exactly:
*the failure relocated, and the doc stopped warning about it*. Fixed structurally rather than by
restoring the imperative: the recipe now runs **`cargo run-onnx`**, so cargo rebuilds before
launching and staleness is impossible.

**Bonus, and not free:** two target dirs are two caches, so flipping between default and `onnx`
stops invalidating the ort/tokenizers/hf-hub tree each time (that round trip was a ~7.5 min
recompile). Paid in disk — `target/onnx/` measures 8.9 GiB of a 36.1 GiB `target/`.

**The pipelines were already correct and are unchanged** (checked, since the question was asked):
`release-build.yml` is `--features onnx` throughout, and its packaged artifact comes from
`--target <triple>` — its own directory — with the build running *after* the tests, so a
structured-only binary cannot ship. A comment now says why it doesn't use the aliases (rust-cache
would stop covering a nested target dir), so nobody "fixes" it later.

**`ci.yml`'s default leg stays — but my recorded reason for it was wrong**, which the reviewer
rightly called the more dangerous half. I wrote that it guards the native-dep-free invariant
(DEP-01, M2.5-R1). It does not: `tests/dependency_footprint.rs` shells `cargo tree` with **no
`--features` flag**, so it asserts the same thing in the onnx leg — DEP-01 needs no default leg at
all, and a future builder who notices that deletes the leg as redundant *and is being reasonable*.
What the leg actually guards is what nothing else in CI compiles: the default feature set linking
and passing green without the native stack, and `src/server.rs`'s `#[cfg(not(feature = "onnx"))]`
block — the **only** not-onnx path in the tree, and the source of the `NER_REQUIRED` backstop error
this entire change leans on. That reason now lives in `ci.yml` next to the leg, not only here.

Docs realigned to the onnx variant: both READMEs' quick start, `SETUP.md`,
`MANUAL_VERIFICATION.md` (the warning box now *describes the split* and its stale-binary limit),
`TESTING.md` → *Running* (it taught only `cargo test` while the READMEs taught the pair), and the
`ner_eval` / `ner_perf` header commands.

**Not promoted to ARCHITECTURE/TESTING, deliberately** (the reviewer's call, and it's the right
one). The durable rule here is *"a live verification must prove which build it ran"* — and
TESTING.md already carries it, next to the thing it governs: the hybrid-with-`NER_REQUIRED=1` bar
and CC-02, "the one that catches a half-running product". The build-dir split is the *ergonomic
implementation* of that rule, with no runtime surface — implementations belong in SETUP, so
ARCHITECTURE would only be noise.

**Stale workflow comments fixed while verifying the pipeline claims** (in scope, since the entry
above asserts the pipelines were checked): three files still said per-push CI no longer exists and
pointed at a `ci.yml.disabled` that isn't there — `ci.yml` was brought back trimmed when Dependabot
was enabled. The cross-compile really is tag/manual-only; *CI* isn't gone, so the comments now say
which half is true. `release-build.yml`'s `upload-artifacts` input is also documented honestly: no
caller passes it, and the caller its rationale cited (the retired per-push `ci.yml`) no longer
exists.

## 2026-07-16 — M7 built: S0 measured the plan wrong, S1 met the bar, S3/S4 deliberately not done

**The plan said start at S0 and re-measure because the headline number was suspect. It was — by
6×. And the *reasoning* was wrong in a more interesting way than the number.**

### S0 — the fixture is the experiment

`tests/m7_latency.rs`: one realistic Claude Code turn — 112 fields, 22,823 B (22.3 KiB) — as the walk actually
sees it (one big `system`, 10 medium `tools[].description`, 100 tiny `input_schema` descriptions,
one ~130 B user message holding all the PII). The shape is **asserted**, not hoped for: the first
draft came out at 13.5 KiB because I wrote 350-byte tool descriptions when the real ones are 1–4 KB,
and the guard rejected it. That guard is the whole point of the file.

**A realistic turn masks in ~4.2–4.7 s — not 27 s.** The 903 ms/KB headline came from a blob densely
packed with Italian names; a realistic turn runs at ~190–210 ms/KiB. *(The `ms/KB` there is the
pre-M7 entry's own unit, hand-computed from a live trace, and nothing records whether its `29.4 KB`
was ÷1024 or ÷1000 — so it stays as written. The M7 fixture's figures are KiB because the harness
divides by 1024 and prints it. Relabelling the old number would be asserting a unit nobody measured
— M7-R10.)* Per field, before any change (one sample — see the S1 note on this harness's noise):

| part | fields | bytes | ms | ms/KiB | passes |
|---|---|---|---|---|---|
| `system` | 1 | 6,151 | 1,881 | 313 | **2** |
| `tools[].description` | 10 | 9,482 | 1,157 | 125 | 10 |
| `input_schema` descriptions | 100 | 7,060 | 1,094 | 159 | 100 |
| user message | 1 | 130 | 46 | 362 | 2 |

### …and S0's *mechanism* was wrong, which matters more than its number

The plan asserts the boilerplate carries **~zero PII**, therefore costs **one** fixpoint pass —
and demotes the fixpoint lead on that basis. **False.** `m7_s0_what_the_ner_finds_in_boilerplate_that_has_no_pii`
prints exactly one hit in text that contains no PII by construction:

```text
system  →  [(Organization, "An")]
```

A **two-character fragment of "Anthropic's"**, tagged `Organization`. So the largest field in the
turn pays a second full NER scan — ~940 ms of the original 4.24 s. And a real Claude Code system
prompt names Anthropic and GitHub constantly, so this is the **normal** case. Two consequences:

- **S4 (skip the NER on later fixpoint passes) is *more* relevant than the plan concluded, not
  less.** It is not done anyway — see S2 — but the reason is the bar, not irrelevance.
- **It is also an over-mask**: `"Anthropic's"` becomes `"[ORG_1]thropic's"` in the system prompt
  the model receives. Not a leak (it fails toward masking) and squarely the accepted M4-R6 class,
  but it is *boilerplate corruption* nobody had seen, because nobody had run the NER over a real
  system prompt. **Logged as a finding for the review, not fixed here** — precision work is not
  M7's scope and would need its own recall argument.

### S1 — measured on both axes, and the numbers refuted the intuition (once, not twice)

`NER_INTRA_THREADS`, explicit-wins-else-derived (`max(1, available_parallelism() / NER_POOL_SIZE)`).
The two knobs multiply, so the **product** must fit the box; `0` is unset for **both** knobs, never
ONNX Runtime's "pick for me", which would put `pool × all-cores` threads on the machine. Both
resolve in one place — `onnx::resolve_pool_and_intra`, which the harness calls too, so it cannot
measure a config the server doesn't ship (M7-R1).

**Latency — one request, 22.3 KiB turn, 12 logical cores, best of 3:**

| pool | intra | ms | ms/KiB | vs pre-M7 |
|---|---|---|---|---|
| **2** | **1** | **~4,700** | 213 | 1.00× ← pre-M7 |
| 1 | 1 | ~4,500–6,000 | — | ~1× |
| 1 | 2 | 4,111 | 184 | 1.15× |
| 1 | 4 | 2,845 | 128 | 1.67× |
| 1 | 6 | 2,651 | 119 | 1.79× |
| 1 | 12 | 2,966 | 133 | 1.60× |
| **2** | **6** | **2,547** | 114 | **1.86×** ← the new default |
| 4 | 3 | 3,192 | 143 | 1.49× |

**Read that table with its noise, which is the whole of [M7-R2](reviews/M7.md#m7-r2).** The same
configuration drifts **~40% between runs** on this box (`1×12` measured at 2.1 / 2.5 / 3.0 s on
different days). The sweep now takes 3 reps and prints min/median/spread; the bar (PERF-M7-05) takes
the **minimum**, which is the closest thing to the interference-free cost. Believe a row only when a
mechanism backs it.

**Throughput — 4 concurrent turns:**

| pool | intra | turns/s |
|---|---|---|
| 2 | 1 | 0.288 ← pre-M7 |
| **2** | **6** | **0.609** ← the new default |
| 1 | 12 | 0.472 |
| 4 | 3 | **0.731** |

**What the measurement settled — and what it did not:**

1. **`pool=1` is *not* free.** I had written "it is not a trade at all". It is: **−23% throughput**
   (0.472 vs 0.609; the reviewer independently measured −21%). Intra-op scaling is sublinear, so 4
   sessions × 3 threads aggregate better than 1 × 12. The deployment-shape argument in ROADMAP → M7
   stands exactly as written — which is why the default is derived and overridable rather than
   repointed at the personal case. **This one is real: two independent measurements, and a
   mechanism.**
2. **Scaling is sublinear** — ~2×, never 12×. Replicates everywhere.
3. **The pool is inert at concurrency 1** — true, and believe it from the **code**, not the table:
   one request occupies one session (the walk holds `&mut Vault`; `infer_chunked` loops its
   windows), so `pool` cannot help it. `2×1` ≈ `1×1` in most runs and diverges in others — that is
   the box, not a mechanism.
4. **SMT — UNRESOLVED, and the first cut claimed otherwise.** I wrote *"12 logical threads beat 6
   (2.09 vs 2.46 s); hyperthreading helps this int8 model"*, and made it the write-up's high point:
   *both* "measure, don't reason" items resolved against intuition. **It was one sample.** Re-run,
   the sign flips (`1×6` by 11%, then `1×12` by 8%, then `1×6` by 3%, then `1×6` by 11%) — an 18%
   effect read off a 40% band. It is load-bearing, too: `default_intra_threads` divides the
   **logical** core count, which is right only if SMT helps. That divisor is now an open question the
   milestone briefly believed it had closed.

**The derived default improves both axes over what shipped** — ~4.7 → ~2.5 s latency *and*
0.288 → 0.609 turns/s — so it is not a trade against the shared proxy at all. The single-client shape
(`NER_POOL_SIZE=1` → intra 12) gets **~2.1 s and half the RAM**, since each session holds its own
copy of the weights. **The RAM half is arithmetic; the latency half is inside the noise on this box** —
so the READMEs lead that recommendation with RAM.

> **The S1 mistake is the S0 mistake, one level up — and I made it inside the milestone that exists
> to name it.** S0 says: *a corpus has a shape, and the shape is the blind spot.* S1's blind spot
> wasn't the corpus, it was the **measurement design**: n=1 on a noisy box. Getting the fixture right
> does not save you from reading a conclusion off a single sample. That is why the reps and the
> spread column are now in the harness rather than in a reviewer's head.

### S2 — the bar was MISSED, and we stopped anyway (which is the stronger sentence)

**~4.7 s at the shipped default** — reproduced independently at **4,724** and **4,757 ms**, isolated,
on its balanced/energy-efficiency power plan. That is **~60% over the ~3 s bar**. The `2.46 s` this
entry once led with was the fastest of
seven observations and has never reproduced; it should not have been the headline, and the READMEs no
longer carry it.

**`S3` (content-keyed cache) and `S4` (skip the NER on later passes) are still not implemented, and the
reason changed.** It is no longer "the bar was met". It is: **what M7 could deliver, it delivered** — a
reproducible **~2×**, checkable on any box — and the rest of the gap is the machine, not the code. Both
leads put real risk on the masking path (state; lost detection), and that risk should be bought
deliberately when something demands it, not spent because we were already in here. The bar was declared
*before* the numbers so this decision couldn't be rationalised afterwards — and the discipline held in
the direction that actually costs something: **the bar came back missed, and the honest move was to say
so rather than to re-describe the number until it fit.**

**The bar guards both shapes** (M7-R1). The first cut asserted it on `pool=1` only — while `server.rs`
defaults to `pool=2` — so it had ~28% headroom on a configuration nobody runs and **none** on the one
they do. Both now resolve through the server's own function.

> **The absolute number here has an un-named variable bigger than every knob in S1 — and I named it
> wrong twice (M7-R9, then M7-R12).** The *same* code, fixture and box, across two people:
> **2,462 / 3,943 / 4,724 / 4,757 / 4,841 / 4,933 / 7,142 ms** — each run internally tight (spread
> under 7%). So min-of-3, my own fix for M7-R2's noise, was **precise and wrong**: it removes jitter,
> and this is not jitter. All reps sit inside the regime and agree confidently on the wrong number.
>
> **Then I explained the regime, and the explanation was fiction.** I wrote *"power and thermal
> regime, nothing else"*. The data refute it: a **battery** run (3,943) beat **three AC** runs
> (4,757 / 4,841 / 4,933). No power model orders that. And "throttled AC" — the third category I
> introduced — was assigned **post hoc from the number itself**; I never measured a thermal state.
> *A run was called slow because it was slow, then cited as evidence that slowness was throttling.*
> **That is the exact move this milestone exists to name**, committed in the entry naming it.
>
> **Then the machine's owner ended the argument in one sentence: the runs I called "AC" were on the
> *same energy-efficiency plan* as the ones I called "battery".** The charger was plugged in; the
> power profile never changed. So the variable was not mis-modelled — **it did not vary.** Every
> theory I built on it was theorising about a constant, which is why no amount of re-measuring could
> have rescued it, and why the "battery beat AC" contradiction was an artefact of the label rather
> than a fact about the box. **The person who owned the hardware knew in one line what four rounds of
> benchmarking could not tell me.** *When a variable you never controlled is doing the explaining, ask
> whoever controls it before you write the mechanism down.* What a **performance** plan does here
> remains unmeasured, and the published figure is the ordinary-laptop case deliberately.
>
> **The variable that is actually measured:** test **concurrency**. Cargo runs the perf tests in
> parallel, so the documented command measured the product **against four other copies of itself** —
> **1.50×** at constant power (4,757 isolated → 7,142 contended). `--test-threads=1` is now part of
> the contract, in the harness doc and in TESTING's recipe.
>
> **M7-R1 taught this milestone to name a number's shape; R9 named its power state; R12 showed the
> power state cannot order the data — and the box's owner showed it was never a state at all.** Four
> rounds, the same shape each time. *Naming one variable does not make a measurement reproducible —
> it makes the un-named ones harder to notice, because the number now looks qualified.*
>
> **So the assert is a ratio, and the ~3 s figure is a reported claim, not a guard.** The bar test
> measures the **pre-M7 shape** (`2×1`) as a calibration leg *in the same run*, seconds from the
> shapes under test, and asserts `pre_m7 / shape > 1.5`. The box's power state divides out:
> **~1.7 – 2.3×** across every one of the seven runs above, while the absolute moved **2.9×**. A
> ratio catches a real regression on any box and cannot go red because the box is slow — which the
> 3 s assert did on five of seven runs, while staying blind to a genuine 20% regression. *(The band
> is quoted loosely and the docs lead with the asserted **≥1.5× floor**, not a tight range: the ratio
> cancels power but not raw box speed, so a faster box compresses it toward the floor — 2.19×
> reference, 1.74× a quicker box — and every tight band we published got undercut by the next clean
> run; M7-R18.)*
>
> **What the ratio does NOT buy, said plainly because an honest guard states its blind spot
> (M7-R14):** at a 1.5 floor against a ~1.7 worst case, it tolerates a **~13% regression** — the same
> blindness the wall-clock bar had. It answers the **false positive**, not the false negative, and
> the floor cannot be tightened without false-firing against the observed spread.
>
> **And it has a domain (M7-R13):** below **4 cores** the derived default *is* `PRE_M7_SHAPE`, so the
> ratio is 1.0 by construction — the guard skips and says so, rather than reporting "a regression in
> the thread work" on a box where M7 simply has nothing to deliver. *The speedup scales with the box.*
>
> A **15 s** sanity ceiling remains for the order-of-magnitude case. It was 8 s and that was not loose
> at all: the reviewer's documented-command run hit a **median of 10,391 ms**, so the ceiling fired on
> the harness's own recipe — and blamed the power state for what was test concurrency.

**Where that leaves the milestone, stated without the flattering framing.**
- **The bar is missed.** ~4.7 s at the shipped default, ~60% over. It is missed at the *fixture's*
  22.3 KiB; real Claude Code turns run **20–40 KB**, so the top of the range is worse again (~8 s).
- **The ~2× is real**, reproducible on any box, and it is what M7 set out to buy: `with_intra_threads(1)`
  was leaving 11 of 12 cores idle on every request. That part is done and guarded.
- **The remaining gap is not a threads problem.** No arrangement of `pool × intra` closes a 4.7 s turn
  to 3 s on this hardware — the sweep's whole surface tops out around 2×, because intra-op scaling is
  sublinear and one request occupies one session.

So: **we missed the bar and stopped anyway**, because the next move is not more of this one. **S3 is
the named lead**, and the reason it is the right one is that it does not fight the box at all: the
boilerplate is byte-identical every turn, so a content-keyed cache makes turn 2+ nearly free
regardless of the machine, the power state, or the core count. It carries a real risk — **state on the
masking path** — and its threat argument is already written (S3, above). If the CC battery re-run (S6)
says the latency still bites in practice, that is where to go, and it should be bought on purpose.

### An asymmetry worth recording for whoever picks this up

At `intra=12`, the **100 tiny schema descriptions became the single biggest tier** — 909 ms of a
~2.2 s turn, for only 7 KB. They barely improved (1,094 → 909 ms, 1.2×) while the tool descriptions
improved 2.5×: a ~20-token sequence cannot use 12 threads, so it is nearly all per-call overhead
(~9 ms × 100). **Threads are done here; batching those calls is the next lead, not more threads.**

## 2026-07-16 — M7 implementation plan (design only — not built)

The technical blueprint for M7 (NER latency), to hand to the builder. **No source written.** Scope
and the measurements that opened it are in ROADMAP → M7 and the entry below.

### S0 — Fix the measurement first, because mine is probably wrong

**Start here, and do not skip it.** The entry below reports **903 ms/KB** and concludes the
fixpoint's second pass triples the cost. That number came from 29 KB **densely packed with names**
(`"Il cliente Mario Rossi di Acme SpA a Milano…"` ×450). **A real Claude Code request has the
opposite shape**: ~30 KB of boilerplate with **almost no PII**, plus a ~100-byte user message that
has it all.

And `Vault::mask_all` runs **per field**, not over the whole body. So on real traffic:

| field | size | entities | fixpoint passes | cost @ 286 ms/KB |
|---|---|---|---|---|
| `system` + tool schemas | ~30 KB | ~0 | **1** (detect → empty → return) | ~8.6 s |
| the user's message | ~100 B | 2–3 | 2 | negligible |

**Which changes the whole priority order.** A real turn is likely **~9 s, not 27 s**, and **lead 2
(the fixpoint) is worth almost nothing** — it only doubles fields that *contain* PII, and those are
tiny. Meanwhile **lead 3 (the cache) gets bigger**: ~8.6 s per turn spent re-scanning byte-identical
boilerplate, every turn, forever.

> **This is the M5 mistake, one level down, and it is mine.** The entry below scolds PERF-01 for
> measuring *a repeated synthetic sentence instead of a real payload* — and then measures *a dense
> synthetic blob instead of a real payload*. The corpus had a shape; the shape was a blind spot.
> **The fixture is the experiment.** Build it right before trusting a single number, including the
> ones I wrote down.

**Do:**
- Add a **realistic** payload fixture: ~30 KB of system prompt + ~10 tool `input_schema`s + a short
  user message carrying a few PII values. Sparse entities, exactly like real traffic. *(A captured
  real body is tempting — the trace log has one — but it is **masked**, so its NER pass finds
  nothing and the measurement lies in the other direction. Synthesize the shape, not the content.)*
- Re-measure per-field, not per-body: it is the field distribution that decides everything.
- **Then re-read the leads.** If a real turn is ~9 s and threads buy 3×, it is ~3 s and M7 may be
  done without touching the fixpoint or adding a cache.

### S1 — Lead 1: use more than one core (`src/pii/onnx.rs`)

`with_intra_threads(1)` + a pool of sessions optimizes **concurrent throughput**; we need
**single-request latency**. Replace the constant with a **derived, overridable** value:

```rust
// NER_INTRA_THREADS, else derive. The two knobs MULTIPLY: pool × intra is the thread count,
// and it must fit the machine — `intra = 12` with `pool = 2` puts 24 threads on 12 cores.
let intra = env("NER_INTRA_THREADS")
    .unwrap_or_else(|| max(1, available_parallelism() / pool_size));
```

**Measure both axes** (this is the trade-off, not a tuning knob):
- **latency**: one request, wall clock — the Claude Code case (concurrency ≈ 1);
- **throughput**: N concurrent requests, req/s — the shared-proxy case, which the current design
  was built for and which must not regress silently.

**Two things to measure rather than assume:** SMT (`available_parallelism()` = 12 logical = 6
physical × HT; dense math often prefers **6**), and sublinear scaling (expect ~3× from 6 threads,
less on an **int8** model whose kernels are memory-bandwidth-bound).

#### S1a — …and the derived formula is wrong at concurrency 1, because the pool does nothing there

**Read the code before believing the formula above — I didn't, and it divides by the wrong thing.**
A single request is sequential at **three nested levels**, and only the innermost is `intra_threads`:

| level | where | today |
|---|---|---|
| fields | `privacy.rs:92` — `mask` is a closure holding `&mut vault` | sequential **by construction** |
| chunks | `onnx.rs:219` — `infer_chunked`'s plain `for` loop | sequential |
| the model call | `onnx.rs:154` — `with_intra_threads(1)` | 1 thread |

So **one request uses exactly one core, whatever `NER_POOL_SIZE` is.** The pool buys *concurrent*
capacity: a second session only ever runs when a second request is in flight. `pool × intra` is
therefore the thread count **under saturated load**, not the count a single request can reach —
that one is `intra` alone.

Which makes `intra = available_parallelism() / pool_size` a **pessimization in the case M7 exists
for**: at the default `pool = 2` it yields `12 / 2 = 6`, and the Claude Code case (concurrency ≈ 1)
then runs 6 threads while **6 cores sit idle** waiting for a second request that never comes. The
divisor is right for the shared proxy and wrong for the personal one.

**Two ways out — and they are not equivalent:**
- **Chunk-level parallelism** (`infer_chunked` fans its ~18 *independent* chunks across the pool).
  Near-linear — chunks are embarrassingly parallel — where intra-op scaling is sublinear (~3×). It
  is also what makes the divisor honest: `pool = 2 × intra = 6` would then really be 12 threads on
  one request. **Costs RAM, and this is the trade:** each session holds its own copy of the weights
  — measured **~834 MB at `pool = 2`** (README), i.e. ~400 MB per session, so `pool = 6` ≈ **2.5 GB**
  against a *lean-RAM* bar. Latency bought in gigabytes.
- **`pool = 1, intra = all`** — free, no RAM change, and already the ROADMAP's recommendation for the
  personal case. It gets the sublinear ~3×, not the ~6×.

> **The boundary that decides which parallelism is even legal: parallelize *detection*, never
> *minting*.** Chunk fan-out is safe because chunks are read-only w.r.t. the `Vault` — they only
> detect, and `infer_chunked` already merges by a deterministic `sort` + `dedup`. The **field** walk
> is not safe to parallelize, and `&mut vault` is not merely why it's hard, it's why it's *wrong*:
> placeholder **numbering follows encounter order**, so racing two fields makes `[EMAIL_1]` vs
> `[EMAIL_2]` a coin flip and breaks the determinism M1 Part B pins. Whoever picks this up: the
> `&mut` is load-bearing, not an obstacle to route around.

**So S1's real question is not "what value for `intra_threads`" — it is "does a single request get
to use the box at all?"** Measure `pool=1, intra=6` and `pool=1, intra=12` first (free, no RAM),
and only reach for chunk fan-out if the sublinear ceiling isn't enough — with S2's bar, not to
exhaustion.

### S2 — The stop criterion, declared *before* the numbers

**If a realistic turn drops below ~3 s, stop.** Ship it, re-run the battery, tag. Do **not** do S3 or
S4. Adding state to the masking path or removing a detector pass are both real risks, and buying
them "because we were already in there" is how a privacy tool grows a leak. **Optimize to a bar, not
to exhaustion.**

### S3 — Lead 3: don't re-scan an unchanged system prompt *(only if S2 says so)*

The boilerplate is byte-identical every turn. A **content-keyed cache** (hash of the field text →
the `Vec<PiiEntity>` found) makes turn 2+ nearly free; the per-request `Vault` still mints the
placeholders, so determinism and the per-request round-trip are untouched.

**It needs its own threat argument before it ships**, because it puts **state on the masking path**:
- what is the key, and can two different texts collide into one entity set? (hash choice is a
  *security* decision here, not a perf one);
- bounded size + eviction — an unbounded cache on an **unauthenticated** path is the M4-R19 shape
  again, in memory instead of CPU;
- what happens on a cache **miss vs a wrong hit**? A miss costs time; a wrong hit **leaks**. Fail
  closed: on any doubt, re-scan.

### S4 — Lead 2: the fixpoint's second pass *(probably unnecessary — see S0)*

Only if S0's realistic numbers still show it matters. The idea: passes ≥2 exist to catch masking
**exposing** PII (a masked phone splits a digit run, revealing a card) — a **deterministic**
phenomenon. So re-run only the structured layer on later passes and skip the NER.

**The correctness argument must be made and tested first**, on paper and in the corpus: masking
`John Smith` inside `John Smith-Jones` yields `[PERSON_1]-Jones` — **does the NER then tag `Jones`,
and do we lose it if pass 2 skips the NER?** Decide it with a test, not an opinion. This is the one
lead that can **lose detection**; the other two cannot.

### S5 — Rewrite the CC prompts as natural agent tasks *(independent — do it anytime)*

No measurement blocks this. See ROADMAP → M7 for the design question (do **not** fix it with a
`CLAUDE.md` telling the agent to comply).

### S6 — Re-run the battery, then publish the numbers

CC-01…CC-09 × {OFF, ON} with `NER_REQUIRED=1`, and record per-turn latency next to the RAM figures
in both READMEs — measured, like the RAM ones, not claimed.

## 2026-07-16 — The live gate closed, then re-opened: the hybrid is unusable with Claude Code

The first real Claude Code session through the proxy **worked** — and then the same afternoon of
measurement turned the celebration into the most useful set of numbers this project has. Nothing
here is a leak; all of it is the kind of thing only a real client on a real provider can show.

**What held (measured, 2026-07-16).** A subscription-logged-in Claude Code, pointed at the proxy
with **no credential configured anywhere**, got **200 on the first request — no 401, no retry**.
It forwards its own credential; the proxy passed it verbatim while `UPSTREAM_API_KEY` was unset.
The request left masked (`contatta [EMAIL_1], IBAN [IBAN_1]`), the reply came back with the real
values restored, and a grep of the whole trace for the raw email and IBAN found **zero** — DBG-02
on real traffic, not the synthetic case `tests/log_safety.rs` pins.

> **The auth doc was backwards, and only the test could say so.** The previous
> `MANUAL_VERIFICATION.md` asserted that routing Claude Code through the proxy was impossible on
> **two** counts: schema *and* auth ("an OAuth token scoped to Claude Code's own flow… not a plain
> Bearer usable against the raw API"). M6 disproved the schema half by construction. The live run
> disproved the auth half by observation. Third-party write-ups still claim a custom
> `ANTHROPIC_BASE_URL` *requires* setting `ANTHROPIC_AUTH_TOKEN`; for 2.1.211 it does not.

**Then: the first run was testing half the product, silently.** No model was configured, so
`load_onnx_ner` returned `Ok(None)` and the proxy ran **structured-only** — the deliberate
fail-*open* posture for names. Email and IBAN masked perfectly *because they are deterministic
recognizers*, so everything looked green while **the NER never ran**; a `Person` would have gone
upstream in clear. `NER_REQUIRED=1` is now mandatory for the battery: it makes that silent
downgrade a fatal startup error. It proved itself within minutes — `cargo test` (default features)
overwrites the onnx binary at the **same path**, with no warning, and the flag caught exactly that.

**And with the NER actually on, the proxy is not usable with Claude Code.** Measured on this box
(Ryzen 5 PRO 8540U, 6 cores / 12 threads), timing the *masking alone* (a credential-less request
masks fully, then 401s — so the time to the 401 **is** the masking cost):

| what | 29 KB field | per KB |
|---|---|---|
| structured only | **20 ms** | ~0.7 ms |
| hybrid, debug | 27,728 ms | 956 ms |
| hybrid, **release** (fat LTO) | 26,863 ms | 926 ms |

- **The masking is linear** — ~0.96 s/KB, constant across 2 / 10 / 29 KB. M4's DoS guards hold;
  this is not an algorithmic bug.
- **The NER is ~100% of the cost**: 20 ms vs 27 s on the same 29 KB — a factor of **~1,400×**.
  The deterministic layer is free, exactly as the README claims.
- **Release buys 3%.** The cost is inside ONNX Runtime — a prebuilt, already-optimized native
  library. Compiling *our* Rust faster changes nothing. (Worth knowing before anyone "fixes" this
  with a profile flag.)
- **Claude Code sends 20–40 KB every turn** (its system prompt plus every tool's `input_schema`),
  which we re-scan from scratch each time → **20–40 s per message**.

**Why nobody saw it.** M5's PERF-01 measured the NER on a *repeated synthetic sentence* and
concluded "linear" — correctly. It never measured a **real client's payload**, which is dominated
by boilerplate re-sent every turn. "Linear" was true and beside the point: the constant is what
makes it unusable, and only a live client exposes the constant.

**Three leads, measured not guessed:**

1. **We use 1 core of 12.** `OnnxNerDetector::load` sets `with_intra_threads(1)` deliberately — the
   design holds a *pool* of single-threaded sessions, which optimizes **concurrent throughput** and
   is exactly wrong for **single-request latency**: an oversized field's ~18 chunks run one after
   another, each on one core.
2. **The fixpoint's second pass costs more than the first.** A 34.7 KB field with **no** PII (one
   detector pass) masks in **9,923 ms** = 286 ms/KB. A *smaller* 29.4 KB field **with** PII (two
   passes) takes **26,567 ms** = 903 ms/KB — **2.7× slower while being 15% shorter**. M4-R21
   accepted that "~2×" when the NER saw small fields; on a 30 KB system prompt it is ~13 s of
   re-scanning per turn. And the phenomenon the second pass exists for — masking *exposing* PII by
   splitting a digit run — is a **structured-recognizer** effect. Masking a name to `[PERSON_1]`
   does not reveal a new name. Re-running only the *deterministic* layer on passes ≥2 is the
   obvious lead; it needs a real argument about NER recall at the seams before it ships.
3. **The static prompt is re-scanned every turn.** The system prompt and tool schemas are identical
   across turns. Nothing exploits that today.

**Two flaws were in the test, not the product.** The battery's prompts said *"reply with exactly
this sentence: contact jane.doe@example.com, IBAN …"* — and the model **refused**, correctly reading
it as an injection attempt ("it has nothing to do with this repository"). Claude Code is an *agent
with a repo context*, not a completion endpoint, and it inherited that context precisely because we
moved the fixture **inside** the repo. The scenarios must be **natural agent tasks** (read this
file, format this contact), which is also what real usage looks like. Rewriting them is the next
step.

**Status: the `1.0.0` tag waits.** Not for a leak — for an honest answer to "is the product we
advertise actually usable?". Investigating leads 1–3 before tagging.

## 2026-07-16 — M6 review round 1 closed (5/5): the leak was a source *inside* a known block

Independent reviewer pass over the M6 landing (`0cdd251` + `98d9f55`). **Five findings, all closed.** Full
record: [`reviews/M6.md`](reviews/M6.md). Post-fix: **85** lib tests default / **97** with `--features onnx`,
**10** e2e; `fmt` + `clippy` clean on both.

**M6-R1 — the one real leak, and it hid one level down from where the guard looks.** The block-type dispatch
is strict (unknown *block* → 400), but inside the known `document` block the **`source.type`** dispatch was
fail-*open*: the first cut masked only a `text` source and skipped every other source type. Anthropic has a
**`content`** document source — `{type:document, source:{type:content, content:[{type:text, text:"…"}]}}` —
whose blocks carry plaintext PII, and the reviewer reproduced it through the real router: a raw email + IBAN
reached the mock upstream **in clear**. The fix mirrors the block-level rule one level down —
`mask_anthropic_document` dispatches `source.type` (`text`→data, `content`→recurse the nested array,
`base64`/`url`→skip, **unknown→fail closed**) and also masks the `title` / `context` metadata the first cut
skipped. **The lesson: "unknown → fail closed" has to hold at *every* dispatch, not just the outermost one —
a fail-open branch inside a known block is exactly as much a leak as an unmodelled block.** R2 is the same
class one level up (the `system` array skipped a no-`text` object open) — closed the same way.

**M6-R3/R4/R5 — the docs/test-quality trio.** R3: the test counts were wrong (a "96 lib default" that was
really the onnx count, "14 unit" that was 12) — corrected and restated in the "N default / M onnx" form the
M4 miscount lesson prescribes. R4: three e2e placeholder-*presence* asserts were satisfiable by the
augmentation prompt (which literally contains `[EMAIL_1]`/`[IBAN_1]` as examples) — not vacuous (the
`!contains(raw)` checks are the real guard), but weaker than they read; now they assert the **specific masked
field** or a token absent from the prompt (`[PHONE_1]` / `[EMAIL_6]`). R5: a "Prepend" comment that appends.

## 2026-07-16 — M6 built: native Anthropic `/v1/messages` (Claude Code passthrough)

Implemented M6 end to end on `feat/m6-anthropic-messages`, following the 7-stage plan below. The proxy now
accepts a **native** Anthropic Messages body, masks it **in place** (no OpenAI translation), forwards
native→native, and de-anonymizes both the buffered reply and the SSE stream. The masking engine
(`Vault::mask_all`, the fixpoint, `spawn_blocking` + fail-closed) carried over untouched — M6 is *only* the
Anthropic-schema walks plus the native forward/auth. **Green on both feature sets** (after review round 1:
**85** lib tests default / **97** with `--features onnx`, **10** new e2e; `fmt` + `clippy` clean).

**What landed, per stage:**
- **T0 — `WireSchema` tag** (`pipeline/mod.rs`): `enum WireSchema { OpenAi, Anthropic }` on `RequestContext`,
  defaulting to `OpenAi`, so the existing path is untouched. `PrivacyStage` and `SseDemasker` dispatch on it.
- **T1 — route + handler** (`server.rs`): `/v1/messages` registered **only when `provider == "anthropic"`**.
  The mask + fail-closed + streaming-detect flow is factored into `run_privacy_stages` / `finish_buffered` /
  `finish_streaming`, shared by both handlers — the two differ only in the schema tag, the forward, and the
  SSE schema.
- **T2 — request mask** (`pipeline/privacy.rs`, `mask_anthropic_request`): `system` (string / text-block
  array), `messages[].content` blocks dispatched on `type`, `tools[].description` + `input_schema`. Unknown
  block type → **fail closed, 400**, with a reason that carries **no** client-controlled value.
- **T3 — augmentation** into the top-level `system` (`inject_augmentation_anthropic`): absent → created;
  string → appended; block array → pushed as a trailing text block. Only when something was masked.
- **T4 — buffered demask** (`demask_anthropic_response`): top-level `content[]` — `text` and `tool_use.input`
  string leaves — mirroring T2.
- **T5 — SSE demask** (`stream.rs`): `SseDemasker` factored into the shared split-placeholder hold-back core
  + a per-schema rewriter. Anthropic `content_block_delta` (`text_delta` / `input_json_delta`), held back per
  block `index`.
- **T6 — native forward/auth** (`proxy.rs`, `send_messages` / `forward_messages` / `messages_auth`): path
  `/v1/messages` (`config.upstream_messages_path`, default `/v1/messages`); credential resolution; default
  `anthropic-version`.
- **T7 — tests:** 13 unit (`privacy.rs` ×9: mask coverage / document sources / fail-closed / augmentation /
  demask; `stream.rs` ×4: SSE split / `input_json_delta` / pre-stop flush / pass-through) + 10 e2e
  (`tests/anthropic_messages_e2e.rs`). *(Counts are post-review-round-1.)*

**Four design calls made while building, recorded honestly:**

- **Auth: client credential wins, proxy key is the fallback — a reconciliation.** The ROADMAP scope says
  *"inject the proxy's own key as `x-api-key` only when the client sent none"* (client wins); the terser
  DEVLOG-plan **T6** line wrote it the other way (*"proxy-key → `x-api-key`, else client `Authorization`"*).
  These conflict. I took the ROADMAP ordering — it matches the existing **chat-path** posture (ARCHITECTURE:
  *"the client's own `Authorization` wins, else the configured key"*) and the M5 note that the proxy *"forwards
  a client `Authorization` verbatim in preference to `UPSTREAM_API_KEY`"*, and it is the whole point of the
  feature (Claude Code's OAuth token must not be overridden by a configured key). Final order:
  client `Authorization` (verbatim) → client `x-api-key` → proxy key as `x-api-key` → **401**. An OAuth token
  only ever rides `Authorization`, never `x-api-key` (Anthropic 401).
- **`thinking` is masked on the way up but never de-masked on the way down.** A thinking block is generated by
  the model over already-masked input, so it naturally contains only placeholders, and its `signature` signs
  *that* placeholder text. Leaving the placeholders intact means the bytes never change across a multi-turn
  replay (re-masking an inert placeholder is a no-op), so the signature stays valid — robustly, even if
  placeholder *numbering* shifts elsewhere in the conversation. De-masking thinking would either break the
  signature or make correctness depend on reproducing identical numbering. So the request walk masks `thinking`
  (safety, in case a client injects fresh PII) and the response walk deliberately skips it. Promoted to
  ARCHITECTURE → *Native Anthropic Messages*.
- **A text-source `document` is masked, improving on the plan's "skip document".** The plan said skip `document`
  as non-text — but an Anthropic `{type:document, source:{type:text, data:"…"}}` carries **plaintext** that can
  hold PII, so skipping it would leak. The walk masks `source.data` when the source is text and skips the
  base64 file case. Never-leak beats the simplification.
- **The SSE demasker holds the `event:` line to fix frame ordering.** Anthropic frames each event as an
  `event:` line + a `data:` line. A block's held-back tail must flush **before** its `content_block_stop`
  frame (a `content_block_delta` after the stop is protocol-invalid). But the `event: content_block_stop` line
  arrives *first*, so flushing while processing the `data:` line would scramble the frame. Fix: the demasker
  holds each `event:` line until its `data:` line, and at a `content_block_stop` it flushes the block's tail
  ahead of the held `event:` line. OpenAI streams have no `event:` lines, so the mechanism is inert there. This
  is exactly the streaming-demask piece the prior proxy left as a TODO.

**One scope boundary:** **server-side result blocks** (`server_tool_use` / `web_search_tool_result`, sent only
when server-side tools are enabled — not the default Claude Code path) currently **fail closed** rather than
being modelled. This is safe (no leak) and a conscious future addition; the core client-side flow
(`text` / `tool_use` / `tool_result` / `thinking`) is covered.

**Still open — the `1.0.0` gate is unchanged:** the opt-in live verification (point a real Claude Code session
at the proxy, run the M5 dual-run against live Anthropic). It needs a human with Claude Code + credentials and
is not runnable in this environment. Everything testable without a live provider is done and green.

## 2026-07-15 — M6 implementation plan (design only — not built)

The technical blueprint for M6 (native Anthropic `/v1/messages`, Claude Code passthrough), to hand to the
builder. **No source written** — this is the plan; scope + the design decisions are pinned in ROADMAP → M6.

**Reuse, don't rebuild.** The masking engine is already schema-agnostic: `Vault::mask_all` (the M4-R17
fixpoint), the `spawn_blocking` + fail-closed handling in `server.rs`, the split-placeholder hold-back in
`stream.rs`, and `AUGMENTATION_PROMPT` all carry over untouched. What M6 adds is *only* the Anthropic-schema
walks (mask / demask / SSE) and the native forward/auth.

**The 7 stages, with files:**
- **T0 — schema tag.** `enum WireSchema { OpenAi, Anthropic }`; a field on `RequestContext` (`pipeline/mod.rs`);
  `PrivacyStage` dispatches on it. Zero impact on the OpenAI path.
- **T1 — route + handler** (`server.rs`). `/v1/messages` registered **only when `UPSTREAM_PROVIDER=anthropic`**;
  a `messages` handler sharing the mask + fail-closed + streaming-detect flow with `chat_completions`
  (factored), differing only in schema, forward, and SSE demasker.
- **T2 — request mask** (`pipeline/privacy.rs`, `mask_anthropic_request`). `system` (in place),
  `messages[].content` blocks dispatched on `type` (`text` / `tool_use.input` object leaves /
  `tool_result.content` recursive / `thinking`), `tools[].description` + `input_schema`. **Unknown block-type
  → fail closed, 400**; the known set is exhaustive for Claude Code and pinned by a guard.
- **T3 — augmentation** into the top-level `system` (string or block array), only if something was masked.
- **T4 — buffered response demask** (`demask_anthropic_response`): top-level `content[]` (`text` +
  `tool_use.input`), mirroring T2.
- **T5 — SSE demask** (`stream.rs`): factor `SseDemasker` into the shared split-placeholder core + a per-schema
  rewriter; handle Anthropic `content_block_delta` (`text_delta` / `input_json_delta`), held back per block
  `index`. *The privacy-critical piece the prior proxy punted.*
- **T6 — native forward/auth** (`proxy.rs`, `send_messages`): path `/v1/messages`; proxy-key → `x-api-key`,
  else client `Authorization: Bearer` (OAuth) verbatim, else client `x-api-key`, else 401; never OAuth in
  `x-api-key`; forward `anthropic-version` (default `2023-06-01`) + `anthropic-beta`.
- **T7 — tests** (adversarial-first): native coverage / fail-closed, buffered + SSE demask, e2e mock native
  upstream, opt-in Claude Code smoke, and the M5 manual dual-run finally runnable via Claude Code → the proxy.

**Delivery:** one branch → PR (`feat/m6-anthropic-messages`), streaming included, gated by the per-PR CI.

## 2026-07-15 — reqwest 0.12→0.13 (TLS backend pinned) + tower-http 0.6→0.7

Took the two Dependabot cargo bumps deliberately, by hand, rather than merging the auto-PRs — because
reqwest 0.13 hid a trap.

- **tower-http 0.6 → 0.7**: a clean bump. No `http`/`hyper`/`tower` major change, and the only thing this
  crate uses — `TraceLayer::new_for_http()` — is unchanged. Nothing to do but the version.
- **reqwest 0.12 → 0.13**: the source APIs this crate uses (`Client`, `.json()`, `.bytes_stream()`,
  `.headers()`, `header::*`) are all unchanged — but reqwest 0.13 **silently flipped its default TLS backend**
  from native-tls to **rustls + aws-lc-rs**. Dependabot's PR kept the default features, so it would have
  pulled `aws-lc-rs` into the **default** build — breaking the native-dep-free guarantee that
  `tests/dependency_footprint.rs` (M2.5-R1) exists to hold, and adding a cmake/NASM build requirement on the
  Windows/arm64 release. So the bump **pins the TLS backend explicitly**:
  `default-features = false, features = ["native-tls", "json", "stream", "http2", "charset", "system-proxy"]`
  — same runtime behaviour as 0.12, native-dep-free default preserved, and the crypto backend is now a
  deliberate choice rather than a dependency's shifting default (the lesson is recorded next to the dep in
  `Cargo.toml`).

Verified locally on the default build: `cargo build`, the full default `cargo test` (incl. the footprint
guard — green, no `aws-lc-rs`), and `cargo clippy -D warnings` all pass. reqwest deduplicates to a single
`0.13.4` (the onnx/hf-hub stack already used it). One benign new duplicate: reqwest 0.13 is now built on
tower, so it pulls its own `tower-http 0.6.11` alongside our `0.7.0` — cargo-deny only **warns** on that
(`multiple-versions = "warn"`). The onnx leg / MSRV / fmt are left to CI (`ci.yml`), which now gates this
through a PR. Landed on a branch so the gate runs before merge.

## 2026-07-15 — CI reinstated (trimmed) as a per-PR gate; Dependabot tuned down

Enabling Dependabot (below) exposed a gap: version-update PRs had **no automated build/test**. The
tag-driven pipeline only builds at tag/manual time, and cargo-deny / CodeQL check vulnerabilities and
patterns — not "does it still compile". So a bump like reqwest 0.12→0.13 (a *breaking* 0.x bump) could read
"Ready to merge" in the UI yet break `main` silently. Fixed on two sides:

- **`ci.yml` brought back, trimmed** (was `ci.yml.disabled`). On push-to-main + PR: `fmt`, `clippy` + `test`
  on both feature sets (`default` and `--features onnx`), and an MSRV `cargo check`. It **drops** the
  all-targets release build — that stays in `manual-build.yml` / the tag release (`release-build.yml`). So
  every PR, Dependabot's especially, is now self-verifying. Ships with `permissions: contents: read` (no
  repeat of the missing-workflow-permissions alert).
  - The MSRV leg **reads the floor from `Cargo.toml`** (`cargo metadata --no-deps | jq .rust_version`)
    instead of hardcoding it, so CI and the manifest can't drift — the M5-R5 lesson, applied.
- **`dependabot.yml` tuned down** — `weekly`→`monthly`; all non-major cargo updates **grouped** into one PR;
  true `semver-major` (1.x→2.x) bumps **ignored** (done by hand). Caveat recorded in the file: Dependabot
  classifies 0.x breaking bumps (reqwest 0.12→0.13) as *minor*, so `ignore semver-major` does **not** suppress
  them — `ci.yml` is what makes those safe.

**This partly reverses the same-day pipeline change** (which had moved fmt/clippy/test off push/PR). The
reversal is deliberate and narrow: the tag-driven *release* is unchanged, and the all-target cross-compile is
still tag/manual-only — what came back is only the lightweight *correctness* gate, because Dependabot needs
one. The 3 Dependabot PRs already open (reqwest 0.13, tower-http 0.7, a github-actions group) are unaffected by
the config change (it applies to future runs); once `ci.yml` lands on `main`, Dependabot rebases them and the
gate runs on each — so their check status shows directly whether the breaking bumps build.

## 2026-07-15 — Least-privilege GITHUB_TOKEN across all workflows (CodeQL alert)

First finding from the just-enabled CodeQL default setup — `actions/missing-workflow-permissions`, on **all
three** build workflows. A true positive: a workflow with no `permissions:` block runs on the repo's default
token, which can be read-write; that violates least privilege for no reason.

Added a top-level `permissions: contents: read` to `release-build.yml` (reusable), `manual-build.yml`, and
`release-build-publish.yml`. The release **publish** job keeps its own `contents: write` — the one privilege
that creates the Release (`write` subsumes `read`); everything else (checkout, build, test, artifact upload)
needs only read (artifact upload uses the Actions runtime token, not `GITHUB_TOKEN`). `security.yml` already
ran read-only. `ci.yml.disabled` is parked — not scanned by Actions/CodeQL — and gets the same block if it is
ever revived. No release behaviour changes.

## 2026-07-15 — Automated dependency-security scanning (Dependabot + cargo-deny)

Added free, public-repo supply-chain scanning — a privacy proxy inherits the CVEs of everything it links, so
the dependency surface is now scanned automatically instead of by hope. All three additions are infra/docs;
no source changed.

- **`.github/workflows/security.yml`** — runs **`cargo deny check advisories bans sources`** via
  `EmbarkStudios/cargo-deny-action@v2`. `advisories` = the **RustSec** CVE DB (the security core); `sources`
  pins crates to crates.io (an unknown git/registry host is the shape a supply-chain attack takes); `bans`
  flags duplicate versions. Triggers: PR + push-to-main **on dependency-manifest changes only** (paths-filtered,
  so doc commits don't spin it up), a **weekly `cron`** (the important one — a CVE can be disclosed against a
  dep you already have, with no code change to trigger a run), and `workflow_dispatch`.
  - **Set `rust-version: stable`.** The action defaults to cargo 1.71, which can't even run `cargo metadata`
    on this tree — it pulls crates needing `edition2024` (cargo ≥ 1.85). cargo-deny only needs a cargo new
    enough to *read* the dependency graph, **not** the project MSRV, so `stable` (always ≥ 1.85) is the lean
    choice — no hardcoded version to drift from `Cargo.toml`. (Left at the default it would have failed on the
    first run.)
- **`deny.toml`** — config for the above. `advisories.unmaintained = "workspace"` (flag OUR deps going
  unmaintained, not the dormant transitive world = noise); `sources` fail-closed; `bans` warn on dup versions,
  deny wildcards. **`licenses` is deliberately OFF the CI command** — compliance, not security, and noisy
  against the `onnx` crypto stack (`ring` / `aws-lc-sys` carry non-SPDX license refs). A ready-to-enable
  allow-list is commented in the file (this crate is AGPL-3.0-or-later).
- **`.github/dependabot.yml`** — version updates for the `cargo` and `github-actions` ecosystems (grouped
  weekly to keep PR noise down). Dependabot **alerts + security updates** are a separate repo toggle the
  maintainer enables in *Settings → Code security & analysis* (same for native CodeQL / secret scanning).

**On the push/PR posture.** The pipeline change below (same day) disabled the *build* CI — fmt/clippy/test
moved to build/tag time. This adds back a **security-only** workflow on push/PR/schedule, deliberately narrow:
it does not compile the crate (cargo-deny reads `Cargo.lock`), so it does not reintroduce per-push build cost,
and fmt/clippy/test stay exactly as that change left them. The design posture is recorded in ARCHITECTURE →
*Supply-chain & dependency security*.

## 2026-07-15 — Pipeline change: CI disabled, tag-driven release only, + Windows-arm64 target

A deliberate change of approach to the GitHub Actions setup (maintainer's call), reversing part of the M5
CI story:

- **`ci.yml` disabled** (renamed `ci.yml.disabled`). Nothing runs on push/PR anymore. **`cargo test` was
  moved into `release-build.yml`** (see below), so the suite still runs on GitHub — on *every* target's native
  runner, at `manual-build` / tag time — just not per push; a release that fails its tests never publishes.
  **`fmt` / `clippy` / `msrv` are now local-only** gates (the CLAUDE.md "green before done" bar). A lint break
  surfaces at build time or locally, not on a PR. (Tradeoff accepted deliberately; recorded so it isn't a
  surprise.)
- **`release-publish.yml` → `release-build-publish.yml`** (`name: Release build & publish`). Same tag-only
  trigger, same structural "no manual run can publish". The other workflows' cross-references were updated to
  match.
- **READMEs now badge the release, not CI**: the `release-build-publish` workflow status ("did the last tag
  build?") plus a `github/v/release` version badge, replacing the old `ci.yml` badge. A tag is needed before
  either shows anything — hence the interim `v0.4.0` below.
- **Added Windows-arm64** (`aarch64-pc-windows-msvc`) to the matrix, built natively on GitHub's free
  `windows-11-arm` runner. **Not yet validated.** A local `cargo check --target aarch64-pc-windows-msvc
  --features onnx` failed — but on a *local-toolchain* gap, not the target: `aws-lc-sys` (onnx TLS stack)
  compiles its ARMv8 crypto from source and needs the ARM64 MSVC toolchain this x86_64 box lacks (`CC=None`,
  `LNK1181 chacha-armv8.o`). aws-lc-sys *does* support the triple but has had win-arm64 friction upstream, so
  it is plausible-but-unproven on the native runner. **Now validated GREEN** — a `manual-build` run built all
  5 targets, win-arm64 included (run 29437007077, 2026-07-15). The native `windows-11-arm` runner has the
  ARM64 toolchain the local x86_64 box lacked, so aws-lc-sys built its ARMv8 crypto with no friction. The
  caution was worth taking; the outcome is positive.
- **Then `cargo test --features onnx` was added to `release-build.yml`** (per the maintainer) — so the tag
  build and every manual build now test on each target's native runner (the old CI only tested x86_64 Linux).
  This still needs one more `manual-build` to confirm the test step is green on all arches before the tag.
- **Interim tag `v0.4.0`** (the current version) to populate the release badge — explicitly *not* the `1.0.0`
  release, which still waits on M6. Cut only after a `manual-build` (with the test step) is green on all targets.

No source changed — workflows + docs only.

## 2026-07-15 — M6 opened: native Anthropic `/v1/messages` (Claude Code passthrough); `1.0.0` gate moved behind it

Promoted the Claude-Code slice of Option B into a scheduled milestone, **M6**, after establishing that the
tool cannot front the LLM we actually use: Claude Code speaks **only** the native Anthropic Messages API
(`POST /v1/messages`), and the proxy is OpenAI-compat-only (in *and* out), so a Claude Code session pointed
at it just 404s.

**Grounded in the prior proxy, not guessed.** Studied `francesco-stimola/llmproxy-extended` — specifically
the maintainer's own commit `d9962a4` *"add Anthropic-native /v1/messages route for Claude Code passthrough"*
(stabilized in `c658e6c`). Its design: accept `/v1/messages`, run the masking pipeline **on the
Anthropic-native body in place** (inject the top-level `system` field as a temporary message so the masker's
`messages[]` walk covers it, then restore), and forward **native→native** to Anthropic — no OpenAI
translation. Auth is a verbatim passthrough of the client's `Authorization: Bearer` (the OAuth
`sk-ant-oat01-*` token) + `anthropic-beta`; the proxy never holds a key. The commit's own note pins the
gotcha: **an OAuth token in `x-api-key` → Anthropic 401** — Bearer for OAuth, x-api-key for API keys.

**Two corrections to my earlier read, recorded honestly:**
- *Auth is not a blocker.* I had said Claude Code was blocked on both schema *and* auth. The prior proxy
  proves auth works via verbatim Bearer + `anthropic-beta` passthrough on a native route. The **only** real
  blocker is the missing native schema handling.
- *Translation is the wrong tool here.* The fork's base (Fabrizio Salmi) ships a full OpenAI↔native
  translation adapter for ~25 providers. Considered and **rejected** for the masking path: two lossy schema
  boundaries = two leak surfaces, against fail-closed. Adopted the maintainer's **mask-in-place** route
  instead; the translation adapter is kept only as a *field map* for the Anthropic schema.

**Where M6 improves on the prior proxy:** that route left **streaming demask as a TODO** — its streamed
replies were forwarded un-demasked, i.e. placeholders could reach the client. M6 makes the Anthropic SSE
demask (`content_block_delta` → `delta.text`) a scope item, reusing the hold-back `SseDemasker` we already
have for the OpenAI shape. A placeholder reaching the client is the exact failure this tool exists to
prevent, so it cannot be a TODO here.

Also: the prior proxy's PII *detection* had real gaps (SECRET unreliable, IBAN misclassified, names missed —
its own README's "Known Issues") — all already solved in this Rust proxy. M6 reuses the old **transport /
schema** design, never its detection.

**Release plan changed (per the maintainer):** the `1.0.0` tag now waits on M6 — a release ships only once
Claude Code works end-to-end against real Anthropic. Fronting the LLM we actually use is a `1.0` requirement,
not a follow-on. This also finally makes the M5 manual dual-run executable for real (a Claude Code
subscription drives the native route). README + README.it now state the OpenAI-compat-only scope explicitly
and point native-client support at M6. No source changed yet — M6 is scoped, not built.

## 2026-07-15 — M5's manual verification: dry-run-validated against a mock; the live-provider bar redefined to opt-in

Closed M5's last open box — the manual dual-run of `docs/MANUAL_VERIFICATION.md` — by **redefining its
bar**, transparently. The box asked for a run against the *real* provider, which needs a live
`ANTHROPIC_API_KEY` this environment does not hold.

**The obvious workaround was investigated and ruled out.** "Use this Claude Code session's own Anthropic
access as the credential" fails twice over: Claude Code speaks **only** the native Messages API
(`POST /v1/messages`, content blocks / `tool_use`), while the proxy proxies **only** the OpenAI-compat
`/v1/chat/completions` — a Claude Code session pointed at the proxy just 404s, nothing masked; and the
subscription credential is an **OAuth token scoped to Claude Code's flow** (the `anthropic-beta` header
carries an OAuth capability the upstream requires), not a plain `Bearer` usable against the raw API. So a
real-provider run cannot be surrogated from inside a session. Routing Claude Code *through* the proxy is
exactly **Option B** (native `/v1/messages` masking) — Backlog, out of M5.

**What was actually done:** ran the procedure's Run A / Run B through the **real compiled binary** against a
throwaway Node mock upstream that echoes the masked text it receives.
- **Run A** (`PII_DEBUG_SKIP_DEMASK=1`) → client received `[EMAIL_1]`; the `forwarding masked request body
  upstream` trace carried the placeholder; the mock saw only the placeholder; the raw email appeared in
  **neither** log stream (DBG-02).
- **Run B** (normal) → client received the restored `jane.doe@example.com`; the trace still showed only the
  placeholder, and the de-masked client output was **never** logged.
- **A vs B on the same input → a byte-identical masked body upstream**: the chain holds end-to-end.

**Why this is enough to close the box (a conscious call, not a silent one):** the *permanent* guarantee
already lives in CI — DBG-01 (`e2e_debug_skip_demask_returns_placeholders_to_client`) and DBG-02
(`tests/log_safety.rs`) — plus the three-preset mock e2e (`tests/proxy_e2e.rs`). The manual dry-run only
confirmed the **written procedure** itself works as documented. The real-provider dual-run is therefore
reclassified from a close-gate to an **opt-in** extra, ready in `docs/MANUAL_VERIFICATION.md` and
`tests/anthropic_smoke.rs` (E2E-INT-01) for whoever next holds a key. The `1.0.0` release gate is unchanged
and still stands on its own (a real green CI run).

Incidental finding folded into the guide: `tracing` writes to **stdout**, so the DBG-02 grep must include
stdout (a `2>`-only redirect would miss the trace). No source changed; no tests added or removed.

## 2026-07-15 — CI builds every release target; workflows renamed; the Node-24 bump done right

Three follow-ups after the first manual run went **green on all four targets** (including the new
`aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm` — so that runner and `ort`'s linux-arm64 prebuilt both
exist):

- **The Node-20 warning, fixed *properly* this time.** The previous bump to `@v5` was wrong: reading the
  actions' own `action.yml`, **`upload-artifact@v5` still declares `using: node20`** (v6 was the first on
  node24), and **`download-artifact@v6` is node20** too (v7 was its first node24). Bumped to the current
  node24 majors — **`upload-artifact@v7`, `download-artifact@v8`** (they differ because they are separate
  repos); `checkout@v5` was already node24. Lesson: don't infer an action's Node runtime from its version
  number — read `runs.using` in its `action.yml`.
- **CI now builds every release target** (reverses the "CI stays lean" call in the entry below). That call
  was made on *cost* grounds — and GitHub Actions is **free for public repos**, so the premise was wrong.
  `ci.yml` gained a `release-targets` job that calls the reusable `release-build.yml` with
  `upload-artifacts: false`: every push/PR now cross-compiles all four targets (a compile check, no
  artifacts), so a target-specific break like the Intel-mac one surfaces on a PR instead of at tag time.
  The reuse means "what CI checks" and "what a release builds" are the *same definition*, byte for byte.
- **Workflows renamed for clarity** (no more "build" vs "release" ambiguity): `build.yml` → `release-build.yml`
  (`name: Release build`), `release.yml` → `release-publish.yml` (`name: Release publish`). `manual-build.yml`
  keeps its name. The reusable workflow grew an `upload-artifacts` boolean input (default true; CI passes
  false) so the one definition serves both "build and keep" and "build to check".

All four workflows re-validated as YAML; no source changed.

## 2026-07-15 — release pipeline restructured: one build definition, two entry points, tag-only publish

Follow-up to the first-run fixes below. Reworked the packaging pipeline from one `release.yml` into
three files so the *manual* build and the *release* build share one definition and can't drift, and so
"a manual run can never publish" stops being an `if`-gate and becomes structural:

- **`build.yml`** — a **reusable** workflow (`on: workflow_call`) holding the whole cross-compile matrix
  and the package/upload steps, with a `retention-days` input. It has **no publish step at all**.
- **`release.yml`** — triggers **only on a `v*.*.*` tag**, calls `build.yml`, then publishes. It no longer
  has a `workflow_dispatch` trigger or a publish `if`-gate: with no manual trigger on the *publishing*
  workflow, a manual run structurally cannot cut a release. This **retires [M5-R11](reviews/M5.md#m5-r11)**
  (the event-vs-ref gate) — there is nothing left to get wrong. *(A note the first-run entry got slightly
  wrong: that run didn't publish primarily because a build failed; it didn't publish because it was a
  **manual event**. Even four green builds would not have published. This restructure makes that guarantee
  obvious instead of subtle.)*
- **`manual-build.yml`** — `workflow_dispatch` only, calls the same `build.yml` with **30-day** retention
  (manual builds are throwaway checks), and has **no publish job**. This is the pre-tag "do all targets
  still compile?" button.

**Added `aarch64-unknown-linux-gnu`** to the matrix — ARM Linux servers (Graviton et al.) are common and
`ort` *does* ship a prebuilt for it (unlike Intel macOS). Built **natively** on GitHub's `ubuntu-24.04-arm`
runner, so there's no cross-linker to maintain. Matrix is now Linux x86_64 + arm64, macOS arm64, Windows
x86_64-msvc. Comments are kept **count-agnostic** so adding/removing a target doesn't need a prose edit.

**On aligning CI with the release targets:** deliberately *not* done. A fat-LTO cross-build of four targets
on every PR is slow and costly for marginal benefit; `manual-build.yml` *is* the on-demand all-targets
check, run before a tag. (A scheduled weekly run is the cheap automation if drift-detection is ever wanted
— noted, not built.) `ci.yml` stays the fast per-push correctness gate (test/clippy/msrv on ubuntu).

All four workflows re-validated as YAML. No source changed. **`1.0.0` gate unchanged**: a real, fully-green
run — now easiest to get by running `manual-build.yml` across all targets first, then pushing the tag.

## 2026-07-15 — the release workflow's first *real* run: two things a green local build can't show

`release.yml` had never actually run — ROADMAP flagged exactly that as the open gate for `1.0.0`. A
manual `workflow_dispatch` run finally exercised it, and (as the whole point of running it was to find
out) it surfaced two things no amount of local `cargo build` could:

- **`x86_64-apple-darwin` fails to build.** `ort`'s `download-binaries` ships **no prebuilt ONNX Runtime**
  for Intel macOS at the pinned `=2.0.0-rc.12`; the build then falls back to compiling ONNX Runtime *from
  source*, which needs a cmake toolchain we don't set up in CI → exit 101. The other three targets
  (Linux x86_64, **macOS aarch64**, Windows x86_64-msvc) built clean and the publish step **correctly
  skipped** (manual run, not a tag — the [M5-R11](reviews/M5.md#m5-r11) event-gate working as designed).
  **Decision: drop the target, don't build ORT from source.** Intel Macs are legacy (Apple finished the
  arm64 transition in 2023), `aarch64-apple-darwin` covers every current Mac, and a from-source ORT build
  is a long, fragile, cmake-dependent step to maintain for a shrinking audience — the *over-engineering*
  the project's bar rules out. An Intel-Mac user builds from source (`docs/SETUP.md`); we don't ship a
  binary no one else here can reproduce.
- **Node-20 deprecation warnings.** `actions/checkout@v4` and `actions/upload-artifact@v4` target Node 20,
  which GitHub is retiring (the runner force-ran them on Node 24 and warned). Bumped the JS actions to the
  current majors on Node 24 — `checkout@v5`, `upload-artifact@v5`, `download-artifact@v5` (verified those
  majors exist and are the matched artifact generation). `Swatinem/rust-cache@v2` and
  `softprops/action-gh-release@v2` were **not** flagged, so left as-is; `action-gh-release` sits in the
  publish job the manual run skipped, so if it warns on the first *tagged* run we bump it then.

No source changed — workflow + docs only. Both workflow files re-validated as YAML, the matrix now lists
exactly the three buildable targets, and ROADMAP's target list is corrected (the DEVLOG entry that first
recorded `release.yml` still reads "x86_64 + aarch64" — that was true when written; this entry supersedes
it rather than rewriting history). **The `1.0.0` gate is unchanged: a real, fully-green run of these
workflows, ideally after the manual live-provider check.**

## 2026-07-14 — M5 review round 3 closed: a guard's own guard, and the ledger goes clean (12/12)

Round 3 reviewed the *product* changes (single MSRV, release profile, the manual release trigger, the
optional provider token, the README rewrite), re-verified the M5-R7 fail-closed fix **through the real
binary**, and opened three findings — **all now closed. M5's ledger is 12/12 and clean.** Build + `fmt`
+ `clippy` clean on both feature sets. Record: [`reviews/M5.md`](reviews/M5.md).

**M5-R10 — the guard the M5-R7 closure rests on guarded the wrong thing.** M5-R7 argued the token-overflow
error path is unreachable, on the strength of 32 tokens of headroom. But the compile-time invariant that
was supposed to *hold* that headroom asserted only `MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS` — satisfied by
**any** headroom ≥ 1. The reviewer measured it: a 511-token window passes the assert and re-tokenizes to
~514, and the PERF-01 `Expand` error is back — reintroduced by a one-character edit that every CI-runnable
guard approves. Fix: make the **headroom** the invariant —
`MODEL_MAX_TOKENS - MAX_WINDOW_TOKENS >= MIN_DRIFT_HEADROOM_TOKENS` (16). The const subtraction underflows
on a window over the ceiling, so it subsumes the old assert rather than sitting beside it. A 510-token
window now **fails to compile** where it used to build green and overflow at runtime.

> **The lesson, promoted to ARCHITECTURE:** *a compile-time invariant must encode the constraint the code
> relies on, not the weakest one that happens to hold at today's values.* `A < B` is not "A leaves room
> for drift" — and when that invariant is the **only** guard a modelless CI can run, the gap between the
> two is the whole exposure. This is the M5-R2 lesson (*"a bound you do not check is not a bound"*)
> recurring one level up: the *check itself* was bounded by a constant nothing checked.

**M5-R11 / M5-R12 — the small ones, both "one home for a fact."** The release workflow's publish gate keyed
on the *ref* (`startsWith(github.ref, 'refs/tags/v')`), but GitHub's "Run workflow" picker lists tags too,
so a manual run *on a tag ref* would have cut a real release — the one thing the gate's comment promises
can't happen. Now gated on the **event** as well (`github.event_name == 'push' && …`). And the
`panic = "unwind"` rationale in `Cargo.toml` cited a **400** for the caught-panic path that is actually a
**500**; rephrased to "blocks the request (fail-closed)", the load-bearing part that can't go stale (the
profile's conclusion was always correct). Both are the *fourth and fifth* M5 items where a fact was right
in one file and stale in a second that repeated it.

Also, unrelated to the review: the README banner now spells **`llm-proxy-pii`** (was `llm-proxy`) — the
`-PII` glyphs appended in the same ANSI Shadow font, EN + IT in step.

## 2026-07-14 — M5 review round 2: my own fix committed the M4 retrospective's signature move

Round 2 verified round 1's six closures (five hold outright) and found three more. **All closed. M5's
ledger is 9/9.** 132 tests green (default) / 145 + 6 `#[ignore]`d (`--features onnx`); `fmt` + `clippy`
clean on both. Record: [`reviews/M5.md`](reviews/M5.md).

**M5-R7 — the fail-closed regression, and it was mine.** My M5-R2 fix enforced the token ceiling by
**clamping** an over-long NER sequence and returning `Ok(partial)` — reasoning that losing a window's tail
beats losing the whole field. The reasoning is fine. **Making the call there is not.** Under `NER_REQUIRED`
the detector goes into the composite *unwrapped*, so a `try_detect` `Err` is what produces the 400. Before
the fix an over-budget sequence errored → **400, nothing forwarded**. After it, the same condition returned
`Ok` → the request is **forwarded with a window's tail unscanned**, to an operator who had explicitly asked
to be blocked. Two claims in my own doc comment propped it up and neither survives: the overlap does **not**
re-cover the *last* window (it has no successor), and — the tell — I had written the 400 as the *bad*
outcome, when under `NER_REQUIRED` **the 400 is the product**.

> **The rule, now in ARCHITECTURE:** *a detector may degrade its own recall, but it may never decide **for
> the caller** that degraded output is acceptable.* Fail-open vs fail-closed is `FailOpen`'s decision and
> the only road to it is the `try_detect` error channel. The clamp was **right for the default posture and
> fatal to the other**, and a component that cannot see the posture must not choose between them. **When in
> doubt, a detector returns `Err` and lets the wrapper decide.**

The fix is the *minimum*, deliberately: `run_and_decode` returns `Err`, naming the cause. I skipped the
reviewer's "better" (re-split the window until it fits) because it buys a retry loop, a termination
argument and a coverage-gap argument to guard a path that **cannot currently be reached** — 32 tokens of
headroom against a measured +1…+3 drift. And the suggested test (make the consts injectable so the valve is
forceable) is *no longer needed*, which is the point: there is no valve. The overflow is now an ordinary
detector error, and "a required detector's error blocks / a `FailOpen`-wrapped one is swallowed" is already
pinned by FC-04 and CMP-02. **The fix didn't just remove the wrong behaviour — it removed the category of
behaviour that needed a bespoke guard.**

**M5-R8 / M5-R9 — the same drift, twice more.** ARCHITECTURE still named `MAX_SEQUENCE_TOKENS` (a constant
that no longer exists) and still called the window *"conservatively under `max_position_embeddings`"* — the
exact hope M5-R2 refuted — **in the file M5-R1 had just promoted to sole home**. Rewritten around what
ships, and the refuted framing is *kept as a warning* rather than deleted: **a bound you do not check is not
a bound.** Also written down at last: the chunker's unstated assumption that the `(0, 0)` offset sentinel
appears **only at sequence ends** (verified over 17 adversarial inputs; a tokenizer swap must re-check it).
M5-R9: the M5-R2 guard hand-copied `32` for `CHUNK_OVERLAP_TOKENS` — the one constant `chunk_char_ranges`
was made `pub` to *avoid* hand-copying. Now `pub` too. **A guard that shares only *some* of its subject's
constants is measuring a program that doesn't exist.**

## 2026-07-14 — Product pass: single MSRV, manual release trigger, max-opt profile, README rewrite

Four changes driven by how the product will actually be run and shipped.

- **One MSRV: `1.89`.** M5-R5 measured two floors (1.86 default, 1.89 `onnx`) and declared 1.86, keeping
  1.89 as documentation. But the product **always runs with `onnx` on** — that is what the hybrid *is* — so
  a "default-build MSRV" is a promise about a configuration nobody deploys: **a second number to keep
  honest, buying nothing**, which is precisely the drift shape this whole review round is about. Manifest
  and CI now carry the single real floor. **It paid for itself immediately:** raising `rust-version`
  un-gated an MSRV-aware clippy lint that `1.82` had been suppressing
  (`clippy::manual_is_multiple_of`, stable since 1.87) — on `luhn_valid`, the card checksum. Semantically
  identical, corpus tests unchanged, but a neat proof of the finding's own thesis: **an under-declared MSRV
  doesn't just fail to protect you, it hides the tooling that would.**
- **The release pipeline does not fire on every push to `main`** (it never did — it was tag-only). Added
  **`workflow_dispatch`**: a "Run workflow" button that builds all four targets from any branch and
  attaches them as **artifacts only**. The publish step is now **tag-gated** (`if: startsWith(github.ref,
  'refs/tags/v')`), so a manual run cannot accidentally cut a release.
- **`[profile.release]` is now the max-optimization one** — `opt-level = 3`, fat LTO, `codegen-units = 1`,
  symbols stripped (~5 min build; the masking path *is* the product's latency). **`panic` stays `unwind`,
  and that is a correctness bar, not a preference:** `abort` would turn a caught masking panic — which
  today blocks **one** request, fail-closed (M4-R19) — into a **process abort**. Converting a contained
  fail-closed into an outage is not an optimization; availability is a privacy property here. Verified the
  stripped LTO binary boots, answers `/healthz`, and still 404s an unproxied path.
- **README rewritten** (EN + IT) as a product README — problem, an ASCII flow diagram, a `curl` showing the
  *actual* masked body the provider receives, the detection matrix, the bars it holds itself to. **The
  internal development status is gone from it**: that belongs in ROADMAP, and the README now defers there.
  Also **`MANUAL_VERIFICATION.md` + `anthropic_smoke.rs`: the API key is optional.** The proxy already
  forwards a client `Authorization` verbatim in preference to `UPSTREAM_API_KEY`, so the runbook now leads
  with the mode where **the proxy never holds the credential at all** — and the smoke test drives that
  path, which is both the recommended posture and the stricter thing to prove.

## 2026-07-14 — M5 review round 1 closed: five of six findings were a *claim that had stopped being true*

All six M5 findings closed. **132 tests green (default) / 145 + 6 `#[ignore]`d (`--features onnx`);
`fmt` + `clippy` clean on both.** Full record: [`reviews/M5.md`](reviews/M5.md). No leak, no fail-open,
no over-mask regression — the reviewer reproduced the pre-fix `Expand` error from the parent commit and
drove the real binary end-to-end with NER on an oversized field before finding anything.

**Two findings were bigger than they were filed as, and for the same reason: the finding said "this
claim is unverified", and verifying it showed the claim was *false*.**

**M5-R5 — the declared MSRV was fiction (filed `low`; really the round's most load-bearing).** The
finding was "CI pins `stable`, so `rust-version` is never exercised". It offered two fixes — add the
job, or drop the claim. I measured instead, and **`1.82` cannot even parse the dependency tree**
(`idna_adapter` needs `edition2024`). The real floors:

| build | declared | true floor |
|---|---|---|
| default | 1.82 | **1.86** (`icu_*` / `idna_adapter`) |
| `--features onnx` | 1.82 | **1.89** (`redb` ← `hf-xet`) |

They **differ per feature set**, which one `rust-version` field cannot express: it now declares **1.86**
(the shipped, native-dep-free default build) and documents 1.89 for onnx, where cargo's own
`redb@3.1.3 requires rustc 1.89` is self-explanatory. A new `msrv` CI matrix **builds** both.

> **This is the M4-R22 lesson finally landing.** M4-R22 added `rust-version` *to prevent exactly this*,
> and it structurally cannot: the field makes cargo refuse a too-**old** toolchain; nothing makes the
> crate stay compatible with it. **A declared MSRV with no job building on it is not a floor — it is a
> comment shaped like a guarantee.** Only a job that builds on the MSRV can hold the MSRV.

**M5-R2 — the constant that bounded nothing.** `MAX_SEQUENCE_TOKENS = 480` claimed to bound the sequence
fed to the model. It bounded the *planning window*: `infer_chunked` **re-tokenizes** each window from its
own text (it must — a middle window needs its own `<s>…</s>` framing), which adds the specials and drifts
at the cut edges, so the real sequence was **always over** the "bound" — measured 481–483 against a
usable ceiling of 512 (XLM-R's 514 `max_position_embeddings` minus RoBERTa's position-id offset of 2).
29 tokens of headroom, held by nothing. Now: the constants are split (**`MAX_WINDOW_TOKENS`** vs
**`MODEL_MAX_TOKENS`**), the ceiling is **enforced** in `run_and_decode` (the single choke point every
path into the session goes through) with a clamp + kind-free `warn!`, their relationships are
**compile-time** invariants (`const _: () = assert!(…)` — get one wrong and the crate doesn't build), and
the drift is **asserted** by a live guard over six adversarial scripts (CJK, Cyrillic, zalgo, a
4 000-char run of `あ` with no spaces), which independently reproduced the reviewer's 481–483.

**M5-R3 — closed with the guard the codebase already had.** The chunk slice `&input[a..b]` hard-indexed
tokenizer offsets — the one spot on the masking path that could panic on attacker input, while
`decode_entities` (M2-R6), `Vault::mask` and `overlap::materialize` all refuse to. The fix is to *apply
the existing rule*, not invent a parallel one: `chunk_char_ranges` now widens every window through
**`overlap::widen_to_char_boundaries`** (promoted to `pub(crate)`), making the ranges sliceable **by
construction**; `infer_chunked` still uses `.get()` + `debug_assert!` + skip. Its unit test carries **its
own non-vacuity assertion** — it checks the offsets table really does cut a multi-byte char, because a
guard that quietly stops exercising its hazard is exactly how M4-R13 and M4-R24 stayed invisible.

**M5-R4 — the fixpoint's proof has a hole, and the *next* model is the one likely to fall in it.**
"A placeholder is inert" is proved **by construction** for the regex recognizers (`[KIND_N]` has no `@`,
no `sk-`, not enough digits). But `mask_all` runs the **composite**, and an ML model is under no such
constraint. If a model tagged `[PERSON_1]`, masking would never shrink the text, `MAX_MASK_PASSES` would
exhaust, and the request would **400** — fail-*closed*, never a leak, but a hard availability failure on
ordinary input. It holds for XLM-R (0 entities on placeholder-only text — now a live guard), so it is an
**empirical property of the chosen model**, not a theorem. Written down next to the invariant in
ARCHITECTURE *and* on `mask_all`, because the Backlog's designated successor is **GLiNER** — a zero-shot,
open-label, **context-driven** extractor, i.e. precisely the kind of model that would read
`Contact [PERSON_1] at [ORG_1]` and tag both. **M5 is also what made this reachable**: before chunking, a
field that large never reached the NER at all.

**M5-R1 / M5-R6 — the docs.** TESTING.md still asserted the NER was unchunked and quadratic — the claim
M5 both *disproved* and *fixed* — because that claim lived in **two** files and only ARCHITECTURE was
updated; it is now a **pointer**, not a duplicate. Both READMEs named, as a future trigger for `1.0.0`,
the very commit that contained the sentence; they now state the real gate (CI has never actually run; the
live-provider check has never been performed) and defer to ROADMAP.

> **The through-line.** Five of six findings are one shape: *a claim that was true when written, was
> never re-checked, and had quietly stopped being true.* None leaked. All were **documentation of a
> guarantee that had drifted from the guarantee.** The project's answer to that is already written down
> for review findings — **one home for a fact** — and this round is what applied it to **design claims**,
> which is where the drift actually lives.

## 2026-07-14 — M5: README + CI/release workflows — code-complete, one box left

Closed the two remaining M5 items that don't need a live provider.

- **README.md + README.it.md rewritten** from the "early development" placeholder to describe
  the shipped product: what it does (three-tier structured detection, optional NER, streaming,
  multi-provider), a quick-start, and a full env-var reference table for both the core proxy and
  the `onnx` feature.
- **`.github/workflows/ci.yml`** — `fmt` (once, feature-independent) + `clippy` + `cargo test`,
  matrixed one job for the default build and one for `--features onnx`, on push to `main` and on
  every PR. Model-dependent NER tests are `#[ignore]`d, so the `onnx` job needs no model file.
- **`.github/workflows/release.yml`** — on a `v*.*.*` tag, cross-compiles the full
  `--features onnx` product for Linux (x86_64-unknown-linux-gnu), macOS (x86_64 + aarch64
  -apple-darwin), and Windows (x86_64-pc-windows-msvc) and attaches the binaries to a GitHub
  Release.
- **Neither has been exercised live yet** — no PR has run the new CI, and no tag has been
  pushed. Both are standard, unremarkable GitHub Actions shapes, but "the YAML parses" is not
  "it's green"; that only gets proven the first time a real push/PR/tag runs them.

**M5 is now code-complete except one box that cannot be closed from inside a session:** the
manual dual-run verification (`docs/MANUAL_VERIFICATION.md`, E2E-INT-02) needs a human with a
real `ANTHROPIC_API_KEY` to actually run it and read the trace output — see the entry below for
what's already in place around it. The first tagged release (`1.0.0`) should wait for a real
green CI run at minimum, and ideally that manual check having been run once.

## 2026-07-14 — M5: integration tests, a performance harness, and a real NER bug found via testing

Picked up M5 (integration & performance testing) end to end. Four threads, in the order they
landed:

**1. Real integration tests (`tests/proxy_e2e.rs`).** Implemented the two cataloged-but-missing
old-proxy scenarios — **E2E-02** (PII in a CSV `tool_result`) and **E2E-04** (all six structured
categories in a `SELECT … FROM DUAL`-style result) — against a mock upstream, both asserting
the masked-upstream / restored-to-client pair like the existing E2E-01/03. Added three more: a
**full-HTTP tool-call round-trip** (the real-server companion to the pipeline-level INT-03: a
mock's `tool_calls[].function.arguments` referencing a placeholder is de-anonymized before the
client sees it), a **multi-turn determinism** e2e (two real HTTP round-trips resending
conversation history, proving a repeated value keeps its placeholder token across two
independent per-request vaults — the stateless-client shape this proxy actually serves), and
extended the provider-agnostic test (LOC-11) to compare **all three** mock upstream shapes M5
asks for (OpenAI / Copilot / Anthropic), not just two. Also added `tests/anthropic_smoke.rs`
(E2E-INT-01): a real-provider smoke test against Anthropic's OpenAI-compat endpoint —
`#[ignore]`d, gated on a real `ANTHROPIC_API_KEY`, never run in CI. **Written and compiling, not
yet run against a live key** (no credentials in this environment) — the same posture the project
already uses for `ner_eval.rs`.

**2. The manual dual-run runbook (`docs/MANUAL_VERIFICATION.md`, E2E-INT-02).** A step-by-step
procedure for the check that can't be a `#[test]`: run the same PII prompt twice against a real
provider, once with `PII_DEBUG_SKIP_DEMASK=1` (proves the request left masked) and once normal
(proves the client gets the restored value), with `RUST_LOG=…=trace` so DBG-02 (never-log-raw-PII)
gets re-checked on real data. Written, not yet executed — same reason as above.

**3. A performance/load harness (`tests/perf.rs`).** The system-level companion to
`tests/complexity.rs` (which pins the masking *algorithm's* complexity, no HTTP):
`healthz_stays_responsive_under_concurrent_masking_load` fires 8 concurrent ~350 KB / 50 K-entity
requests and polls `/healthz` while they're in flight, turning the M4-R19 "masking runs on
`spawn_blocking` so the executor never starves" architecture claim (previously only
hand-measured) into a repeatable guard — measured **~40 ms** for `/healthz` under load (budget
2 s). `streaming_throughput_of_repeated_placeholder_restoration_stays_within_budget` streams
~150 KB containing ~6000 placeholder occurrences through the real SSE de-anonymizer in small
fragments — measured **~0.75 s** (budget 20 s). Both are generous wall-clock budgets in the
`tests/complexity.rs` style: they catch a regression back to seconds-to-minutes, not a
micro-benchmark.

**4. NER field-size measurement — and a real, live bug (`tests/ner_perf.rs`, `src/pii/onnx.rs`).**
The ROADMAP's own suspicion was that `OnnxNerDetector` feeding a field to the model as one
sequence would be *slow* on large fields (quadratic self-attention). Measuring it found
something worse: past the model's `max_position_embeddings` (514 for the picked XLM-R int8), the
ONNX graph's position-embedding lookup goes **out of range**, and the run call fails outright
(`Non-zero status code returned while running Expand node … invalid expand shape`) — not a
slowdown, a hard error, on any field over roughly 500 tokens (~2 KB of prose). Fails open by
default (silently drops to structured-only for that request) but is a **hard 400 block** under
`NER_REQUIRED` — an availability gap in the same family as M4-R19/R24, though opt-in and off by
default so never the unauthenticated DoS those were.

**Fix: overlapping-window chunking.** `OnnxNerDetector::infer` now tokenizes once; if the
sequence fits the budget it runs exactly as before (M2, unchanged), otherwise `infer_chunked`
splits it into overlapping token windows (`MAX_SEQUENCE_TOKENS = 480`, `CHUNK_OVERLAP_TOKENS =
32`), each **re-tokenized independently** — a middle window needs its own `<s>…</s>` framing, so
it can't be a raw slice of the whole field's token ids — run through the same single-window
path, and merged with exact duplicates from the overlap deduped.

**And chunking had its own bug, caught only by testing at a size that exercised the last
window.** The first version computed a window's char end from `offsets[token_end - 1].1`. That's
correct *unless* `token_end == seq`, in which case `token_end - 1` is the closing `</s>` token —
whose offset is the sentinel `(0, 0)`, not the real text end. The bug collapsed the **entire
final window** to a zero-length slice: measured on a 64-sentence input, this silently dropped 61
of 192 entities (32%) with **no error at all** — worse than the original bug in one way, because
it degrades silently instead of failing loudly. Caught by `tests/ner_perf.rs` at reps=64 (the
first size in the sweep that needed two windows) — reps=16 had (correctly) only ever needed one.
Fixed: a window reaching the sequence end uses `input.len()` for its char end. Extracted the
window math into a pure `chunk_char_ranges` function (offsets + lengths in, char ranges out — no
tokenizer or model needed) specifically so this exact bug gets a **real unit test**
(`the_last_window_reaches_the_true_text_end_not_the_closing_token_sentinel`,
`src/pii/onnx.rs::chunk_tests`) rather than living only in an `#[ignore]`d, model-dependent check.

**Measured after the fix:** linear scaling and full recall — 64/256/1024× a repeated sentence run
in 448 ms / 2.07 s / 7.53 s (debug profile), 192/192, 771/768, and 3084/3072 entities found (the
small excess above 100% at larger sizes is an occasional un-deduped near-boundary double-detection
— a precision nit, not a recall loss). Re-measured M4-R21's ~2× fixpoint-confirmation cost live
too: ~1.8–3× on a short field, consistent with the 64 ms → 127 ms (1.99×) recorded when M4-R21 was
closed — confirmed as the deliberate correctness cost it always was, not a regression.

**132 tests green (default) / 144 + 4 `#[ignore]`d (`--features onnx`), no warnings; `fmt` +
`clippy` clean on both feature sets.**

## 2026-07-14 — Review 11: M4-R24 closure verified — DoS class closed on both axes

Independent reviewer pass over the builder's M4-R24 fix (`eed9949`). **Holds.** Read the rewritten
`Vault::mask` (one L→R copy into a capacity-reserved buffer, O(n + k)) and reproduced both splice strategies
in a standalone probe: old `replace_range` R→L is **×4 per doubling** of entity count (O(n²), 18 s at 400 K —
past DOS-04's budget), new forward copy is **×2** (linear), and the two produce **byte-identical** output, so
it is a pure speedup. Confirmed no third quadratic hides behind it (`demask` = linear `replace_all`,
`placeholder_for` = O(1)), DOS-04 is non-vacuous (`entities.len() == reps`, 600 K entities → 214 ms), the
malformed-span guard is drop-never-leak-never-panic, and MSRV `1.82` is untouched. Both feature sets green
(126 default / 134 + 1 ignored onnx), `fmt` + `clippy` clean. No new finding — **M4's ledger is genuinely
clean, 24/24**. Full round: [`reviews/M4.md#review-11`](reviews/M4.md#review-11).

## 2026-07-14 — M4-R24: the *other* quadratic — ask what a guard holds constant, not what it varies

**M4 is done — all 24 findings closed, and M5 is unblocked.** (This supersedes the "M4 is NOT done" line in
the entry below.)

**The bug.** `Vault::mask` spliced placeholders in **right-to-left** with `String::replace_range`. Correct,
and quadratic: every splice memmoves the whole tail of the string, so *k* entities in *n* bytes shift Θ(n·k)
bytes — and a field of many **small** values (`a@b.co `, an SSN, a phone) has *k* growing with *n*, so it is
**Θ(n²)**. A 13.4 MiB body of repeated emails — under the 16 MiB limit, on the **unauthenticated** masking
path — burned **~7 minutes** of CPU. The fix is one **left-to-right copy** into a fresh, capacity-reserved
buffer: O(n + k), each byte touched once. Placeholder numbering is untouched, because it always followed the
entities in *start order*; splice **direction** never determined it. Measured (debug, splice isolated):
800 K entities go from **91,049 ms → 272 ms**, and ×3.9-per-doubling becomes ×2.0 — **335×**, and linear.
Over HTTP, the reviewer's own payload: **13.4 MiB → 1.8 s, 200 OK**; eight concurrently in 4.3 s with
`/healthz` answering in 48 ms.

**Why it survived M4-R19, which was the *same* bug.** Because the masking path has **two** size axes, and
we had only closed one. M4-R19 made detection linear in the **field size**; this one is quadratic in the
**entity count**. They are independent, and *closing one says nothing about the other* — linear detection
does not bound the splice, which is why the M4-R19 pass sailed straight past it.

**And the guards could not see it — that is the lesson.** DOS-01…03 scale the field to megabytes, and every
single one of them pins the entity count at **one** (DOS-03's card row coalesces to k≈1). So they varied *n*
and silently held *k* constant, and a per-entity quadratic lived directly underneath them. The smoking gun is
exact: 13.4 MiB as **one** email masks in 219 ms; the **same** 13.4 MiB as many small emails took **421 s**.
Identical bytes — the only variable is *k*.

> **A quantity a test never varies is a quantity the test cannot see.** This is the M4-R13 lesson — *"a
> corpus has a shape, and that shape is a blind spot"* — arriving a second time, now on the **guards
> themselves**. A guard is a corpus too, and it has a shape. Ask what it holds *constant*, not only what it
> varies.

**DOS-04** is the guard that varies *k*: 600 K entities, timing `Vault::mask` **alone** (the code under test
— that makes it decisive, ~0.2 s linear vs ~52 s quadratic, where an end-to-end debug timing would have left
a flaky 2× margin). It asserts `entities.len() == reps`, so if the corpus ever coalesces back to k≈1 the
guard *says so* rather than quietly going blind again. **Non-vacuity, and it reproduces the blind spot
exactly: on the pre-fix splice DOS-04 times out while DOS-01/02/03 all pass.**

**One hardening the finding didn't ask for.** The new loop slices `text[cursor..start]`, so a malformed span
(overlapping / out of bounds / off a `char` boundary) would **panic** — and this is a proxy on
attacker-influenced input. The precondition (guaranteed by `resolve_overlaps`, the only production caller) is
now stated, `debug_assert!`ed, and in release handled by advancing the cursor **past** the bad span, widened
to a `char` boundary: **drop, never leak**, never a panic.

**One thing found while closing it, left open on purpose.** `OnnxNerDetector` feeds the **whole field as one
sequence** — no chunking — so the NER path is quadratic in field size from self-attention. Opt-in and off by
default, so it is *not* the unauthenticated DoS these two were, and not a leak. But it is the third
appearance of the same lesson, so the docs now say "the **structured** path is linear" rather than "the path
is linear", and PERF-01 must measure it before NER is recommended for large bodies.

**126 tests green (default) / 134 + 1 `#[ignore]`d (`--features onnx`); `fmt` + `clippy` clean on both.**

## 2026-07-13 — Review 10: the DoS pass verified, and a *second* O(n²) found (M4-R24, reopens M4)

**M4 is NOT done — [M4-R24](reviews/M4.md#m4-r24) (BLOCKER) is open; M5 is blocked again.** (Supersedes the
"M4 is done" line in the entry below, which was true only of M4-R19…R23.)

Independent verification of M4-R19…R23. **They hold:** M4-R19 (the candidate *rescan* quadratic) is
genuinely closed — the two unbounded shapes are linear by measurement (email/secret ~15–30 ms at 1–2 MB),
DOS-01…03 are non-vacuous, and the `Scan::Sequential` + fixpoint argument is sound. M4-R20 fails closed and
is non-vacuous; **the R19↔R20 worry that Sequential-scanned chains could exhaust the passes into a 400 does
not happen** — every chain converges in ≤ 2 passes. R21/R22/R23 all hold (MSRV `1.82` is the true floor; no
stale ROADMAP pointers remain).

**But the DoS class was not fully closed.** M4-R19 fixed the quadratic in *candidate generation*; masking
itself — `Vault::mask`'s right-to-left `replace_range` splice (`anonymizer.rs:138-143`) — is **still
O(n²)**, now in the **entity count**: each splice shifts the string tail, so *k* entities in an *n*-byte
field cost Θ(n·k). Measured through the real `mask_all`: a 13 MiB `content` field of small emails ≈ **7
minutes** of CPU, while the *same* 13 MiB as **one** email masks in **0.2 s** — that gap is why DOS-01…03,
which each pin a single entity, never saw it. Same unauthenticated path as M4-R19; `spawn_blocking` keeps
the async executor alive but the shared blocking pool still saturates. Fix: single-pass splice (O(n)) +
**DOS-04** (a many-entity guard). Full record: [`reviews/M4.md`](reviews/M4.md#m4-r24).

## 2026-07-13 — M4-R19/R20/R22/R23: a safety fix has a cost, and the cost is part of the fix

**M4-R19…R23 closed** *(but see Review 10 above — M4-R24 later reopened M4).*

**M4-R19 (BLOCKER) — the fix for M4-R17 was a denial of service.** Making candidate generation see *every*
overlapping match meant resuming the regex one `char` past each match's **start** — O(n) start positions.
Fine while a match is **bounded** (a card is ≤ 19 digits), but the two patterns with **no length bound** —
`Email` (`[…]+@[…]+`) and `Secret` (`sk-[…]{6,}`) — re-matched an O(n)-long value at every one of them:
**O(n²)**. A ~1 MB `content` field, far under the 16 MiB body limit, pegged a core for **minutes** on an
**unauthenticated** path (151 s at 200 KB; masking runs *before* any upstream auth). A proxy that is down
forwards nothing — and protects nothing. **Availability is a privacy property here.**

The fix is a `Scan` enum on `Recognizer`, decided by one property of the pattern — *is its match length
bounded?* The ten bounded patterns keep the rescan (`Scan::Overlapping`, O(n·L), linear — **and the M4-R17
repro is a `CreditCard`, so it stays closed**); the two unbounded ones go back to plain `find_iter`
(`Scan::Sequential`).

**The hard part was proving that costs no coverage** — because shrinking a candidate set is *exactly* how
M4-R17 made PROP-03 pass vacuously, so this needed an argument, not an assertion. A same-recognizer match
that starts inside an earlier one is **contained** in it (both run greedily to the same word boundary), so
it adds no bytes — for `Secret` that is the whole story. For `Email` there is one shape that isn't
contained, a chained `a@b.com@c.com`, whose second email starts inside the first's *domain* and reaches
past its end. **The fixpoint catches it**: masking the first leaves `[EMAIL_1]@c.com`, and `mask_all`
re-detects until nothing is left, so any surviving `local@domain` is masked on the next pass
(`a@b.com+x@c.com` → `[EMAIL_1][EMAIL_2]`). What remains is a bare `@domain` — not PII (M4-R11). So the two
mechanisms turn out to be **complementary**: *bounded recognizers rescan; unbounded ones iterate.*

Measured (release, 1 MB inputs): email **15 ms**, secret **23 ms**, and the *bounded* card row **160 ms** —
and doubling N doubles the time (2.1 → 3.7 → 7.7 → 15.3 → 29.7 ms across N = 125 K → 2 M), so it is
**linear by measurement**, not just "fast". Masking also moved to `tokio::task::spawn_blocking`: it is
CPU-bound (regex scans, plus NER inference when on), and inline it could starve the executor. A panicking
stage now **blocks** the request — we'd be holding a body of unknown PII status.

New `tests/complexity.rs` (**DOS-01…03**) pins it. They are *timing* guards on a worker thread with a
wall-clock budget, so a quadratic regression fails in **seconds** rather than hanging for hours — and each
also asserts the value is still masked and round-trips, so a "fix" that buys speed with blindness fails
too. **Verified non-vacuous: on the pre-fix code DOS-01/02 time out while DOS-03 passes** — precisely the
bounded/unbounded split the finding predicted.

**M4-R20 — the fixpoint is now *confirmed*, not assumed.** Exhausting `MAX_MASK_PASSES` used to return the
text anyway, forwarding anything still un-masked — a fail-*open* in a fail-closed product. The reassuring
comment ("hitting it can only mean over-masking") was **unproven**: *"each pass strictly shrinks the
un-masked text"* buys **eventual** convergence, never convergence **within four passes**. `mask_all` now
runs one final `try_detect`; anything still detectable → `Err` → `PrivacyStage` blocks (400). No cost on
the normal path (a converging text — every real one, ≤ 2 passes — runs exactly as many detections as
before). Guarded by a synthetic `NeverConverges` detector.

**M4-R21 — closed as *not a bug*.** `mask_all` runs the detector ≥2× (~2× NER inference). That second pass
**is** the fail-closed confirmation above, so it is a deliberate correctness cost, not an oversight.
Carried into M5's PERF-01 as a *measurement*.

**M4-R22/R23 — the small ones.** `rust-version = "1.82"` declared (`Option::is_none_or` sets the floor;
M5's CI would have discovered it by failing), and the four code comments left dangling by the docs refactor
now point at `docs/reviews/` anchors instead of ROADMAP sections that no longer hold the explanation.

**125 tests green (default) / 133 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R16/R17/R18: an invariant is only as strong as the set it quantifies over

Sixth review of this area. The theme this time is **the limits of the guard itself**.

**M4-R17 — `find_iter` hid values from the invariant.** `Regex::find_iter` is
leftmost-**non-overlapping**: after a hit it resumes at the match's *end*. A real value that **starts
inside** an earlier match of the *same* recognizer is therefore never emitted as a candidate at all — so
PROP-03 ("every raw candidate is covered") was satisfied **vacuously** for it. The resolver was fine; it
simply never learned the value existed.

```
4111 1111 1111 1111@123-45-6789-4111 1111 1111 1111
                        └─ the shifted window `6789-4111 1111 1111` is Luhn-valid and matches first,
                           so the REAL trailing card (which begins inside it) was never a candidate
                        →  masked: [CARD_1]@[CARD_2] 1111   ← a card digit group in clear
```

Fixed where the finding says the cause is — **candidate generation**: each recognizer now resumes one
`char` past a match's **start**, and its overlapping hits are coalesced into maximal runs (bounded
candidate set on pathological input, same coverage). *Note: the reviewer's suggested "re-scan the
uncovered gaps" would **not** have closed this repro — the gap is ` 1111`, and the hidden card starts
inside the already-covered region, so no gap re-scan can surface it.*

**And then PROP-04 found a live leak on its first run.** The reviewer asked for a companion property —
"re-running the detector on the masked output must yield nothing" — and it immediately failed:

```
4111111111111111555 867 5309   → one 19-digit run: NOT Luhn-valid, so the card is correctly not a
                                 candidate (an ID never fires inside a longer token)
4111111111111111[PHONE_1]      → masking the phone SPLIT the run, exposing a clean, Luhn-valid card
                                 that would go upstream IN CLEAR
```

**Masking rewrites the bytes around what it replaced, and a value is only recognizable in context.** So
masking now runs to a **fixpoint** (`Vault::mask_all`, wired into `PrivacyStage`): re-detect until the
text yields nothing. It converges because a placeholder is inert (no recognizer can match `[KIND_N]` or
span across it), and the round-trip stays exact. This is the deeper form of the same lesson as the
union-merge: *the property you assert must quantify over something the bug can't hide behind.* PROP-03
quantifies over the **candidate set**; PROP-04 quantifies over the **output bytes**. Only the second one
could have caught this.

**M4-R16 — the ASCII blind spot was still in the guard.** The corpus grew `non_ascii_scripts` at M4-R13,
but **PROP-03's own tables were still 100% ASCII** — the exact blind spot that let M4-R13 survive four
reviews, sitting in the one test the whole no-abandoned-bytes guarantee rests on. Added CJK / Cyrillic /
accented glue and non-ASCII-context samples, so the invariant is exercised on multi-byte input.

**M4-R18 — the code still described the deleted containment gate.** Five comments and two test names
still called it a deletion. Renamed and rewritten to the naming-rule mechanism — worth the churn,
because "the gate deletes the enclosed span" is precisely the mental model that produced the M4-R10 leak.
**119 tests green (default) / 127 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R13/R14/R15: the recognizers were **inert in Chinese** (Unicode `\b`)

The fifth review of this area, and it found the worst leak yet — not in the overlap code everyone had been
staring at, but in a single character of every regex.

**M4-R13 (blocker).** Rust `regex`'s `\b` is **Unicode-aware**: a Han / Kana / Cyrillic letter *is* a word
character, so there is **no word boundary between a CJK character and a digit**. Chinese and Japanese have no
inter-word spaces, so the glued form is the **natural** way to write it — not an evasion. Every `\b`-anchored
structured recognizer was therefore **completely inert** in CJK prose:

```
我的信用卡号是4111111111111111    → matched NOTHING — 16 Luhn-valid card digits upstream in clear
我的身份证号是11010519491231002X  → the zh Resident ID pack we shipped in M4 never fired, in Chinese
密钥sk-abcdef123456              → API secret in clear;  账号DE89370400440532013000 → IBAN in clear
カード番号は4111111111111111です  → card in clear;      Карта4111111111111111      → card in clear
```

The same values mask the instant a space is inserted, which pins the cause exactly. This is squarely inside the
**declared M4 domain**: `zh` is one of the ten declared languages and ARCHITECTURE claims "structured PII is
language-independent". Fix: `(?-u:\b)` on all 12 anchored recognizers. The anti-FP guarantee is preserved
**exactly** (an ID still can't fire inside a longer *ASCII* token — `card4111111111111111`, hashes, base64);
only a non-ASCII *letter* stops counting as part of a number. `Email`/`Phone` were never affected (character
classes, not `\b`).

**Why it survived four reviews — the real lesson.** `tests/corpus/pii_cases.json` contained **zero non-ASCII
characters**, and M4's "validate across the declared domain" pass validated the **NER** on zh — never the
*structured* recognizers. The test suite was ASCII-shaped, so an entire class of total failure was invisible to
it. Every review, mine included, kept auditing the code we had tests for. Added a `non_ascii_scripts` corpus
category (CJK/JA/Cyrillic positives + ASCII-token negatives) and non-ASCII round-trips. **Non-vacuity:**
restoring the Unicode `\b` makes the detector return `[]` on the Chinese card sentence and makes corpus CJK-01 +
RT-05 fail with *"PII must be masked"*.

**M4-R14 (hardening).** The merge's one fallback degraded *toward* the leak class it exists to prevent: if the
union re-slice failed it returned the **winning candidate alone**, abandoning the other constituents' bytes. Now
the union is first **widened out to enclosing `char` boundaries** (widening only ever *adds* bytes, so it can't
abandon a constituent), which makes the slice total; the remaining unreachable arm returns the **whole group
unmerged**, with a `debug_assert!` + kind-only `warn!`. Never a panic — this is a proxy on attacker-influenced
input.

**M4-R15 — and it deleted the containment gate.** The union was named by whichever candidate *survived*, so a
`Secret` enclosed by an email came out as `[PHONE_1]`: no leak, but the model is told the blob is a phone and the
audit log under-reports a secret. The fix is to name the union by the highest-priority **raw** candidate it
covers — and doing so revealed that **the gate was never needed**: it never affected a *span* (an enclosed span,
if not deleted, simply merges *into* its enclosing email → identical union), only the *label*. So
`drop_spans_contained_in_an_email` is **gone**, replaced by a naming rule in `name_of` — highest-priority raw
candidate, **except** when the union is exactly an `Email` span (a genuine email whose local part merely looks
like a card/ID keeps the `Email` label, preserving M4-R7/R9). Deleting nothing also **structurally removes the
M4-R10 trap**: with no deletion, no span can be stranded. Less code, same behaviour, one fewer booby trap.
**115 tests green (default) / 123 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R10/R11: the resolver gets an *invariant* instead of a ranking (union-merge)

Third review on the same overlap code, and the one that found the **root cause** the previous two fixes
kept dancing around. Worth recording, because the lesson is bigger than the bug.

**The pattern.** M4-R7 made `Email` win an overlap; the card leaked. M4-R9 made the structured span win;
the **email** leaked. Both fixes were "correct" against their own test — and both were wrong, because they
were tuning *which side of a partial overlap gets abandoned*. The root cause was never the priorities:
`resolve_overlaps` settled an overlap by **dropping the whole loser span**, so the loser's bytes were left
**in clear**. A flat `priority()` scalar can only express *"one of them wins"*; it cannot express *"both must
be masked"*. Two leaks the reviewer reproduced through `Vault::mask`:

```
555 867 5309john.doe@example.com      → [PHONE_1]john.doe@example.com     (M4-R11: a deliverable email in clear)
555 867 5309.4111111111111111@x.com   → [PHONE_1].4111111111111111@x.com  (M4-R10: 16 card digits in clear)
```

M4-R10 is the nastier one and it was **introduced by my own M4-R9 gate**: the gate deletes a span contained in
an email *before* the priority sort — but the containing email was **not guaranteed to survive** that sort. A
third span partially overlapping the email dropped the email too, so the already-deleted card was masked by
**nothing at all**. (`Secret` is the sharpest case: the highest-priority kind, deleted before priority ever ran.)

**Fix — replace the ranking with an invariant:** *no structured span's bytes are ever abandoned.*
- **Structured union-merge** — two overlapping structured spans now collapse into their **union** (labelled by
  the highest-priority kind) instead of one being dropped. One sort-by-start sweep reaches the fixpoint. The
  union is masked as one placeholder and restored verbatim, so the round-trip stays exact; it over-masks a
  little (a bare `@domain` can land inside the placeholder) — the project's stated direction: over-mask, never
  leak.
- **The containment gate stays and is now provably safe:** an email is never *dropped*, only absorbed into a
  union covering at least its own span, so a span the gate deleted can no longer be stranded.
- **NER keeps the whole-span drop** (M2-R7) — a lost `Person` remainder costs recall, never a leak.
- `resolve_overlaps` now takes `input` (the union's `text` must be re-sliced from the source: `Vault` keys on
  `entity.text` and splices by `span`). `PiiKind::priority` now ranks **labels, not survivors** — a lower
  priority can no longer cost coverage.

**The test that should have existed from the start (PROP-03).** `every_structured_candidate_byte_is_covered`:
glue PII values (incl. the grouped shapes) in arbitrary orders, then assert **every raw structured candidate is
fully covered by some resolved span**. A per-byte invariant *cannot* be satisfied by picking a winner — which is
exactly why the two priority-only fixes each passed their hand-written case while leaking the other side. Proof
it bites: with the union-extension disabled it **independently rediscovered** the M4-R11 leak, shrinking straight
to `555 867 5309john.doe@example.com` ("Email at 8..32 is left in clear"). All 7 M4-R10/R11 tests fail under that
probe. **110 tests green (default) / 118 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R9: containment-gate the Email priority (fixes a leak my M4-R7 introduced)

The review caught a **real leak in my own M4-R7 fix**, and it's worth recording *why* the reasoning
failed. M4-R7 made `Email` the top structured priority, justified by: "no other structured kind carries
`@`, so a card/IBAN/secret can only overlap an email by being a **substring of its local part**." That
claim is true only for the **continuous** forms my test covered. The email local-part class
`[A-Za-z0-9._%+-]` **excludes the space** — so against a *space-grouped* card or IBAN glued to a domain,
the email match forms from **only the trailing group** and merely **partially overlaps** the structured
span. `resolve_overlaps` drops the *whole* lower-priority span, so a top-priority `Email` discarded the
entire card:

```
card 4111 1111 1111 1111@example.com   →   card 4111 1111 1111 [EMAIL_1]   ← 12 card digits IN CLEAR
iban DE89 3704 0044 0532 0130 00@…     →   iban DE89 3704 0044 0532 0130 [EMAIL_1]
```

A **regression**: pre-M4-R7 the card won and masked whole. Lesson: "kind X can never contain `@`" bounds
*containment*, not *overlap* — I generalized from the shape my test happened to use.

**Fix — the recommended containment gate (not the minimal revert), so M4-R7's benefit survives.** Two
complementary mechanisms:
- **`drop_spans_contained_in_an_email`** in `resolve_overlaps`, run **before** the priority sort: a
  structured span lying **entirely inside** an `Email` span is a false decomposition of its local part →
  dropped, so the email wins (`4111111111111111@x.com`, `123456789@x.com`). Partial overlaps are untouched.
- **`Email` moved to the lowest structured priority** (below `Phone`, still above NER): every *partial*
  overlap now falls through to priority, where the checksum-backed structured span wins — digits masked,
  never fragmented.

Containment → Email wins; partial overlap → structured wins. This also closes the space-grouped **NINO**
(`AB 12 34 56 C@x.com`), which was a latent leak even *before* M4-R7 (`Email` already outranked
`NationalId`). New `PiiKind::is_structured()` backs the gate. **Tests:**
`grouped_forms_attached_to_a_domain_do_not_leak` (recognizers), `grouped_pii_glued_to_a_domain_leaks_nothing`
(`tests/adversarial.rs` — asserts on the **masked body** that no card/IBAN/NINO group survives in clear),
and the two resolver units. **Verified non-vacuous** by re-raising `Email` above the structured kinds: they
fail. **103 tests green (default) / 111 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4 review follow-ups closed: M4-R6 / M4-R7 / M4-R8

A review session opened three non-blocking precision follow-ups on the completed M4 (all fail-safe —
over-mask/utility, never a leak). Closed all three. **99 tests green (default) / 107 + 1 `#[ignore]`d
(`--features onnx`), no warnings** on both profiles.

- **M4-R6 — accepted-FP tradeoff on the pure-numeric IDs (documented + pinned).** `\b\d{9}\b`
  (BSN ∪ NIF) and `\b\d{11}\b` (DE ∪ LV) over-mask a fraction of ordinary numbers on checksum alone
  (~2/11 ≈ 18% of arbitrary 9-digit tokens; the LV `32…` form adds an unconditional ~1% at 11 digits).
  Resolved by **documenting** the accepted magnitude (code comments on both recognizers, like the LV
  shape-only note) — deliberately **not** context-gating: gating a national ID on a nearby keyword would
  reintroduce leaks and contradict the always-on M4-R1 decision. The clean precision path is the
  contextual **GLiNER** detector (Backlog). Test `bare_numeric_national_ids_are_masked_by_design`:
  `524287244` (an arbitrary PT-NIF-valid number) is masked by design; `524287245` (fails both checksums)
  is left in clear — so it's not a blanket "mask every number".
- **M4-R7 — Email is now the top structured priority (generalized the substring fix).** The earlier
  Email>national-ID reorder is generalized to Card/Iban/Secret: `PiiKind::priority` now ranks
  **Email > Secret > Iban > CreditCard > Ssn ≈ NationalId > Phone**. An `@`-token that parses as an email
  *is* an email, and no other structured kind carries `@`, so a card/IBAN/secret can only overlap an email
  by being a **substring of its local part** — there the whole email is correct and must win, else the
  `@domain` forwards in clear (`4111111111111111@x.com` → previously `[CARD_1]@x.com`). Non-email spans
  never share this tier, so lifting Email regresses nothing outside the containment case (confirmed: no
  corpus/adversarial/proptest change). Test `email_beats_a_card_iban_or_secret_local_part`.
- **M4-R8 — DE Steuer-ID consecutive-triple exclusion.** `de_steuerid_valid` now enforces the 2016+ rule:
  a digit appearing three times in the first 10 must **not** occupy three *consecutive* positions. Pure
  precision gain (rejects a look-alike), zero recall cost (a valid ID never has one). Self-verifying test
  `de_steuerid_rejects_a_consecutive_triple` (same digits + valid checksum: consecutive → rejected,
  non-consecutive → accepted). **M4 review follow-ups done.** Next: M5.

## 2026-07-13 — Relicensed MIT → AGPL-3.0-or-later; version 0.1.0 → 0.4.0

The project is gaining traction, so it moved from MIT to the **GNU Affero GPL v3-or-later**
(`AGPL-3.0-or-later`) — a network-copyleft that keeps the ecosystem open: anyone running a *modified*
version **as a service** must share their changes, which fits a privacy proxy served over the network
(running it unmodified carries no obligation). `LICENSE` is the **official FSF text** (fetched from
gnu.org, byte-exact); `Cargo.toml` `license`, both READMEs (EN/IT), and ROADMAP M0 updated. No per-file
source headers added yet (optional, can follow). Version bumped **0.1.0 → 0.4.0** to reflect M0–M4;
**`1.0.0` is reserved for the first tagged release** (the M5 CI/release pass). No functional code changed.

## 2026-07-13 — M4 COMPLETE: all national-ID packs, CF/FR checks, IBAN per-country, provider-agnostic

Closed every remaining M4 item. All tests green (default + `--features onnx`), no warnings.

- **National-ID packs for all XLM-R-aligned countries** (always-on, checksum-gated): added **DE**
  Steuer-ID (ISO 7064 Mod 11,10 + the one-repeated-digit structural rule), **NL** BSN (11-proef) / **PT**
  NIF (mod-11) behind one 9-digit recognizer, **LV** personal code (classic mod-11 + post-2017 `32…`
  shape-only form), **zh** China Resident ID (ISO 7064 MOD 11-2, 18 chars). `ar` gets no pack (no single
  Arabic national ID). Each validator hand-checked against an official test number.
- **M4-R3 — IT Codice Fiscale check character** (`cf_check_valid`, odd/even table + mod-26): a
  wrong-checksum look-alike is now rejected, consistent with the other national IDs.
- **M4-R5 — FR NIR completeness**: the month alternation admits the INSEE special codes (`20`, `30–42`,
  `50–99`) so those real NIRs aren't missed on the always-on tier (mod-97 key still gates precision).
  Corsica `2A`/`2B` documented as a known limitation.
- **IBAN per-country length**: `confidence_of` tags an IBAN `Verified` only when mod-97 **and** its
  country's ISO 13616 fixed length both check out (`iban_country_length` table); otherwise `Structural`
  (still masked). Unknown countries rely on mod-97 alone.
- **Overlap priority fix (found by proptest).** The new pure-numeric recognizers (`\d{9}`, `\d{11}`,
  18-digit) can match a numeric **email local part** (`123456789@x.com`), and `NationalId` (then priority
  3) out-ranked `Email` (2) → the email got fragmented and PROP-01 failed. Fix: reordered
  `PiiKind::priority` to **Secret > Iban > Card > Email > Ssn ≈ NationalId > Phone** — a national ID never
  *is* an email, so the email (the complete match) must win the substring overlap. This also fixes a
  latent SSN-in-email case. Guard: `numeric_email_local_part_is_not_hijacked_by_a_national_id`.
- **Provider-agnostic verification**: `e2e_masking_is_provider_agnostic` — the same request via the
  `openai` vs `anthropic` presets yields a byte-identical masked body upstream (masking is schema-based;
  presets only affect routing). **M4 is done.** Next: M5 (integration & performance testing).

## 2026-07-13 — M4 continued: national IDs always-on + ES/FR packs + 10-language NER validation

Advanced M4 per the decided three-tier scope. **86 tests green (default), 94 + 1 `#[ignore]`d
(`--features onnx`), no warnings.**

- **M4-R2 (tighten GB NINO).** Added a `validate` fn with the official prefix rules (1st letter
  ∉ D/F/I/Q/U/V; 2nd ∉ D/F/I/O/Q/U/V; invalid pairs BG/GB/KN/NK/NT/TN/ZZ), so the shape regex no
  longer masks look-alikes (`PO123456A`, `GB…`, `DA…`). Prerequisite for always-on.
- **M4-R1 (three-tier structure).** National-ID recognizers now run **always**, independent of
  `PII_LOCALES` (privacy-first — a national ID that reaches the proxy is masked even if its country
  isn't configured). Split into `national_id_recognizers()` (always-on) and `fp_prone_recognizers(code)`
  (opt-in via `PII_LOCALES` — empty seam for future national *phone* formats). `PII_LOCALES` now gates
  only *ambiguous* recognizers, not "which countries".
- **National-ID packs (always-on, checksum-specific).** ES DNI/NIE (mod-23 check letter, NIE X/Y/Z →
  0/1/2) and FR NIR (15 digits + mod-97 key). DE Steuer-ID **deferred** (needs the ISO 7064 check +
  structural rules to hit near-zero FP as an always-on recognizer). Tests: check-letter / key validators
  + detection (a wrong-check look-alike is not masked).
- **NER validated across its declared 10-language domain.** Added `multilingual_preview` cases for
  ar/es/fr/lv/nl/pt/zh (de + en/it already present) — one Person + Location each — and scored the picked
  XLM-R int8 through the hybrid via the `#[ignore]`d harness:

  | kind | recall | prec | F1 |
  |---|---|---|---|
  | Person | 0.83 | 0.71 | 0.77 |
  | Organization | 1.00 | 1.00 | 1.00 |
  | Location | 0.91 | 0.91 | 0.91 |

  The five added Latin-script European languages (es/fr/pt/nl/lv) match **cleanly**; ar/zh find the
  names/cities but with a minor boundary artifact (a preposition token — Arabic `ب`, Chinese `在北京`).
  Confirms the model genuinely covers its declared domain; structured PII remains authoritative and the
  NER stays fail-open best-effort. **Remaining M4:** more national-ID packs, FP-prone locale phone
  formats, IBAN per-country checks (all documented in ROADMAP).

## 2026-07-12 — M4 first landing: locale-parametrized recognizers + national IDs (IT/GB)

Started M4 (broad locale coverage) with the recognizer-architecture change that is its
barycenter. **83 tests green (default), 91 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Locale-parametrized recognizers.** `StructuredRecognizers` split into `universal_recognizers()`
  (email, secret, credit card, IBAN — already any-country — and phone US/`+CC`) plus
  `locale_recognizers(code)` national-identifier packs. New `with_locales(&[codes])` (kept `new()` =
  default `it, us`, backward-compatible). Active locales come from **`PII_LOCALES`** (default `it,us`,
  on `Config.pii_locales`), threaded into `server.rs::build_detector`.
- **National IDs (new `PiiKind::NationalId`, placeholder `[NATID_N]`).** IT **Codice Fiscale**
  (`[A-Za-z]{6}\d{2}[A-Za-z]\d{2}[A-Za-z]\d{3}[A-Za-z]`) and GB **NINO** (compact + space-grouped) —
  both deliberately specific (interleaved letters/digits) for a low false-positive rate. `PiiKind`
  gained the variant + `label`/`priority`(national-ID tier = 3, shared with SSN)/`from_label`.
- **Tests.** `italian_codice_fiscale_detected_by_default`, `uk_nino_needs_the_gb_locale` (incl. that it
  is *off* without the GB locale), `locale_selection_is_scoped` (a US-only set ignores a CF).
- **Deferred (documented in ROADMAP M4):** more locale national-ID packs (ES/FR/DE), locale phone
  *national* formats (FP-prone without a `+CC` anchor), IBAN per-country length checks, and validating
  the already-multilingual XLM-R against a wider corpus. **Next: continue M4 (widen the locale seam).**

## 2026-07-12 — M3-R2: JSON-aware de-mask for tool-call arguments

Fixed a pre-existing correctness bug surfaced by the M3 review. **80 tests green (default),
88 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Problem.** `Vault::demask` did a plain string substitution `[KIND_N]` → raw value. In a
  **JSON-encoded string** field — `tool_calls[].function.arguments` / legacy
  `function_call.arguments` — a value containing a `"`, `\`, or control char produced **invalid
  inner JSON**, so the client couldn't parse the tool-call arguments. Not a leak (client-side;
  request masking unaffected), but a real correctness gap on both the buffered and streaming paths.
- **Fix.** Added `Vault::demask_json_string`, which substitutes the **JSON-string-escaped** value
  (`json_string_body` = `serde_json::to_string` minus the outer quotes). Wired it into the
  `arguments` fields only: buffered `demask_response` (`src/pipeline/privacy.rs`) now uses it for
  `tool_calls`/`function_call` args, and streaming `SseDemasker::demask_for` picks it for
  `StreamKey::ToolArg`. `content` keeps the plain `demask` (it's not a JSON string).
- **Tests.** `demask_json_string_keeps_inner_json_valid` (vault, incl. asserting plain demask *would*
  break it); `tool_call_arguments_demask_stays_valid_json` + `content_demask_is_not_json_escaped`
  (buffered); `tool_call_arguments_deanon_stays_valid_json` (streaming). **Next: M4.**

## 2026-07-12 — M3 close-out: tool-call-arg streaming de-anon, M3-R1 fallback, SSE error events

Closed the remaining M3 follow-ups + the M3-R1 review nit (request-level routing stays deferred,
per user). **76 tests green (default), 84 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Streaming de-anon of tool-call arguments.** `SseDemasker`'s hold-back buffers are now keyed by
  field — `StreamKey::Content { choice }` **and** `StreamKey::ToolArg { choice, tool }` — so a
  placeholder split across streamed `delta.tool_calls[].function.arguments` deltas is reassembled
  and de-masked, not just `delta.content`. `flush_pending` synthesizes the right chunk shape per key.
  Test `tool_call_arguments_split_across_deltas_are_restored`.
- **M3-R1 — non-SSE fallback.** `stream_chat_completions` branches on the upstream response
  `content-type`: when it isn't `text/event-stream` (a JSON 401/429, or a provider that ignored
  `stream`), it falls back to `buffered_fallback` — forward the real status + content-type and run
  the `on_response` de-mask — instead of forcing an event-stream. Test
  `e2e_streaming_non_sse_error_falls_back_to_json` (429 `application/json` reaches the client intact).
- **Terminal SSE error events.** A mid-stream upstream error is now turned into a terminal
  `event: error` (after flushing buffered content) and the stream ends cleanly, rather than aborting
  the client connection. `demasking_sse_body` was made generic over the stream error type so it is
  unit-tested with a synthetic erroring stream (no HTTP): `mid_stream_upstream_error_becomes_terminal_sse_event`.
- **Deferred (unchanged):** request-level provider routing — per-instance today; documented in ROADMAP.

## 2026-07-12 — M3: SSE streaming de-anon + multi-provider routing (Option A)

Streaming and provider routing landed together (real Copilot/Anthropic usage is streamed).
**73 tests green (default), 81 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Streaming (SSE) with incremental de-anon** (`src/stream.rs`, new). `stream:true` is now
  forwarded (no more 400): the handler masks the request as usual, then streams the response
  back through `SseDemasker`, which parses `data:` lines and de-anonymizes each
  `choices[].delta.content`. A placeholder can be **split across two token deltas**
  (`[EMA` + `IL_1]`), so it keeps a per-choice **hold-back buffer** — `split_demaskable`
  finds the last point that could still be an incomplete placeholder, emits everything before
  it, and holds the tail until the next delta (or stream end) closes it. `[DONE]` and non-data
  lines pass through; a clean request (nothing masked) or `PII_DEBUG_SKIP_DEMASK` streams
  through untouched. **Fail-closed intact:** request-side masking runs first, so the provider
  only ever sees placeholders — streaming de-anon is a client-side usability step, never a
  privacy gate. `server.rs` builds the response body with `Body::from_stream` over a
  `futures_util::stream::unfold` adapter (new dep `futures-util`).
- **Multi-provider routing — Option A** (`config.rs` + `proxy.rs`). `UPSTREAM_PROVIDER`
  (`openai`/`copilot`/`anthropic`) selects a preset for the per-provider *shape*: chat path
  (`upstream_chat_path` — Copilot drops `/v1`), a client-header passthrough allowlist
  (`forward_request_headers` — e.g. `anthropic-version`, Copilot editor headers), and static
  `upstream_extra_headers`. All overridable (`UPSTREAM_CHAT_PATH` / `UPSTREAM_FORWARD_HEADERS`
  / `UPSTREAM_EXTRA_HEADERS`); base URL + key stay env-driven. `Upstream` gained a raw `send`
  (used by streaming) plus the configurable path/headers; `Config::Debug` redacts extra-header
  values (may be secrets).
- **Tests.** `stream.rs` units (hold-back split, split-placeholder reassembly, mid-line byte
  splits, passthrough) + e2e `e2e_streaming_deanonymizes_split_placeholder` (client gets the
  real value from a `[EMAIL_1]` split across SSE events; the upstream saw only the masked body).
- **Deferred (documented, not leaks):** streaming de-anon of `delta.tool_calls[].function.arguments`
  (streamed tool args currently pass through), request-level provider routing, and terminal
  SSE error events. See ROADMAP M3 follow-ups. **Next: M4 (broad locale coverage).**

## 2026-07-12 — M2.5-R1 / M2.6 review nits: footprint guard, log-safety test, env_flag dedup

Closed the last M2.5/M2.6 review items. **68 tests green (default), 76 + 1 `#[ignore]`d
(`--features onnx`), no warnings.**

- **M2.5-R1 builder tasks (decision: Option A — keep `hf-hub 1.0`).** Corrected the three
  inaccurate "no new native deps" claims (`Cargo.toml` comment, this log's M2.5 entry, the
  ROADMAP M2.5 dep bullet) to the real onnx-only footprint — under `--features onnx` hf-hub 1.x
  pulls a second `reqwest 0.13` + `hf-xet` + `rustls`/`aws-lc-rs`. Added **`tests/dependency_footprint.rs`**:
  a `cargo tree` guard that fails if the **default** build ever pulls
  `hf-hub`/`hf-xet`/`aws-lc`/`ort`/`tokenizers` (they must stay `onnx`-gated). Verified
  non-vacuous — all five appear under `--features onnx`, none in the default tree.
- **Log-safety regression test (M2.6).** `tests/log_safety.rs` captures the crate's
  `trace`-level logs during a real PII round-trip and asserts the `trace!` masked-body log shows
  `[EMAIL_1]` and **never** the raw value, while the reply really did carry the de-masked value
  (so a leak would be caught). Turns the DBG-01 inspection rule into an automated guard.
- **`env_flag` de-dup.** One `pub(crate) config::env_flag`; `server.rs` imports it, so
  `PII_DEBUG_SKIP_DEMASK` / `NER_REQUIRED` / `NER_TOKEN_TYPE_IDS` share a single `1`/`true`/`yes`/`on`
  parser and can't diverge.

## 2026-07-12 — M2.5 review follow-ups (R1 parked, R2 fixed) + M2.6 debug modes

Closed the M2.5 review's R2 and the new M2.6 milestone; R1 investigated and parked.
**66 tests green (default), 74 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **M2.5-R1 (hf-hub footprint) — investigated, PARKED (user's call: no downgrade).**
  `cargo tree --features onnx` confirmed `hf-hub 1.0.0` pulls a **second `reqwest 0.13.4`**,
  **`hf-xet 1.5.3`**, `rustls 0.23` → **`aws-lc-rs`/`aws-lc-sys`** (native crypto),
  `reqwest-middleware`, and `ureq 3` — so the "no new native deps" claim is inaccurate.
  Verified in hf-hub 1.0.0's `Cargo.toml` that `hf-xet` + `reqwest 0.13` are **non-optional**
  (its only features are `blocking`/`rustls-tls`/`socks`/`default=[]`), so the DECIDED
  `default-features = false` trim is a **no-op** — the only way to shed them is a downgrade
  to the pre-Xet `hf-hub 0.4.3` (reuses the in-tree `reqwest 0.12`, no xet/aws-lc). The user
  chose **not** to pin an older API; parked for a reviewer session. **No code changed.**
  Details + the correction to-do recorded in ROADMAP M2.5-R1. The **default** build stays
  native-dep-free regardless (hf-hub is `onnx`-gated).
- **M2.5-R2 (fail-closed hygiene) fixed.** `parse_id2label` now `ensure!(!pairs.is_empty())`
  after the contiguity check, so an **empty** `id2label` fails closed at load instead of
  returning `Ok(vec![])` and only surfacing later (and only under `NER_REQUIRED`). Test
  `empty_id2label_is_an_error`.
- **M2.6 — debug & observability modes** (opt-in, off by default; neither weakens
  fail-closed — request-side masking always runs). (1) **`PII_DEBUG_SKIP_DEMASK`** on
  `Config.debug_skip_demask` (not a bare env read → isolated + testable): skips the response
  de-mask so the client sees the placeholders the provider saw; a **loud `warn!`** fires at
  startup. `chat_completions` guards the `on_response` loop on it. (2) **`trace!` of the
  masked upstream body** just before forwarding (masked → safe), `debug!` kept for the
  kind-only audit. **Safety boundary upheld:** the de-masked client output is never logged
  (only the `trace!` masked request + a body-less `debug!` on skip). Test
  `e2e_debug_skip_demask_returns_placeholders_to_client`; new `Config.debug_skip_demask`
  threaded through `spawn_proxy_cfg` in the e2e harness.

## 2026-07-12 — M2.5: HuggingFace model auto-download (`hf-hub`) + M2-R10 — COMPLETE

Model management is now library-managed and reproducible, and the last M2 harness
nit is closed. **65 tests green (default), 72 + 1 `#[ignore]`d (`--features onnx`),
no warnings.** Verified end-to-end against a live download.

- **M2-R10 (harness precision) closed.** `tests/ner_eval.rs` scored TP/FP/FN by
  `Vec::contains` (set membership), so a duplicate `(kind, text)` could inflate recall.
  Replaced with a `tally` helper that counts each `(kind, text)` as a **multiset**
  (`tp = min(expected, detected)`). Non-network test `tally_counts_duplicates_as_multiset`
  pins recall 0.5 (not 1.0) when two expected entities meet one detection. Recorded
  numbers were unaffected (no corpus case has a duplicate in one sentence), as predicted.
- **M2.5 — opt-in `hf-hub` auto-download** (`src/pii/hf.rs`, feature `onnx`). The model
  file is resolved in priority order in `server.rs::load_onnx_ner`: (1) explicit
  `NER_MODEL_PATH` (+ tokenizer + labels) — zero outbound calls, always wins; (2) when
  unset but `NER_MODEL_REPO` (`owner/name`) is set, `HfModelSpec::resolve` fetches a
  **revision-pinned** model (`NER_MODEL_REVISION` default `478a2a3`, `NER_MODEL_FILE`
  default `onnx/model_quantized.onnx`, `NER_TOKENIZER_FILE`, `NER_CONFIG_FILE`) into the
  standard HF cache. **`NER_LABELS` is derived from `config.json` `id2label`** (class-id
  order, contiguity-checked → fail closed) unless set explicitly — this removes the
  error-prone hand-typed 9-label list. `hf-hub` 1.0 uses the async API;
  `AppState::new`/`build_detector`/`load_onnx_ner` became `async` for the startup fetch.
  *(Correction, M2.5-R1: an earlier version of this entry claimed hf-hub "reuses the
  reqwest already in the tree — no new native deps". That is **inaccurate**: under
  `--features onnx` hf-hub 1.x pulls a second `reqwest 0.13` + `hf-xet` +
  `rustls`/`aws-lc-rs`. The **default** build is still native-dep-free. See the M2.5-R1
  entry above and ROADMAP M2.5-R1.)*
- **Standard-cache pin (real bug found by running it).** `hf-hub` 1.0 falls back to
  `/tmp/.cache/huggingface` on Windows when `HOME` is unset — a non-shared, drive-relative
  location (`C:\tmp\…`) that defeats the point. `build_client` now sets
  `cache_dir = <home>/.cache/huggingface/hub` (the `huggingface_hub` convention) when
  `HF_HOME`/`HF_HUB_CACHE` are unset, otherwise defers to them. The model now lands in
  `%USERPROFILE%\.cache\huggingface\hub`, deduped with every other tool. Unit-tested via
  `standard_hub_cache_dir` (no network).
- **Verified live + consolidated.** Ran the eval through the new download path: `hf-hub`
  fetched `jiting/xlm-roberta-base-ner-hrl_onnx@478a2a3` into the standard cache and the
  hybrid scored **Org 1.00 / Loc 1.00** and **Person 0.75/0.60/0.67 on the required M2
  corpus** — identical to the manual run (the cached blob is byte-for-byte the manual
  `model_quantized.onnx`, 278,709,677 B). *Note:* the harness's aggregate table also
  counts the DE `multilingual_preview` case ("Herr Müller" → the model returns "Müller",
  no title), which is labelled *not required at M2*; with it, the printed Person line is
  0.60/0.50/0.545. Model/detections unchanged — purely scoring scope.
- **Manual models removed** (user-authorized): deleted the hand-downloaded `ner-models\`
  (XLM-R + rejected Piiranha, ~600 MB) and a mislocated `C:\tmp\.cache` artifact from the
  pre-fix run (~564 MB); the only surviving copy is the `hf-hub`-managed one in the
  standard cache. ~1.16 GB freed.
- **Docs:** ROADMAP M2.5 ✅ + M2-R10 closed; ARCHITECTURE (model-management env contract +
  privacy note); SETUP (§4 — auto-download vs explicit-local). **Next: M3 (streaming).**

## 2026-07-12 — M2 model chosen (measured): XLM-R int8; R6 closed — M2 COMPLETE

Downloaded both locked candidates from the Hub and scored them **through the hybrid
resolver** on `tests/corpus/ner_cases.json` (8 cases incl. the DE non-ASCII preview),
per `docs/M2-NER-EVALUATION.md`. int8 (`model_quantized.onnx`), ORT CPU EP.

| Model | repo @ rev | Person R/P/F1 | Org R/P/F1 | Loc R/P/F1 | latency | size |
|---|---|---|---|---|---|---|
| **XLM-R** (winner) | `jiting/xlm-roberta-base-ner-hrl_onnx` @ `478a2a3` | 0.75 / 0.60 / 0.67 | **1.00 / 1.00 / 1.00** | **1.00 / 1.00 / 1.00** | ~23 ms/case | 266 MB |
| Piiranha | `onnx-community/piiranha-…-ONNX` | 0.00 | 0.00 (no ORG label) | 0.00 | ~37 ms/case | 302 MB |

**Pick: XLM-R int8.** Perfect Org/Loc; Person misses only the pathological single-token
"Caia" (tokenized `▁Cai`+`a`, tagged as two spans) and the "Herr" title on "Herr Müller".
Multilingual (10 langs incl. IT), drop-in (PER/ORG/LOC labels, no `token_type_ids`, no
label mapping). **Piiranha rejected:** ~0 recall on natural-sentence NER (it fires only
subword fragments — it's a form/structured-PII model) **and has no Organization label**.
GLiNER escalation not needed. (Piiranha's granular labels were wired anyway via an
extended `label_to_kind`; `type_vocab_size:0` → it needs no `token_type_ids`.)

- **R6 closed by the live run.** Confirmed non-ASCII "Müller" (2-byte `ü`) extracts on
  exact byte boundaries → the HF tokenizer emits **byte** offsets and the `&str`
  slicing is correct. Added a **whitespace trim** in `decode_entities`: SentencePiece
  includes the leading space in a token offset (`▁Mario` spans " Mario"), so the raw
  span was " Mario Rossi"; trimming shifts the span to the real content (masking now
  preserves the space, and the span text matches the value). This is what took recall
  from 0.00 (exact-text mismatch) to the numbers above. Test:
  `leading_space_in_token_offset_is_trimmed`.
- **`label_to_kind` broadened** to cover granular PII schemes (GIVENNAME/SURNAME → Person,
  CITY/STATE/COUNTRY → Location) while keeping structured categories (EMAIL/PHONE/…) →
  `None` (owned by the deterministic layer). Test updated.
- **To run the NER** (off-repo model, ~266 MB, not committed): download
  `onnx/model_quantized.onnx` + `tokenizer.json` from the XLM-R repo, then set
  `NER_MODEL_PATH` / `NER_TOKENIZER_PATH` / `NER_LABELS="O,B-DATE,I-DATE,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC"`
  and build `--features onnx`.
- **M2 COMPLETE.** 65 tests green (default), `--features onnx` builds + runs a live model
  clean, no warnings. Next milestone: M3 (streaming).

## 2026-07-12 — M2 review findings closed (8/9) + fail-closed NER + eval harness

Closed the M2 review (M2-R1…R9 except the model-gated R6), all with tests.
**64 tests green (default), `--features onnx` builds clean, no warnings.**

- **Fail-closed NER (R1/R2).** New `PiiDetector::try_detect(&str) -> Result<_,
  DetectError>` (default delegates to `detect`). `CompositeDetector` propagates a
  sub-detector error; `PrivacyStage::on_request` sets `ctx.block` (→ 400) when a
  *required* detector errors. `FailOpen(Box<dyn PiiDetector>)` opts a non-critical
  detector out (logs + empty). `NER_REQUIRED` drives it: `build_detector`/
  `AppState::new` now return `Result`, so a configured-but-unloadable required NER
  is fatal at startup; unset → old fail-open via `FailOpen`. `DetectError` carries
  only a static label, never input (R8).
- **Decode robustness.** `is_begin` accepts `B_`/underscore prefixes (R5, else
  adjacent same-type entities glue); `validate_label_count` rejects a
  `NER_LABELS`/model class-count mismatch instead of silently dropping entities
  (R3); `decode_entities` `warn`s (kind only) on an off-boundary offset rather
  than silently dropping a name (R6 mitigation — full non-ASCII verification stays
  open, needs a real tokenizer).
- **ONNX I/O (R4).** `outputs.get("logits")` → graceful `Err`, no panic;
  `token_type_ids` threaded when `NER_TOKEN_TYPE_IDS` is set (BERT-family, e.g.
  Piiranha); required input/output contract documented. `.lock()` recovers a
  poisoned session mutex (R9).
- **Overlap remainder (R7).** Documented + tested the deliberate choice: an NER
  span overlapping a kept structured span is dropped whole (structured never lost;
  only the non-overlapping unstructured remainder).
- **Eval harness.** `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d) scores a
  live model against `ner_cases.json` through the hybrid resolver
  (recall/precision/F1 per type + timing). Run:
  `cargo test --features onnx --test ner_eval -- --ignored --nocapture`.
- **Still open:** R6 (non-ASCII offset verification) + the measured model
  selection — both genuinely need a real model/tokenizer file.

## 2026-07-12 — M2 part 2: OnnxNerDetector behind the `onnx` feature (compiles)

The ONNX NER detector is implemented and **`cargo build --features onnx` is
green with no warnings** — `ort` 2.0.0-rc.12 (native ONNX Runtime, downloaded at
build) + `tokenizers` 0.23 both build under MSVC here. The default build stays
native-dep-free (feature off).

- **Pure decode** (`src/pii/ner_decode.rs`, **not** feature-gated, unit-tested on
  the default build): `label_to_kind` (strips `B-`/`I-`, maps PER/ORG/LOC + GPE),
  and `decode_entities` (BIO merge → char spans via token offsets; a `B-` label
  always starts a fresh entity so adjacent same-type entities don't glue). NER
  hits are tagged `Confidence::Structural`.
- **`OnnxNerDetector`** (`src/pii/onnx.rs`, `onnx` feature): HF fast tokenizer +
  a **pool of `ort` sessions** (round-robin, `NER_POOL_SIZE`) so inference isn't
  single-threaded (the M2 concurrency item). `detect` tokenizes with offsets,
  runs the session, argmaxes per-token logits (`num_labels = logits.len()/seq`,
  avoiding the Shape API), and hands off to `ner_decode`. ort's non-`Send`/`Sync`
  builder errors are converted to strings before entering `anyhow`.
- **Wiring** (`src/server.rs`): `build_detector()` composes the structured
  recognizers with the NER when the feature is on and `NER_MODEL_PATH` /
  `NER_TOKENIZER_PATH` / `NER_LABELS` (+ optional `NER_POOL_SIZE`) are set; a load
  error logs and falls back to structured-only.
- **Cargo**: `onnx = ["dep:ort", "dep:tokenizers"]`; `ort` pinned `=2.0.0-rc.12`
  with `download-binaries`; `tokenizers` `default-features = false` + `fancy-regex`
  (pure-Rust regex, no C regex backend).
- **Still pending (needs a model file):** the *measured* model selection (#2) and
  the fail-closed-on-NER-error decision (both noted in ROADMAP). No numbers are
  fabricated.
- **Tests: 57 green (default), no warnings; `--features onnx` compiles clean.**

## 2026-07-12 — M2 part 1: hybrid-detection infrastructure (model-independent)

Landed the M2 architecture that doesn't depend on a specific model, so the ONNX
model becomes a drop-in — implementing it *before* the measured model choice, per
`docs/M2-NER-EVALUATION.md` (measure, don't guess; don't fabricate an evaluation).

- **Shared overlap resolution** (`src/pii/overlap.rs`): extracted `resolve_overlaps`
  from the recognizers into a reusable function keyed on `PiiKind::priority()`.
  Structured PII (Secret>Iban>Card>Ssn>Email>Phone) outranks NER
  (Person/Org/Location = 0), so a deterministic email/IBAN always wins a span an
  ML guess overlaps. Recognizers now delegate to it (behaviour unchanged).
- **`CompositeDetector`** (`src/pii/composite.rs`): a `PiiDetector` that fans out
  to N detectors and merges their spans through the shared resolver — the hybrid
  seam. The server now builds one (structured-only today; the NER joins once
  wired). Tested with the real recognizers + a fake NER: merge works and the
  deterministic layer wins overlaps.
- **NER corpus** (`tests/corpus/ner_cases.json` + `tests/ner_corpus.rs`):
  labelled Person/Org/Location in IT+EN, single-word names (Tizio/Caia), REG-03
  negatives (`anubi` must not be a Person), a DE multilingual preview. Positive
  *recall* is measured once a model lands; the enforced-now guard is that the
  **deterministic layer never emits an unstructured entity**.
- **Symmetric response de-masking** (M1.5-review micro): `demask_content` now
  mirrors `mask_content`, restoring a bare-string element in a response content
  array too.
- **Tests: 53 green, no warnings.**
- **Next (M2 part 2):** `OnnxNerDetector` behind the `onnx` feature (`ort` +
  `tokenizers`, CPU EP, session pool for concurrency, pure BIO-decode unit-tested)
  — then the measured model selection with real files.

## 2026-07-11 — M1.5 code-review follow-ups closed

Three follow-ups from the M1.5 review (all in code from the previous session):

- **Array-content fail-closed gap** (`src/pipeline/privacy.rs`, `mask_content`):
  a content-array element that wasn't an object was silently skipped — a leak for
  a bare-string element carrying PII. Now: bare strings are masked, object parts
  have their `text` masked (non-text parts like `image_url` skipped), and any
  other element (number/bool/null/nested array) **fails closed**. Consistent with
  the top-level content rule.
- **Phone international over-match** (`src/pii/recognizers.rs`): the open
  `\+\d{1,3}(?: \d{2,7}){1,4}` swallowed a trailing number group
  (`+39 333 0000001 12345` → masked `12345` too). Replaced with two canonical
  shapes (three-group `+39 333 000 0001`, two-group `+39 333 0000001`), tried
  three-group first — same fix pattern as the IBAN over-match.
- **`Confidence` was write-only** (`src/pii/mod.rs` / `recognizers.rs`): now
  consumed — `detect` emits a `debug` audit log for each `Structural` match (KIND
  only, never the value). Field is read; richer use (audit sink, ML thresholds)
  deferred to M2.
- **Tests: 46 green, no warnings.** +`content_array_*` pipeline cases,
  +`phone_international_span_stops_at_the_number` adversarial case.
- **Next: M2** — ONNX NER.

## 2026-07-11 — M1.5: robustness & fail-closed (+ M1 code-review fixes)

Hardened M1 so the proxy fails *closed*, and folded in the M1 code-review items.

- **Fail-closed pipeline.** `RequestContext` gained a `block: Option<String>`. The
  privacy stage sets it on an unreadable `content` shape (bare object/scalar) or a
  missing/!array `messages`; the handler returns 400 and never forwards. Unproxied
  paths now 404 via a router `fallback` (only chat/completions + healthz are in
  scope). Masking still runs before forwarding, so nothing leaks on later errors.
- **Full field coverage** (`src/pipeline/privacy.rs`): added `messages[].name`,
  legacy `function_call.arguments`, `tools[].function.description`, and recursive
  `description`s inside `tools[].function.parameters`; content-array parts now mask
  any part carrying a `text` string (robust to new part types). One shared vault →
  a value split across fields still collapses to one token.
- **Tolerant de-mask** (`src/pii/anonymizer.rs`): replaced the two-pass
  contains+replace loop (code-review CR-4) with a single regex pass that also
  tolerates model corruption — `[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]`. An
  unresolved but known-kind placeholder is `warn!`-logged, never silently shipped.
- **IBAN over-match fixed** (code review): the regex now matches the two canonical
  IBAN shapes (continuous / space-grouped-in-4s) instead of "optional space before
  any char", so `IBAN IT60…456 EUR` no longer swallows `EUR`. Corpus `IBAN-04` +
  unit test guard it.
- **`iban_mod97` wired** (code review): used in `confidence_of` — a mod-97-valid
  IBAN is `Verified`, a structure-only one `Structural` (still masked). New
  `PiiEntity.confidence` + `Confidence` enum + `PiiKind::from_label`.
- **Single system message** (code review): augmentation now merges into an existing
  `system`/`developer` message instead of inserting a duplicate.
- **Response headers forwarded** (code review): safe allowlist (`retry-after`,
  `x-request-id`, `x-ratelimit-*`, `openai-*`, `anthropic-*`); content/hop-by-hop
  dropped. `forward_chat_completions` now returns the upstream `HeaderMap`.
- **Body-size limit**: `MAX_BODY_BYTES` (default 16 MiB) via `DefaultBodyLimit`.
- **Broadened phone recognizers** (evasion recall): `(555) 867-5309`, `555.867.5309`,
  `+1 …`, extra Italian grouping. Obfuscated emails documented as an accepted gap.
- **Tests: 43 green, no warnings.** New `tests/adversarial.rs`; corpus `PHONE-03..05`,
  `IBAN-04`; pipeline INT-07 + coverage/fail-closed/split-field cases; e2e header/
  404/fail-closed cases; lib demask-tolerance + IBAN-span + phone-shape + confidence.
- **Next: M2** — ONNX NER (see `docs/M2-NER-EVALUATION.md`).

## 2026-07-11 — M1 complete: pipeline, server, prompt-augmentation round-trip

Finished the rest of M1 (Part A wiring + Part B, the ⭐ primary feature).

- **`Stage` trait finalized** (`src/pipeline/mod.rs`): resolved the open design
  point — stages now share a per-request **`RequestContext`** that carries the
  `Vault` from `on_request` to `on_response`. Stages run in order out, reverse
  order back. (Added `Vault::is_empty`.)
- **`PrivacyStage` implemented** (`src/pipeline/privacy.rs`): masks every text
  field of the outgoing OpenAI payload with one shared vault — `messages[].content`
  (string *or* `text` parts) and `messages[].tool_calls[].function.arguments` — so
  the same value maps to the same `[KIND_N]` everywhere. On the way back it
  restores `choices[].message.content` and `choices[].message.tool_calls[].function.arguments`
  (INT-03: the client runs its tools with real values). Clean requests are left
  byte-for-byte unchanged.
- **Prompt augmentation** (Part B): when anything was masked, a system message is
  prepended (`AUGMENTATION_PROMPT`) telling the model that `[KIND_N]` are typed
  real values to use verbatim (incl. tool-call args), never altered — and that
  they're auto-restored downstream. Injected only when PII is present, so clean
  prompts aren't polluted.
- **Determinism across turns** (INT-05): masking is a deterministic function of
  value; re-sending the same history yields identical tokens, and a repeated
  value reuses its token in reading order.
- **Server + forwarding** (`src/server.rs`, `src/proxy.rs`, `src/config.rs`):
  axum router (`POST /v1/chat/completions`, `GET /healthz`), `AppState` holding an
  `Arc<Upstream>` (reqwest) + the stage list, `TraceLayer`. Streaming requests
  get a clear 400 (M3). `Config::from_env` (`LISTEN_ADDR`, `UPSTREAM_BASE_URL`,
  optional `UPSTREAM_API_KEY`) with a **secret-redacted `Debug`** so the key never
  hits the logs. Upstream errors map to a JSON 502.
- **Tests: 26 green, no warnings.** Added `tests/pii_pipeline.rs` (INT-01…06 +
  clean-passthrough, stage-level, no network) and `tests/proxy_e2e.rs` (real
  client → proxy → mock upstream: asserts the upstream saw only masked values +
  the augmentation message, and the client got the originals back; plus a
  clean-passthrough case). Smoke-tested the real binary: `/healthz` → 200,
  chat vs a dead upstream → 502 JSON error.
- **Next: M2** — ONNX NER for names/orgs/locations (see `docs/M2-NER-EVALUATION.md`).

## 2026-07-11 — M1 Part A: structured-PII masking core

- **Recognizers implemented** (`src/pii/recognizers.rs`): email, phone (US dashed
  + IT/international), US SSN, credit card (Luhn-gated), IBAN, and a deterministic
  **SECRET** recognizer (`sk-…`, `sk-ant-…`, `AKIA…`) the old ML model missed.
- **Overlap resolution** by priority (Secret > IBAN > CreditCard > SSN > Email >
  Phone), then span length — this is what fixes the old proxy's REG-01 bug (IBAN
  mis-masked as PHONE): IBAN outranks phone/card and wins the shared digits.
- **Validators**: pure `luhn_valid` (checksum only; length enforced by the CC
  regex) and `iban_mod97`. IBAN detection is **structure-based**; mod-97 is a
  confidence signal only, so synthetic-but-shaped IBANs are still masked
  (privacy > strict validation), matching corpus case IBAN-03.
- **`Vault`** (`src/pii/anonymizer.rs`): `[KIND_N]` placeholders, numbered in
  reading order, spliced right-to-left so byte offsets stay valid. Deterministic —
  the same value reuses its token (VAULT-05). Exact `demask(mask(x)) == x`; the
  `]` terminator prevents `[EMAIL_1]` matching inside `[EMAIL_11]`.
- **`PiiKind`** gained a `Secret` variant, a `label()` for placeholders, and
  serde derives so the JSON corpus deserializes straight into it.
- **Tests green (16)**: inline unit tests + corpus-driven integration
  (`tests/pii_corpus.rs`, all `recognizers`/`validators`/`vault_roundtrip` cases)
  + property tests (`tests/pii_properties.rs`, proptest — PROP-01 detect+roundtrip
  for generated email/phone/SSN, PROP-02 no false positives on alphabetic text).
  Added `proptest` as a dev-dependency only. `cargo build --all-targets` clean, no
  warnings.
- **Next (Part B / rest of Part A)**: wire the privacy stage into the pipeline,
  add the axum server + reqwest forwarding of `/v1/chat/completions`, then the
  prompt-augmentation round-trip (system-prompt injection, tool_calls de-anon,
  deterministic placeholders across turns) with INT-01…06.

## 2026-07-11 — Toolchain up, first green build

- **Rust + portable MSVC installed, no admin.** rustup per-user (`cargo`/`rustc`
  1.97); MSVC linker via PortableBuildTools into `C:\Lavoro\Tools\MSVC` (`link.exe`,
  Windows SDK 10.0.26100, VC 14.44) with `env=user` (HKCU). See `docs/SETUP.md`.
- **First `cargo build` is green** — 177 deps, ~2 min, no warnings; even C-linking
  crates (`ring`) build, confirming the MSVC C toolchain works.
- **`ort`/`tokenizers` removed from M1**: `ort` 2.x is prerelease-only and not
  needed until M2, so the `onnx` feature is now empty and re-adds them at M2.
  Default build has zero native deps.
- **Decisions locked**: placeholder format `[KIND_N]` (e.g. `[EMAIL_1]`), ASCII;
  locale coverage IT + US. ARCHITECTURE and TESTING updated accordingly.
- **Roadmap refined** (session paused at baseline): prompt augmentation & round-trip
  promoted to a highlighted **primary** sub-milestone (M1 · Part B, right after the
  masking core) rather than a buried checkbox; kept in M1 because it is coupled to
  masking. Added a future **M5 — broad locale & language coverage** (beyond IT + US),
  relevant when we move off the OpenAI model.

## 2026-07-11 — Project bootstrap & scaffold

- Created repo docs: `README.md` (EN, canonical) + `README.it.md` (IT).
  Description kept generic (no ONNX) so the detection tech can change later.
  Convention adopted: Italian docs are named `<basename>.it.md`.
- Confirmed **MIT** license.
- Masked the git identity repo-locally — commits must never use the real/work
  email (global config was the Capgemini address).
- Ported PII test cases from the old `llmproxy-extended` into
  `tests/reference/old-proxy/` (PII only; "headroom" compression tests excluded).
  Key lesson extracted: structured PII was regex/Luhn/IBAN-based; the ONNX model
  (unreliable) only handled unstructured entities → adopted **hybrid detection**.
- Locked decisions: modular pipeline, **CPU-first**, hybrid detection, stack
  (tokio / axum / tower / reqwest / serde / ort / tokenizers). Roadmap M1→M4.
- Wrote the Rust module scaffold — trait/type definitions with `todo!()` bodies.
  ⚠️ **Unverified**: the Rust toolchain is not yet installed, nothing has been
  compiled. First job once MSVC is in place: get `cargo build` green, fix any
  scaffold errors, and verify the dependency versions in `Cargo.toml`.
- **Doc policy**: internal docs stay English-only; only root-level files get an
  Italian version (`README.it.md`). Removed the `docs/*.it.md` mirrors.
- **Prompt augmentation decided**: the privacy stage will transparently inject a
  system instruction so the model treats placeholders as typed real values and
  uses them verbatim (incl. tool calls). This expands the round-trip to
  `tool_calls` arguments and tool results, and requires deterministic placeholder
  assignment — see ARCHITECTURE.md.
- **Toolchain install started** — no admin rights on this PC. Rust core installs
  per-user without admin; the MSVC linker does not (VS Build Tools needs admin),
  so MSVC will come via portable extraction of the build tools + Windows SDK,
  with the GNU toolchain as a fallback that unblocks M1 immediately.
- **Rust installed** (per-user, no admin): `cargo`/`rustc` 1.97, host
  `x86_64-pc-windows-msvc`. Confirmed the only missing piece is the **MSVC
  linker** (`error: linker 'link.exe' not found`).
- **MSVC linker plan**: portable Build Tools via PortableBuildTools v2.10.2
  (`accept_license env=user target=x64 host=x64 path=<folder>` — writes
  INCLUDE/LIB/Path to HKCU, no admin; `devcmd.ps1` sets the env per-session).
  Awaiting the user's chosen install folder. Procedure in `docs/SETUP.md`.
- **Testing doc added** (`docs/TESTING.md`) from the old `docs/guide/testing.md`
  (TC-01…04 PII; headroom excluded) — captures the old proxy's real failures
  (IBAN→PHONE, secrets missed, `anubi`→PERSON false positive) as explicit test
  guards, and flags a new SECRET recognizer to add.
- **Test battery drafted**: expanded `docs/TESTING.md` with a full test catalog
  (unit / property / integration / e2e / regression) and added
  `tests/corpus/pii_cases.json` (data-driven detection / validator / roundtrip
  cases, reusing the old test values). Surfaced open decisions: locale scope
  (IT + US), placeholder format, IBAN validation strictness, SECRET patterns.

### Pending / next

- User provides the MSVC install folder → run PortableBuildTools → `cargo build` green.
- Then start M1: recognizers (incl. SECRET) + vault + privacy stage + prompt
  injection + forwarding, validated against the ported tests.

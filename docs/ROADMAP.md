# Roadmap

Development is split into milestones. Each builds on the previous one and is
independently testable. The checkboxes track progress — keep them current as
work lands. **This document is the single source of truth for "what's next".**

## M0 — Project setup ✅
- [x] Repo, license (MIT), README (EN) + README.it.md (IT)
- [x] Port PII test cases from the old proxy → `tests/reference/old-proxy/`
- [x] Tracking docs (roadmap, ARCHITECTURE, TESTING, SETUP, DEVLOG)
- [x] Rust module scaffold; `cargo build` green (177 deps)
- [x] Install Rust toolchain (rustup, per-user, no admin) — cargo/rustc 1.97
- [x] MSVC linker via portable Build Tools (no admin) → `C:\Lavoro\Tools\MSVC`
- [x] Decisions locked: hybrid detection, CPU-first, `[KIND_N]` placeholders, IT + US

## M1 — Structured PII pipeline (CPU, no ML) ✅

Locales: **IT + US**. Placeholder format: **`[KIND_N]`** (e.g. `[EMAIL_1]`).

### Part A — Masking core ✅
Reproduce the old proxy's structured-PII behavior, better, in Rust.
- [x] `PiiDetector` trait + `PiiEntity` / `PiiKind` types (incl. `Secret`)
- [x] Deterministic recognizers: email, phone (IT/US), SSN, credit card (Luhn), IBAN, with priority-based overlap resolution
- [x] SECRET recognizer (API keys / tokens, e.g. `sk-…`, `sk-ant-…`, `AKIA…`) — deterministic; the old ML model missed these
- [x] `Vault`: mask to `[KIND_N]` placeholders + exact-restore demask (deterministic per value)
- [x] Privacy stage wired into the pipeline (per-request `RequestContext` threads the `Vault` from request to response)
- [x] axum server forwarding `/v1/chat/completions` upstream (non-streaming), `reqwest` client, `/healthz`, config from env
- [x] Port the reference tests to Rust; no false positives; multi-PII round-trip exact (corpus-driven + proptest)

### Part B — Prompt augmentation & round-trip  ⭐ primary feature ✅
Make the masked data actually usable by the model — without this it mishandles
placeholders, especially in tool calls. A headline capability, not a nice-to-have.
- [x] Transparent system-prompt injection: teach the model that `[KIND_N]` are typed real values, to be used verbatim (incl. tool-call arguments) and never altered
- [x] Round-trip covers `tool_calls` arguments (de-anon in responses) and tool results (re-anon in requests)
- [x] Deterministic placeholder assignment (same value → same token across turns)
- [x] Integration tests INT-01…06 green (+ E2E-01/03 against a mock upstream)
- [x] Binary smoke test (`tests/binary_smoke.rs`) — boots the real `.exe` end-to-end

## M1.5 — Robustness & fail-closed  🔒 ✅
For a privacy tool the failure mode *is* the product: when anything is unexpected,
it must **fail closed** (block / scrub), never forward raw PII. Hardens M1 before
we broaden detection.
- [x] **Fail-closed policy**: unrecognized payload shapes, endpoints, or internal errors never pass PII through un-masked. The privacy stage sets a `RequestContext.block` on an unreadable `content` shape or missing `messages`; the proxy then returns 400 and never forwards. Unproxied paths → 404 (`fallback`).
- [x] **Full field coverage**: `messages[].content` (string + `text` of array parts), `messages[].name`, `tool_calls[].function.arguments`, legacy `function_call.arguments`, `tools[].function.description`, and every `description` inside `tools[].function.parameters`. All roles scanned (system/developer included).
- [x] **API scope decision**: **chat/completions only** is proxied; `/healthz` is served; everything else 404s and is never forwarded (documented in `ARCHITECTURE.md` and `src/server.rs`).
- [x] **Adversarial / evasion tests** (`tests/adversarial.rs` + corpus): broadened phone shapes ((555) 867-5309, dots, +1, extra IT grouping) and IBAN-before-word now caught; obfuscated emails pinned as a documented recall gap for NER/M2.
- [x] **Body-size limit**: `MAX_BODY_BYTES` (default 16 MiB) via `DefaultBodyLimit`, above axum's 2 MiB default so long-context requests aren't silently 413'd.
- [x] **Demask robustness**: a single tolerant pass restores `[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]` etc.; an unresolved but known-kind placeholder is logged (never silently shipped).

### Code-review findings — all closed ✅
8 findings from the M1 and M1.5 reviews — IBAN over-match, `iban_mod97` wiring,
single `system` message, response-header allowlist, tolerant de-mask; plus the
array-content fail-closed gap, phone over-match, and `Confidence` consumed — all
fixed with tests. Full detail in `docs/DEVLOG.md` (2026-07-11) / commits
`bb68707`, `775945f`.

## M2 — Unstructured entities (ONNX NER, CPU) ✅
Goal: add names / organizations / locations via a local ML model.
Candidate models & evaluation plan: [docs/M2-NER-EVALUATION.md](M2-NER-EVALUATION.md).

**Done:** hybrid detector + `OnnxNerDetector` (`--features onnx`), the model
**measured and picked** (XLM-R int8, head-to-head vs Piiranha — see DEVLOG
2026-07-12), and all 9 review findings closed. The NER runs off-repo model files
via env vars (`NER_MODEL_PATH` / `NER_TOKENIZER_PATH` / `NER_LABELS`); the shipped
default build stays feature-off and native-dep-free.
- [x] `OnnxNerDetector` behind the `onnx` feature (CPU EP) — `src/pii/onnx.rs`: HF tokenizer + `ort` session, per-token argmax → BIO decode; `--features onnx` compiles clean. Runtime verification is gated on a chosen model.
- [x] **Evaluate candidate models & pick one** — DONE (measured, head-to-head, `--features onnx`). **Picked: XLM-R int8** (`jiting/xlm-roberta-base-ner-hrl_onnx` @ `478a2a3`). Numbers + method in `docs/DEVLOG.md` (2026-07-12).
  - [x] **Eval harness** — `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d): scores a live candidate against `ner_cases.json` (recall/precision/F1 per type + timing) **through the hybrid resolver**. Runs with `cargo test --features onnx --test ner_eval -- --ignored --nocapture`.
  - [x] **Candidate A — XLM-R** (drop-in baseline): int8, hybrid **Org 1.00 / Loc 1.00 / Person 0.75** (only misses the single-token "Caia" split + the "Herr" title), ~23 ms/case CPU, 266 MB, multilingual, no `token_type_ids`, no label mapping. **Winner.**
  - [x] **Candidate B — Piiranha** (PII specialist): int8, hybrid **~0.00 recall** on natural-sentence NER (fires only fragments like "Pinc"/"New") and has **no Organization label** — it's a form/structured-PII model, not free-text NER. Rejected. (Its granular labels wired via extended `label_to_kind`; `type_vocab_size:0` → no `token_type_ids` needed.)
  - [x] **Pick + record** → XLM-R int8; numbers in `docs/DEVLOG.md`. GLiNER escalation not needed (XLM-R clears the bar).
- [x] Combine deterministic + ML detectors behind one `PiiDetector` — `CompositeDetector` fans out to N detectors and merges via the shared `overlap::resolve_overlaps` (structured PII outranks NER via `PiiKind::priority`)
- [x] Extend the corpus with unstructured-entity cases — `tests/corpus/ner_cases.json` (Person/Org/Location, IT+EN, single-word names, REG-03 negatives, a DE multilingual preview)
- [x] **NER concurrency**: `OnnxNerDetector` holds a round-robin **pool of sessions** (`NER_POOL_SIZE`) so inference isn't a single-threaded bottleneck.
- [x] *(micro, from M1.5 review)* **Symmetric response de-masking** — `demask_content` now mirrors `mask_content` (bare-string array elements restored too).

**Fail-closed for NER — RESOLVED (M2-R1/R2, see below).** A `NER_REQUIRED` switch
now makes the NER fail closed: a required detector's load *or* inference error is
fatal / blocks the request via a new `PiiDetector::try_detect` error channel; when
not required, a `FailOpen` wrapper keeps the old fail-open behaviour explicitly.

### M2 review findings — ALL 9 closed ✅
Closed R1–R9 with tests (see DEVLOG 2026-07-12); **65 tests green (default),
`--features onnx` builds + runs a live model clean, no warnings**. R6 was closed by
downloading the XLM-R model and confirming non-ASCII ("Müller", 2-byte `ü`) extracts
on exact byte boundaries — plus a whitespace-trim fix so SentencePiece leading-space
offsets don't leak into the span. The closed findings are left inline for the
reviewer to collapse to a DEVLOG pointer per the lifecycle.

- [x] **M2-R1 — load-time NER downgrade is silent (fail-open).** *(closed: `NER_REQUIRED` makes a configured-but-unloadable NER fatal at startup; `build_detector`/`AppState::new` now return `Result`. Test: `required_ner_is_fatal_when_absent`.)* `build_detector` /
  `load_onnx_ner` (`src/server.rs:78-100`) fall back to structured-only when a
  *configured* NER fails to load: it logs at `error` but the server then runs with
  weakened protection and forwards names upstream. The ROADMAP "Open" note above
  only covers the *request-time* inference error, not this *load-time* case. **Fix:**
  add a `NER_REQUIRED` (or fail-closed-by-default) switch so a configured-but-
  unloadable NER is fatal at startup, and broaden the deferred fail-closed decision
  to cover **both** the load path and the request path (one detector→pipeline error
  channel serves both). **Test:** with `NER_REQUIRED` set and a bad model path,
  `AppState::new` / `run` returns an error instead of a structured-only server.
- [x] **M2-R2 — NER inference error silently drops all names (fail-open leak).** *(closed: added `PiiDetector::try_detect -> Result<_, DetectError>`; the composite propagates, `PrivacyStage` blocks the request on a required-detector error, and a `FailOpen` wrapper opts a non-critical detector out. Tests: `required_detector_error_propagates`, `fail_open_wrapped_detector_is_swallowed`, `required_detector_error_blocks_request_fail_closed`.)*
  `OnnxNerDetector::detect` (`src/pii/onnx.rs:146-161`) returns `Vec::new()` on any
  `infer` error, so unstructured PII in that request is forwarded raw (structured
  PII is still masked). The `PiiDetector::detect` signature has no error channel, so
  the pipeline can't fail closed. **Fix (the deferred decision, spelled out):** change
  the trait to `fn detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError>`
  (or add a fallible method), and in `PrivacyStage::on_request` set `ctx.block` when a
  *required* detector errors. **Test:** a stub detector that returns `Err` on a text
  with a name → request is blocked (400), not forwarded.
- [x] **M2-R3 — `NER_LABELS` length/order not validated against the model.** *(closed: `ner_decode::validate_label_count` fails inference when `NER_LABELS.len() != num_labels`. Test: `label_count_validation`.)*
  `infer` (`src/pii/onnx.rs:120-129`) does `id2label.get(best).unwrap_or("O")`, so if
  the configured label list is shorter than (or misordered vs.) the model's real
  output dimension, out-of-range class ids silently become `O` — a `Person` token is
  dropped and the name leaks. Model selection sets `NER_LABELS` by hand, so this is
  an easy misconfiguration. **Fix:** validate `id2label.len() == num_labels` on the
  first inference (or at load); on mismatch fail closed / return an error rather than
  silently degrading. **Test:** a detector built with a too-short `id2label` errors
  instead of returning zero entities.
- [x] **M2-R4 — hardcoded model I/O couples the detector to one export shape.** *(closed: `outputs.get("logits")` returns a graceful `Err` instead of panicking; `token_type_ids` is threaded when `NER_TOKEN_TYPE_IDS` is set (BERT-family); the required input/output contract is documented in `onnx.rs`. A fixture-model test for a non-`logits` output is deferred to model landing.)*
  `outputs["logits"]` (`src/pii/onnx.rs:117`) **panics** (`no output named
  \`logits\``) if the model's output node has another name; and `ort::inputs!`
  (`onnx.rs:108-113`) supplies only `input_ids` + `attention_mask`, so a model that
  requires `token_type_ids` fails **every** inference and (via M2-R2) silently falls
  open. **Fix:** read the output by index or a configurable name and return a graceful
  error instead of panicking; document the required-input/output contract and
  constrain model selection to models matching it (or thread `token_type_ids` when
  present). **Test:** decode against a tiny fixture whose output node isn't `logits`
  → graceful `Err`, not a panic.
- [x] **M2-R5 — `is_begin` misses underscore BIO prefixes.** *(closed: `is_begin` now treats the first `-`/`_`-separated segment `B` (case-insensitive) as a begin. Test: `underscore_bio_prefix_also_splits_entities`.)* `label_to_kind`
  (`src/pii/ner_decode.rs:22-30`) accepts both `-` and `_` separators, but `is_begin`
  (`ner_decode.rs:32-35`) only recognises `b-`. A model using `B_PER`/`I_PER` labels
  decodes kinds correctly yet never sees a "begin", so two adjacent same-type entities
  (e.g. `New York` then `London`) glue into one span. **Fix:** make `is_begin` treat
  the first `-`/`_`-separated segment `B` (case-insensitive) as a begin. **Test:** the
  existing `adjacent_same_type_entities_are_split_by_begin` case, re-run with
  `B_LOC`/`I_LOC`/`B_LOC`, still splits.
- [x] **M2-R6 — offset units unverified for non-ASCII (recall/robustness).** *(closed with the real model: XLM-R extracts "Müller"/"Berlin" on exact byte boundaries — the tokenizer emits byte offsets, so the `&str` slicing is correct for non-ASCII. Added a **whitespace-trim** in `decode_entities` (SentencePiece includes the leading space in a token offset, e.g. `▁Mario` spans " Mario") so the span is exact and masking preserves surrounding spaces. Tests: `leading_space_in_token_offset_is_trimmed` + the live `ner_eval` on the DE case. A dropped off-boundary span still `warn`s as a backstop.)*
- [x] **M2-R7 — partial-overlap drops the NER remainder (recall, by-design).** *(closed as a conscious choice: `resolve_overlaps` drops the whole lower-priority span — documented in the function + tested (`ner_span_enclosing_a_structured_span_is_dropped_whole`). Structured PII is never lost; only the non-overlapping unstructured remainder.)*
  `resolve_overlaps` (`src/pii/overlap.rs:26-33`) discards a whole lower-priority NER
  span when it overlaps a kept structured span — including the non-overlapping part,
  which may be a real name (e.g. a `Person` span that happens to enclose an email
  keeps only the email). Deterministic-wins is preserved (correct); the remainder loss
  is an acceptable lean choice but should be a conscious, tested one. **Fix:** decide
  keep-vs-trim when the model lands; if kept, document it. **Test:** an NER span that
  strictly encloses a structured span asserts the intended remainder behaviour.
- [x] **M2-R8 — inference-error log may format input-derived text.** *(closed: the tokenize path now maps to a fixed `"tokenizer error"` (no `{e}`), so no input text can reach a log or the `DetectError` message / 400 body.)* `detect`'s
  `warn!(error = %err, …)` (`src/pii/onnx.rs:156`) prints the full `anyhow` error; the
  `tokenize: {e}` variant (`onnx.rs:85-86`) could embed input text, which for a
  "never log raw PII" tool is a leak vector. **Fix:** log the error *category* on the
  tokenize path, not the formatted error. **Test:** n/a (hardening) — or assert the
  warn payload contains no input substring.
- [x] **M2-R9 (minor, non-leak) — poisoned session mutex disables a pool slot.** *(closed: `.lock().unwrap_or_else(|p| p.into_inner())` recovers a poisoned lock instead of panicking for the process lifetime.)*
  `self.sessions[idx].lock().expect("session mutex poisoned")` (`src/pii/onnx.rs:107`)
  panics for the life of the process if an `ort` panic ever poisons that mutex,
  permanently removing 1/`pool_size` of capacity. No leak (the panic precedes
  forwarding), but an availability cliff. **Fix:** recover via `into_inner()` on a
  poisoned lock. **Test:** n/a (robustness).

**Doc follow-up (done in this review):** `docs/ARCHITECTURE.md` "Robustness &
fail-closed" now records the known M2 gap that NER load/inference errors fail
**open** for unstructured PII (cross-links M2-R1/R2), so the strong fail-closed
posture there is no longer misleading.

## M2.5 — HuggingFace model management (auto-download via `hf-hub`)
Ergonomics + reproducibility, **not a correctness gap** (M2 works). Today the NER
model is downloaded by hand and lives in a plain folder; the standard HF cache
(`~/.cache/huggingface/hub`) is a **library-managed** content-addressed tree
(`refs`/`blobs`/`snapshots`) you must not hand-populate. Use the official **`hf-hub`
Rust crate** (behind the `onnx` feature) to fetch the **revision-pinned** model into
that standard cache — conventional location, dedup, reproducible. Pure Rust: the
default build stays native-dep-free.
- [ ] Add `hf-hub` as an **optional** dep, wired into the `onnx` feature only.
- [ ] Resolve a model file from `(repo, revision, filename)` → a local path in the HF
  cache. **`NER_MODEL_PATH` (explicit local file) keeps priority** — anyone wanting
  zero outbound calls just sets it, exactly as today.
- [ ] **Opt-in** auto-download: when `NER_MODEL_PATH` is unset but `NER_MODEL_REPO`
  (+ pinned `NER_MODEL_REVISION`, default `478a2a3`, + `NER_MODEL_FILE` e.g.
  `onnx/model_quantized.onnx`, + tokenizer file) is set, fetch via `hf-hub`. It's a
  **one-time model fetch (not user data)** — log it; never silent.
- [ ] Derive `NER_LABELS` from the model's `config.json` `id2label` (in class-id
  order) so the hand-typed list is optional (keep an explicit override).
- [ ] Docs: record the env contract + the privacy note (single opt-in outbound model
  fetch) in `ARCHITECTURE.md` / `SETUP.md`; default XLM-R repo/revision pinned.
- [ ] Tests: path-resolution + `id2label`-ordering logic unit-tested **without
  network**; the actual download exercised only by the `#[ignore]`d eval harness.

## M3 — Streaming
Goal: SSE token streaming with incremental de-anonymization.
- [ ] Streaming passthrough of provider responses
- [ ] Hold-back buffer that restores placeholders split across chunks

## M4 — GPU optimization & load
Goal: faster inference once the model is locked, and prove it holds under load.

**Why GPU is safely deferred (decided 2026-07-12):** the M2 model choice is
**execution-provider-agnostic** — every candidate (incl. GLiNER) is standard ONNX
and runs on any `ort` EP, so GPU does *not* constrain model selection; we pick on
CPU recall/latency now and revisit the EP here. On this Windows/no-admin box the
natural EP is **DirectML** (any DX12 GPU, no CUDA/admin). Going to GPU is mostly a
config change: swap the EP and switch the weight file from int8 (CPU) to the
pre-shipped `model_fp16.onnx` (fp16 is the GPU sweet spot; int8 on GPU needs
EP-specific support). No premature GPU work before the CPU baseline is locked.
- [ ] GPU execution provider (CUDA / **DirectML** for no-admin Windows) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline (CPU int8 → GPU fp16)
- [ ] **Load / throughput harness** (concurrent connections, large bodies) — stability under load was the founding motivation; measure it, don't assume it

## M5 — Broad locale & language coverage (future)
Goal: extend PII coverage beyond IT + US to a wide set of locales and languages,
so the proxy protects data regardless of the user's language or the upstream
provider. Likely valuable once we move off the OpenAI model and serve broader
traffic — priority can be pulled earlier if real usage demands it.
- [ ] Locale-parametrized structured recognizers (phone, national IDs, IBAN countries)
- [ ] Multilingual NER model(s) for names / orgs / locations (evaluate vs a multilingual corpus)
- [ ] Extend the test corpus with multi-language / multi-locale cases
- [ ] Provider-agnostic verification (not tied to OpenAI-specific behavior)

## Backlog / later
Auth & rate-limiting stages, **TLS / running behind a TLS terminator**, structured
audit logging (**never log raw PII**), config-file support & container deployment,
additional providers, metrics/observability.

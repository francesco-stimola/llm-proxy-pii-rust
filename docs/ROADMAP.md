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

## M2 — Unstructured entities (ONNX NER, CPU)
Goal: add names / organizations / locations via a local ML model.
Candidate models & evaluation plan: [docs/M2-NER-EVALUATION.md](M2-NER-EVALUATION.md).

**Infrastructure + the ONNX detector are built and compile** (`--features onnx`
clean, native ORT + tokenizers); only the **measured model choice** remains, since
that genuinely needs real model files (measure, don't guess). See DEVLOG 2026-07-12.
- [x] `OnnxNerDetector` behind the `onnx` feature (CPU EP) — `src/pii/onnx.rs`: HF tokenizer + `ort` session, per-token argmax → BIO decode; `--features onnx` compiles clean. Runtime verification is gated on a chosen model.
- [ ] Evaluate candidate models against the corpus; pick the most reliable one — **remaining measured step**: needs real model files (download + run through `OnnxNerDetector`), scored against `ner_cases.json` per `docs/M2-NER-EVALUATION.md`. Not guessed, not fabricated.
- [x] Combine deterministic + ML detectors behind one `PiiDetector` — `CompositeDetector` fans out to N detectors and merges via the shared `overlap::resolve_overlaps` (structured PII outranks NER via `PiiKind::priority`)
- [x] Extend the corpus with unstructured-entity cases — `tests/corpus/ner_cases.json` (Person/Org/Location, IT+EN, single-word names, REG-03 negatives, a DE multilingual preview)
- [x] **NER concurrency**: `OnnxNerDetector` holds a round-robin **pool of sessions** (`NER_POOL_SIZE`) so inference isn't a single-threaded bottleneck.
- [x] *(micro, from M1.5 review)* **Symmetric response de-masking** — `demask_content` now mirrors `mask_content` (bare-string array elements restored too).

**Open (surfaced during build), for review:** on a per-request NER **inference
error** the detector logs and yields no NER entities (structured PII still masked)
— i.e. fail-*open* for unstructured PII. Decide whether a configured-but-failing
NER should instead **fail closed** (block the request); needs a detector→pipeline
error channel. Tracked here so the reviewer can weigh it against the model choice.

### M2 review findings — OPEN
Verified independently: 57 tests green (default) with no warnings; `--features onnx`
`check` + `build` link clean. The hybrid seam is sound — a deterministic match
always outranks an NER guess, so no structured span is ever lost to the ML layer,
and NER is inert in the shipped default build (no leak today). The findings below
are latent for when a model lands, plus doc accuracy and tightening the two
deferrals. **Verdict: M2 is sound enough to build model-selection on top of; no
blockers.** Both deferrals (measured model selection; fail-closed-on-NER-error)
are the right calls — model choice genuinely needs real files, and the error-policy
decision needs a real error rate to tune — but the fail-closed one is
under-specified (see M2-R1/R2).

- [ ] **M2-R1 — load-time NER downgrade is silent (fail-open).** `build_detector` /
  `load_onnx_ner` (`src/server.rs:78-100`) fall back to structured-only when a
  *configured* NER fails to load: it logs at `error` but the server then runs with
  weakened protection and forwards names upstream. The ROADMAP "Open" note above
  only covers the *request-time* inference error, not this *load-time* case. **Fix:**
  add a `NER_REQUIRED` (or fail-closed-by-default) switch so a configured-but-
  unloadable NER is fatal at startup, and broaden the deferred fail-closed decision
  to cover **both** the load path and the request path (one detector→pipeline error
  channel serves both). **Test:** with `NER_REQUIRED` set and a bad model path,
  `AppState::new` / `run` returns an error instead of a structured-only server.
- [ ] **M2-R2 — NER inference error silently drops all names (fail-open leak).**
  `OnnxNerDetector::detect` (`src/pii/onnx.rs:146-161`) returns `Vec::new()` on any
  `infer` error, so unstructured PII in that request is forwarded raw (structured
  PII is still masked). The `PiiDetector::detect` signature has no error channel, so
  the pipeline can't fail closed. **Fix (the deferred decision, spelled out):** change
  the trait to `fn detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError>`
  (or add a fallible method), and in `PrivacyStage::on_request` set `ctx.block` when a
  *required* detector errors. **Test:** a stub detector that returns `Err` on a text
  with a name → request is blocked (400), not forwarded.
- [ ] **M2-R3 — `NER_LABELS` length/order not validated against the model.**
  `infer` (`src/pii/onnx.rs:120-129`) does `id2label.get(best).unwrap_or("O")`, so if
  the configured label list is shorter than (or misordered vs.) the model's real
  output dimension, out-of-range class ids silently become `O` — a `Person` token is
  dropped and the name leaks. Model selection sets `NER_LABELS` by hand, so this is
  an easy misconfiguration. **Fix:** validate `id2label.len() == num_labels` on the
  first inference (or at load); on mismatch fail closed / return an error rather than
  silently degrading. **Test:** a detector built with a too-short `id2label` errors
  instead of returning zero entities.
- [ ] **M2-R4 — hardcoded model I/O couples the detector to one export shape.**
  `outputs["logits"]` (`src/pii/onnx.rs:117`) **panics** (`no output named
  \`logits\``) if the model's output node has another name; and `ort::inputs!`
  (`onnx.rs:108-113`) supplies only `input_ids` + `attention_mask`, so a model that
  requires `token_type_ids` fails **every** inference and (via M2-R2) silently falls
  open. **Fix:** read the output by index or a configurable name and return a graceful
  error instead of panicking; document the required-input/output contract and
  constrain model selection to models matching it (or thread `token_type_ids` when
  present). **Test:** decode against a tiny fixture whose output node isn't `logits`
  → graceful `Err`, not a panic.
- [ ] **M2-R5 — `is_begin` misses underscore BIO prefixes.** `label_to_kind`
  (`src/pii/ner_decode.rs:22-30`) accepts both `-` and `_` separators, but `is_begin`
  (`ner_decode.rs:32-35`) only recognises `b-`. A model using `B_PER`/`I_PER` labels
  decodes kinds correctly yet never sees a "begin", so two adjacent same-type entities
  (e.g. `New York` then `London`) glue into one span. **Fix:** make `is_begin` treat
  the first `-`/`_`-separated segment `B` (case-insensitive) as a begin. **Test:** the
  existing `adjacent_same_type_entities_are_split_by_begin` case, re-run with
  `B_LOC`/`I_LOC`/`B_LOC`, still splits.
- [ ] **M2-R6 — offset units unverified for non-ASCII (recall/robustness).**
  `decode_entities` (`src/pii/ner_decode.rs:41-77`) treats tokenizer offsets as byte
  offsets into the Rust `&str`. If the chosen tokenizer emits char offsets, non-ASCII
  names (e.g. `Müller`) misalign; `text.get(start..end)` then returns `None` and the
  entity is silently dropped (a name leak). **Fix (at model landing):** add a non-ASCII
  NER case and assert an exact mask→restore round-trip, confirming byte-offset
  alignment (or convert offsets if the tokenizer is char-based). **Test:** the new
  non-ASCII corpus case round-trips exactly under `OnnxNerDetector`.
- [ ] **M2-R7 — partial-overlap drops the NER remainder (recall, by-design).**
  `resolve_overlaps` (`src/pii/overlap.rs:26-33`) discards a whole lower-priority NER
  span when it overlaps a kept structured span — including the non-overlapping part,
  which may be a real name (e.g. a `Person` span that happens to enclose an email
  keeps only the email). Deterministic-wins is preserved (correct); the remainder loss
  is an acceptable lean choice but should be a conscious, tested one. **Fix:** decide
  keep-vs-trim when the model lands; if kept, document it. **Test:** an NER span that
  strictly encloses a structured span asserts the intended remainder behaviour.
- [ ] **M2-R8 — inference-error log may format input-derived text.** `detect`'s
  `warn!(error = %err, …)` (`src/pii/onnx.rs:156`) prints the full `anyhow` error; the
  `tokenize: {e}` variant (`onnx.rs:85-86`) could embed input text, which for a
  "never log raw PII" tool is a leak vector. **Fix:** log the error *category* on the
  tokenize path, not the formatted error. **Test:** n/a (hardening) — or assert the
  warn payload contains no input substring.
- [ ] **M2-R9 (minor, non-leak) — poisoned session mutex disables a pool slot.**
  `self.sessions[idx].lock().expect("session mutex poisoned")` (`src/pii/onnx.rs:107`)
  panics for the life of the process if an `ort` panic ever poisons that mutex,
  permanently removing 1/`pool_size` of capacity. No leak (the panic precedes
  forwarding), but an availability cliff. **Fix:** recover via `into_inner()` on a
  poisoned lock. **Test:** n/a (robustness).

**Doc follow-up (done in this review):** `docs/ARCHITECTURE.md` "Robustness &
fail-closed" now records the known M2 gap that NER load/inference errors fail
**open** for unstructured PII (cross-links M2-R1/R2), so the strong fail-closed
posture there is no longer misleading.

## M3 — Streaming
Goal: SSE token streaming with incremental de-anonymization.
- [ ] Streaming passthrough of provider responses
- [ ] Hold-back buffer that restores placeholders split across chunks

## M4 — GPU optimization & load
Goal: faster inference once the model is locked, and prove it holds under load.
- [ ] GPU execution provider (CUDA / DirectML) behind config
- [ ] Quantization tuning; benchmark against the CPU baseline
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

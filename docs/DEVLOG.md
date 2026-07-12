# Development log

Newest first. One entry per meaningful change — note *what* and *why*, not just
*what*. This is the running history so context is never lost between sessions.

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

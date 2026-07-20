# Testing strategy

Defines *what* we test and *how*, so the PII behavior is provably correct and the
reliability problems that sank the old proxy don't come back.

Derived from the old proxy's tests: the ported unit/property tests in
`tests/reference/old-proxy/` and its manual `docs/guide/testing.md`. Only PII
scenarios are carried over — the old "headroom"/compression tests are out of scope.

## Test levels

1. **Unit** — pure functions, no I/O.
   - Recognizers: each structured category (email, phone, SSN, credit card,
     IBAN, secret) matches valid samples and rejects near-misses.
   - Validators: Luhn (credit card) and IBAN mod-97 checksum.
   - `Vault`: mask produces typed placeholders; demask restores the exact
     original; round-trip is identity; text with no PII is unchanged.
2. **Property-based** — port `test_pii_hypothesis.py`: generate valid PII and
   assert it is always detected, masked, and round-tripped; generate safe
   alphabetic strings and assert they are never detected (no false positives).
3. **Integration** — the privacy stage over realistic OpenAI-shaped payloads:
   messages, multiple PII in one message, `tool_calls` arguments, tool results.
   Covers the prompt-augmentation injection and deterministic placeholder
   consistency across turns.
4. **End-to-end** — the proxy in front of a mock provider: a request with PII
   goes out masked; a response referencing placeholders comes back
   de-anonymized. Ported from the manual scenarios below.

## Reliability guards (lessons from the old proxy)

The old proxy's own checklist recorded these real failures — our tests must
explicitly cover each:

| Failure seen in the old proxy | Our guard |
|---|---|
| IBAN mis-masked as PHONE (`IT[PRIVATE_PHONE_1]`), country prefix leaked | IBAN recognizer + checksum wins over phone; assert label is IBAN and no prefix leaks (already in the ported hypothesis test) |
| Secrets / API keys (`sk-ant-…`, `sk-…`) not detected by the ML model | deterministic SECRET recognizer with known key formats — never rely on the model for these |
| Connection name (`anubi`) masked as PERSON — false positive | names come only from the ML NER (M2) with a confidence threshold; structured recognizers never guess names |
| Single-word / low-signal names dropped inconsistently | tracked as an M2 NER-quality item, measured against a labelled corpus |

## Ported scenarios (PII only)

From the old `docs/guide/testing.md` (TC-01…TC-04; TC-05…TC-08 were headroom and
are excluded). Old categories: `PRIVATE_PERSON`, `PRIVATE_EMAIL`, `PRIVATE_PHONE`,
`ACCOUNT_NUMBER` (IBAN), `SECRET`.

- **TC-01 — multiple PII in one chat message** (IT + EN): two names, email,
  phone, IBAN → all masked to typed placeholders, nothing in clear.
- **TC-02 — PII in a file / tool result** (CSV of contacts): email / phone / IBAN
  inside a `tool_result` are masked.
- **TC-03 — secret in a shell command output**: an API key + email are masked
  before the model reads them.
- **TC-04 — PII from a DB query result** (`SELECT … FROM DUAL`): all categories
  in one tabular result are masked.

Each becomes an integration / e2e test with a mock provider once M1/M2 land. The
expected placeholders follow our format, not the old `[PRIVATE_*]` one.

## Category & placeholder mapping (old → new)

| Old label | New `PiiKind` | Engine |
|---|---|---|
| PRIVATE_EMAIL | `Email` | structured |
| PRIVATE_PHONE | `Phone` | structured |
| ACCOUNT_NUMBER | `Iban` | structured |
| (from unit tests) | `Ssn`, `CreditCard` | structured |
| (M4, always-on) | `NationalId` (IT CF · GB NINO · ES DNI/NIE · FR NIR · DE Steuer-ID · NL BSN · PT NIF · LV code · zh Resident ID) | structured |
| SECRET | `Secret` *(to add to `PiiKind`)* | structured |
| PRIVATE_PERSON | `Person` | ML NER (M2) |

## Test catalog

The concrete battery the tool must pass, grouped by level. Data-driven
structured-PII cases live in `tests/corpus/pii_cases.json` (consumed by the Rust
tests); the integration / e2e / regression cases are behavioral and described here.

### Unit — recognizers (`tests/corpus/pii_cases.json` → `recognizers`)
Per category, positives (detected with the right `PiiKind`) and negatives (no
false positive): `email`, `phone`, `ssn`, `credit_card`, `iban`, `secret`,
**`non_ascii_scripts`**.

> **`non_ascii_scripts` exists because its absence was a leak (M4-R13).** Until then this corpus held
> **zero non-ASCII characters** — so a *total* detection failure in CJK text (Rust `regex`'s `\b` is
> Unicode-aware; a Han character is a word character, so no boundary separates it from a digit, and CJK
> has no inter-word spaces) was invisible to the suite and survived **four** reviews. The category pins
> the glued forms (`我的信用卡号是4111111111111111`, `カード番号は…`, `Карта…`, the zh Resident ID, a
> secret, an IBAN, an SSN) **and** the negatives proving the anti-false-positive guarantee still holds
> inside a longer *ASCII* token. **Any new recognizer must be exercised in a non-ASCII context here.**

### Unit — validators (`… → validators`)
- `luhn` — accepts valid card numbers, rejects non-Luhn 16-digit strings.
- `iban_mod97` — mod-97 checksum accept/reject (used as a confidence signal, not
  a hard gate — see open decisions).

### Unit — Vault
- VAULT-01 — mask replaces the raw value with a typed placeholder (raw absent).
- VAULT-02 — `demask(mask(x)) == x` for every `vault_roundtrip` case.
- VAULT-03 — text with no PII is returned unchanged.
- VAULT-04 — multiple PII in one string are all masked and all restored.
- VAULT-05 — deterministic: the same value gets the same placeholder within a text.

### Property-based (ports `test_pii_hypothesis.py`)
- PROP-01 — generated valid email/phone/ssn/credit-card/iban are always detected,
  masked, and round-tripped.
- PROP-02 — random alphabetic strings are never detected (no false positives).
- **PROP-03 — the resolver's invariant: *no structured span's bytes are ever abandoned*** (M4-R10/R11,
  `every_structured_candidate_byte_is_covered` in `src/pii/recognizers.rs`). Glue PII values — including the
  **grouped** shapes (spaces inside the value) that make a recognizer *partially* overlap an email — in arbitrary
  orders and separators; assert every **raw** structured candidate (pre-resolution) is **fully covered** by some
  resolved span, and that mask→demask is still exact. This is the guard the priority-only fixes lacked: M4-R7 and
  M4-R9 each passed their own hand-written case while silently abandoning the *other* side of a partial overlap —
  **a per-byte invariant cannot be satisfied by picking a winner**. Verified non-vacuous (it rediscovers the
  M4-R11 leak on its own when the union-merge is disabled).
  > **Its tables carry non-ASCII glue on purpose (M4-R16).** `GLUE` includes CJK / Cyrillic / accented fragments
  > and two samples are already embedded in non-ASCII context, so the invariant is exercised on **multi-byte**
  > input (`widen_to_char_boundaries`, the union re-slice). ASCII-only tables were the exact blind spot that let
  > M4-R13 live through four reviews — and they were sitting in the one test the whole guarantee rests on.
- **PROP-04 — nothing PII-shaped survives masking** (M4-R17, `masking_leaves_nothing_detectable`). Re-run the
  detector on the **masked output**: it must find no structured entity. This is the necessary companion to
  PROP-03, which quantifies over the *candidate set* and is therefore only as strong as that set — a value the
  recognizers never emitted (because `find_iter` was leftmost-non-overlapping, or because masking later *exposed*
  it) satisfies PROP-03 **vacuously**. PROP-04 quantifies over the **output bytes**, which no candidate-generation
  gap can hide. It earned its keep immediately: on its first run it found a live leak
  (`4111111111111111555 867 5309` → `4111111111111111[PHONE_1]`, a Luhn-valid card in clear), which is what
  drove `Vault::mask_all` (mask to a fixpoint).

### Integration — pipeline over OpenAI-shaped payloads
- INT-01 — user message with multiple PII → outgoing request carries placeholders; vault populated.
- INT-02 — PII inside a `tool` result message is masked.
- INT-03 — assistant `tool_calls` arguments in the response are de-anonymized before the client sees them.
- INT-04 — the augmentation system message is injected into the outgoing request.
- INT-05 — deterministic placeholders across turns: re-sending masked history yields the same tokens.
- INT-06 — response text referencing placeholders is de-anonymized.

### End-to-end — proxy in front of a mock provider (from the old TC-01…04)
- E2E-01 (TC-01) — multi-PII chat message, IT + EN (`e2e01_multi_pii_roundtrip`).
- E2E-02 (TC-02) — PII in a CSV `tool_result`, masked upstream and restored to the client
  (`e2e02_csv_tool_result_pii_is_masked_and_restored`).
- E2E-03 (TC-03) — secret + email in a shell command output (`e2e03_secret_and_email_masked_before_upstream`).
- E2E-04 (TC-04) — all categories (email/phone/SSN/card/IBAN/secret) in a `SELECT … FROM DUAL`-style
  result, masked upstream and restored to the client (`e2e04_db_query_result_all_categories_masked_and_restored`).
- E2E-BIN — `tests/binary_smoke.rs`: boots the **compiled binary** (`main` → `from_env` → `run`) against a mock upstream for one PII round-trip; the only test that exercises the real process (kept to a single case).
- **M5 additions (`tests/proxy_e2e.rs`)** — the full-HTTP companions to the pipeline-level INT
  tests, driving the real router + a mock upstream rather than `PrivacyStage` directly:
  - `e2e_tool_call_arguments_round_trip_over_http` — the INT-03 round-trip over real HTTP: a
    mock upstream's `tool_calls[].function.arguments` referencing the placeholder the request
    masked is de-anonymized before the client sees it.
  - `e2e_multi_turn_determinism_across_stateless_requests` — two real HTTP round-trips
    resending conversation history (as a stateless OpenAI-style client does every turn): a
    repeated value keeps its placeholder token across the two independent per-request vaults,
    and a new value gets the next one.
  - `e2e_masking_is_provider_agnostic` (LOC-11, extended) — now compares all **three** mock
    upstream shapes M5 asks for: OpenAI, Copilot, **and** Anthropic all yield a byte-identical
    masked body.

### Regression guards (the old proxy's real failures)
- REG-01 — Italian IBAN masked as IBAN, not phone; country/check prefix does not leak.
- REG-02 — secrets (`sk-…`, `sk-ant-…`) are detected (the old ML model missed them).
- REG-03 — structured recognizers never mask a plain word / connection name (e.g. `anubi`) as a person; names come only from the ML NER (M2) with a threshold.
- REG-04 — a 16-digit non-Luhn number is not masked as a credit card.

### M1.5 — robustness & fail-closed
- FC-01 — unreadable `content` shape (bare object) → request blocked (400), not forwarded (`fail_closed_on_unrecognized_content_shape`, e2e `…returns_400…`).
- FC-02 — missing `messages` → blocked (`fail_closed_on_missing_messages`).
- FC-03 — unproxied path/method → 404, never forwarded (`e2e_unproxied_endpoint_returns_404`).
- COV-01 — `name`, `tools[].function` description + nested param descriptions, and legacy `function_call.arguments` are masked (`full_coverage_masks_name_tools_and_function_call`).
- COV-02 — a value split across fields shares one token (`same_value_split_across_fields_shares_one_token`).
- SYS-01 — augmentation merges into an existing system message; exactly one reaches the upstream (INT-07).
- HDR-01 — safe upstream response headers are forwarded, arbitrary ones dropped (`e2e_forwards_safe_response_headers_only`).
- DEM-01 — de-mask tolerates `[EMAIL 1]` / `[email-1]` / `[ EMAIL_1 ]`; unknown bracketed text is left as-is.
- ADV-01 — evasion recall: broadened phone shapes + IBAN-before-word are caught; obfuscated emails are a **documented gap** (`tests/adversarial.rs`).
- IBAN-04 — the IBAN span never absorbs a trailing ALL-CAPS word (code-review guard).

### M2 — hybrid detection (unstructured entities)
- OVL-01 — shared overlap resolution: structured PII wins a span an ML entity overlaps; non-overlapping spans all survive in reading order (`src/pii/overlap.rs`).
- CMP-01 — `CompositeDetector` merges structured + (fake) NER entities; the deterministic layer wins overlaps (`src/pii/composite.rs`).
- NER-CORPUS-01 — `tests/corpus/ner_cases.json` parses; every positive label is Person/Org/Location.
- NER-REG-03 — the **deterministic layer never emits** Person/Org/Location over the whole NER corpus (structured recognizers never guess names; `anubi` stays untagged).
- DEM-02 — response `content` array is de-masked symmetrically (bare-string elements too).
- DEC-01 — `label_to_kind` strips `B-`/`I-` and maps PER/PERSON/ORG/LOC/GPE → the right `PiiKind`; `O`/unknown → `None` (`src/pii/ner_decode.rs`).
- DEC-02 — `decode_entities` merges a multi-token entity into one span via offsets.
- DEC-03 — a `B-` label splits two adjacent same-type entities (New York | London).
- DEC-04 — `O`/unknown-only token streams yield no entities.
- DEC-05 — `underscore`-BIO (`B_LOC`/`I_LOC`) still splits adjacent entities (M2-R5).
- DEC-06 — `validate_label_count` accepts a match, rejects a mismatch (M2-R3).
- FC-04 — a required detector error blocks the request (400); block reason carries no input text (M2-R2, `required_detector_error_blocks_request_fail_closed`).
- FC-05 — `build_detector(true)` is fatal when no NER can be present; `build_detector(false)` is Ok (M2-R1).
- CMP-02 — required detector error propagates through the composite; a `FailOpen`-wrapped one is swallowed (M2-R2).
- OVL-02 — an NER span enclosing a structured span is dropped whole; only the structured span survives (M2-R7).
- DEC-07 — a token offset that includes the leading space (SentencePiece `▁`) is trimmed so the span is exact (M2-R6, `leading_space_in_token_offset_is_trimmed`).
- DEC-08 — `label_to_kind` maps granular PII labels (GIVENNAME/SURNAME→Person, CITY→Location) and keeps structured ones (EMAIL/PHONE/…)→None.
- EVAL-01 — `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d): scores a live model against `ner_cases.json` through the hybrid resolver. **Run 2026-07-12** (XLM-R int8 vs Piiranha — see DEVLOG); run with `-- --ignored --nocapture` once a model is configured (`NER_MODEL_REPO` auto-download or `NER_MODEL_PATH` explicit).
- EVAL-02 — `tally_counts_duplicates_as_multiset` (`tests/ner_eval.rs`, non-network): the harness scores TP/FP/FN as a multiset, so a duplicate `(kind, text)` can't inflate recall (M2-R10).

### M8 — GLiNER (contextual / open-label, feature `onnx`)
- GLINER-DEC-01…12 — `src/pii/gliner_decode.rs` unit tests (**no model**): the regex word splitter (reference `\w+(?:[-_]\w+)*|\S` — separates trailing punctuation, handles multibyte), `gliner_label_to_kind` (incl. `phone number → Phone`, `address → Location`; email stays `None`), `parse_gliner_labels` **fails closed** on an unmappable label, and `decode_spans` (sigmoid threshold, greedy non-overlap, multi-word spans, out-of-range/malformed spans skipped not indexed).
- GLINER-WIN-01…05 — `src/pii/gliner.rs` unit tests (**no model**): `GlinerParams::from_config_json` (fields + published defaults) and `plan_word_windows` (single window, token-budget split with an N-word overlap, an oversized single word in its own window, empty).
- SMOKE-GLINER *(live, `--features onnx`, `#[ignore]`d)* — `tests/gliner_eval.rs::smoke_gliner_detects_known_entities`: the **S0 validation** — proves the tensor contract on the **real** int8 model (Mario Rossi→Person, Google→Org, Milano→Location, `020 7946 0958`→Phone). If `words_mask` / `span_idx` / the logits layout were wrong these would not decode. Gated on `GLINER_MODEL_PATH` / `GLINER_TOKENIZER_PATH` / `GLINER_CONFIG_PATH`.
- GLINER-CHUNK *(live, `#[ignore]`d)* — `gliner_chunks_a_large_field_and_keeps_recall`: exercises the **multi-window** path on a field far longer than one window — asserts it runs **without error** (chunking + the M8-R1 `max_len` choke-point guard don't crash/spuriously-fail) and keeps recall on a well-positioned entity. The guard against the seq≈384 all-low-logits zone + the `MAX_WINDOW_TEXT_TOKENS`=100 recall cap (M8-R1/R2).
- GLINER-EVAL *(live, `#[ignore]`d)* — `evaluate_gliner_against_corpus`: scores GLiNER int8 through the hybrid on `ner_cases.json`. **Run 2026-07-19** — the *not-a-successor* verdict (Person 0.58 < XLM-R 0.83; Loc 0.91 / Org 1.00 at the measured-optimal threshold 0.15). Numbers + sweep in DEVLOG.
- GLINER-INERT-01 *(live, `#[ignore]`d)* — `gliner_placeholder_inertness_canary`: the **M5-R4 "GLiNER especially"** canary. Confirms GLiNER **does** tag `[KIND_N]` placeholders (unlike int8 XLM-R) yet `mask_all` **converges** on placeholder-dense text (`keep_maskable` drops exact hits by construction + S4 keeps it off later passes). The durable model-swap canary for GLiNER.

### M8.1 — national phone recognizer (opt-in per `PII_LOCALES`, `phonenumber`-validated, `src/pii/recognizers.rs`)
Default build, no model. The FP-prone tier's first recognizer: a loose `0`-trunk regex + a per-locale `is_valid()` validator. A miss here is a leak, so the cases are adversarial.
- PHONE-NAT-01 — `gb_national_phone_detected_when_gb_enabled` / `de_national_phone_detected_when_de_enabled`: the domestic shapes the universal (US 3-3-4 / `+CC`) arm misses — GB 3-4-4 / 5-6 mobile / compact / freephone, DE geographic / mobile / compact — mask as `Phone` under `["gb"]` / `["de"]`.
- PHONE-NAT-02 — `national_phone_is_gated_by_pii_locales`: a GB 3-4-4 number (`020 7946 0958`, which the universal arm can **not** catch and whose spaces break the contiguous 9/11-digit ID patterns) is masked by **nothing** with `gb` off (`["it","us"]`, `["de"]`) and only masked with `gb` on — proving the gate is real, not a coincidental universal hit.
- PHONE-NAT-03 — `national_phone_validator_rejects_lookalikes`: **compact** `0`-leading runs of phone length (`0000000000`, `0123456789`, `0999999999`) reach the validator (the universal arm can't touch a separator-less run) and are rejected by `is_valid()` — the M4-R1 FP concern, defused. Compact on purpose: a 3-3-4-shaped junk number would be masked by the pre-existing universal arm regardless.
- PHONE-NAT-04 — `national_phone_does_not_swallow_an_adjacent_number`: two real GB numbers separated by a **single space, no word** (`020 7946 0958 0161 496 0000`, both `0`-leading) must yield **two** spans — the bounded-group regex can't grab them as one over-long span that `is_valid` would then reject (a leak).
- PHONE-NAT-05 — `national_phone_validators_accept_reals_reject_junk`: direct `gb_phone_valid` / `de_phone_valid` unit tests (reals accepted, compact junk rejected). Documents that the validator is **not a locale discriminator** — a GB mobile also validates as DE (numbering plans overlap; privacy-safe) — while a London geographic number is *not* a valid DE number.

### M9 — execution providers & the provider benchmark (feature `onnx`, no model needed)
`src/pii/onnx.rs::ep_tests` — unit, runs in plain `cargo test-onnx`. The runtime knob is parsed here, and the two *failure modes must stay distinct*: a **typo** fails startup, a real-but-absent accelerator **falls back to CPU**. Conflating them would silently run CPU while the operator believed a GPU was engaged.
- EP-01 — `parses_known_providers_case_insensitively_and_trims`: every provider name plus the aliases (`dml`, `trt`, `vino`) and `""` → `Cpu`, case- and whitespace-insensitive.
- EP-02 — `an_unknown_provider_is_an_error_not_a_silent_cpu`: a typo (`directl`, `gpu`) is an `Err` naming the bad value — never a silent CPU run. Pins that **`vulkan` is rejected**: it is *not* an ONNX Runtime backend (WebGPU is the nearest cross-vendor EP), so it must not look valid to an operator who assumes it is.
- EP-03 — `as_str_round_trips_through_parse`: `parse(as_str(p)) == p` for every variant, so the value logged at startup is always one an operator can paste back into `NER_EXECUTION_PROVIDER`.

- EP-04 — `requesting_an_unavailable_provider_yields_cpu_as_the_effective_one`: `Cpu` produces no dispatch (never an accelerated build), and every accelerator *does* produce one — availability is ORT's answer to give, not ours to pre-empt by skipping the attempt.

**CLI surface (`tests/binary_smoke.rs`, spawns the real `.exe`).** M9 gave the binary its first arguments, and unexpected input must refuse rather than serve.
- CLI-01 — `unknown_cli_argument_is_refused_and_never_binds` (M9-R4): `--bench-provider` (the singular typo) exits **non-zero** *and* the port **never becomes reachable**. Both halves are load-bearing — an exit-code-only assertion would pass for a binary that served for ten seconds first, which is exactly how the defect was found.
- CLI-02 — `help_prints_usage_and_exits_zero` (M9-R4): `--help` prints usage listing the flags and exits 0, so the natural way to ask "what does this take?" is not itself an error.
- CLI-03 — `bench_providers_never_advises_an_ep_feature_rebuild` (M9-R14, M9-R16): `--bench-providers` output must contain **no `ep-*` feature name**. The platform's accelerator is wired per-target, so naming a feature sends the operator to rebuild what they already have — and `ep-directml` is Windows-only, so naming it on macOS/Linux points at a backend their hardware cannot provide. **Runs in both builds on purpose**: M9-R14 fixed the `onnx` branch and left `cfg(not(onnx))` saying it verbatim, and the defect survived a whole review round precisely because M9-R14's proposed test was never written. A guard that covers only the branch a finding quoted is how the *same* defect comes back in its sibling.

**Execution-provider fallback (`tests/ner_perf.rs`, `#[ignore]`d, needs the real model).**
- NER-EP-01 — `ner_ep_01_unavailable_provider_falls_back_to_cpu_for_the_whole_pool` (M9-R1, M9-R2): requesting `rocm` (never present on this project's platforms) must (a) still **load** — a privacy proxy starts even when the named GPU isn't there; (b) report **`Cpu`** as the effective provider, which is what the startup log's `provider=` field is derived from; and (c) hold for `pool_size = 2`, pinning that the pool is **homogeneous** — otherwise round-robin dispatch would make the backend a per-request variable and detection could silently vary between requests. It also asserts the post-fallback detector still detects: falling back is a latency change, not a masking one.

> **The `provider=` log line is verified by hand, not by NER-EP-01 — know the seam.** NER-EP-01 asserts `detector.provider()`; the defect M9-R1 actually described was a **log statement** reading the wrong variable, and no test reads a log line. The gap is one function wide (`server::log_ml_detector_loaded`) and it is the function whose whole job is honesty, so re-verify it by **booting the real binary** whenever that path is touched. The three shapes worth running (reviewer round 2, 2026-07-20): an available accelerator at `pool_size = 1` → `INFO … provider="directml"`; the same at `pool_size = 2` → *two* `NER session initialized` lines, proving the pool is homogeneous rather than one GPU session and one CPU session; and an unavailable one (`NER_EXECUTION_PROVIDER=rocm`) → **`WARN … loaded on a FALLBACK backend … provider="cpu" requested="rocm"`**. The `WARN` level and the `requested=` field are the contract, not decoration: an `INFO` line naming the effective provider without naming the request is indistinguishable from a box that simply has no GPU.

> **Not covered by an automated test, on purpose:** whether an accelerator is *faster* is hardware-specific and cannot be asserted in CI. That question is answered by the shipped `--bench-providers` mode (`src/pii/bench.rs`), which measures the **model × provider** matrix on the operator's own machine. Likewise, **no accelerator has been run against the cross-thread determinism guard** (`m7_r3_intra_threads_changes_speed_not_detection`) — recording that it has *not* been run is the deliverable, and ARCHITECTURE's provider table marks the difference between *benchmarked* and *trusted* (M9-R7). And **no test reaches a build script**: which ONNX Runtime distribution a target downloads is decided in `ort-sys`'s `resolve_dist`, so a per-target EP feature's real effect is only visible in a build log — check it there, per M9-R13.

### M2.5 — HuggingFace model management (feature `onnx`, no network)
- HF-01 — `parse_id2label` orders labels by class id (not JSON order), matches the XLM-R config, and **fails closed** on non-contiguous ids / missing `id2label` / non-integer keys / **empty `id2label`** (`empty_id2label_is_an_error`, M2.5-R2) (`src/pii/hf.rs`).
- HF-02 — `standard_hub_cache_dir` yields the conventional `<home>/.cache/huggingface/hub` tail (not `hf-hub`'s `/tmp` fallback).
- HF-03 *(manual / live)* — the opt-in download itself is exercised only by the `#[ignore]`d `ner_eval` harness with `NER_MODEL_REPO` set (see EVAL-01).

### M2.6 — debug & observability (off by default)
- DBG-01 — `PII_DEBUG_SKIP_DEMASK` (via `Config.debug_skip_demask`): the client receives the placeholder (`[EMAIL_1]`), not the value, while the upstream still saw the masked body (`e2e_debug_skip_demask_returns_placeholders_to_client`).
- DBG-02 — **log-safety regression** (`tests/log_safety.rs`, `crate_logs_carry_placeholders_never_raw_pii`): captures the crate's `trace` logs during a real PII round-trip and asserts the `trace!` masked-body log shows `[EMAIL_1]` and **never** the raw value (while the reply did carry the de-masked value) — the never-log-raw-PII rule is now automated, not just inspection.

### M3 — streaming (SSE) & multi-provider routing
- STR-01 — `split_demaskable` (`src/stream.rs`): holds back only a suffix that could still become a placeholder (unclosed `[…`), emits a closed `[…]` or clearly-non-placeholder `[…`, and is length-bounded.
- STR-02 — `SseDemasker` reassembles a placeholder **split across SSE events** (`[EMA` + `IL_1]` → the real value); `[DONE]` and non-`data:` lines pass through; bytes chopped mid-line are reassembled.
- STR-03 — e2e `e2e_streaming_deanonymizes_split_placeholder` (`tests/proxy_e2e.rs`): a `stream:true` round-trip through a mock SSE upstream that fragments the reply — the client receives the de-masked value with no `[EMAIL_1]` leak, and the upstream saw only the masked body.
- STR-04 — `tool_call_arguments_split_across_deltas_are_restored` (`src/stream.rs`): a placeholder split across streamed `delta.tool_calls[].function.arguments` deltas is reassembled and de-masked (per-`(choice, tool)` hold-back buffer).
- STR-05 — `mid_stream_upstream_error_becomes_terminal_sse_event` (`src/server.rs`): a mid-stream upstream error is turned into a terminal `event: error` (buffered content flushed first) with a clean end, injected via a synthetic stream (no HTTP round-trip).
- STR-06 — `e2e_streaming_non_sse_error_falls_back_to_json` (`tests/proxy_e2e.rs`, M3-R1): a `stream:true` request the upstream answers with a `429 application/json` error reaches the client as that JSON error (correct status + content-type), not force-wrapped as SSE.
- STR-07 — **JSON-aware de-mask (M3-R2):** a value with a `"` de-masked into a tool-call `arguments` field stays valid inner JSON — `demask_json_string_keeps_inner_json_valid` (vault), `tool_call_arguments_demask_stays_valid_json` + `content_demask_is_not_json_escaped` (buffered, `src/pipeline/privacy.rs`), `tool_call_arguments_deanon_stays_valid_json` (streaming, `src/stream.rs`).

### M4 — locale coverage (structured recognizers, `src/pii/recognizers.rs`)
Three tiers: universal (always), national IDs (always, off `PII_LOCALES`), FP-prone (opt-in via `PII_LOCALES`).
- LOC-01 — `national_ids_are_always_on`: a US-only set still masks an IT Codice Fiscale + a GB NINO, and an unrelated `fr` locale still masks a US SSN — proves national IDs are **not** gated by `PII_LOCALES` (M4-R1).
- LOC-02 — `italian_codice_fiscale_detected_by_default`: a CF is detected as `NationalId` by `new()`.
- LOC-03 — `uk_nino_prefix_rules_reject_lookalikes` (M4-R2): the NINO prefix rules reject look-alikes (`PO123456A`, `GB…`, `DA…`) while a valid NINO still masks — required now that NINO is always-on.
- LOC-04 — `spanish_dni_nie_check_letter` + `spanish_dni_detected_and_lookalike_rejected`: ES DNI/NIE mod-23 check letter (valid masked, wrong-letter rejected).
- LOC-05 — `french_nir_key_check` + `french_nir_detected`: FR NIR mod-97 key (valid masked, wrong-key rejected).
- LOC-06 *(live, `#[ignore]`d)* — **10-language NER validation** via `ner_eval` over `multilingual_preview` (ar/de/es/fr/lv/nl/pt/zh): Person 0.83 / Org 1.00 / Loc 0.91 (recorded in DEVLOG 2026-07-13).
- LOC-07 — national-ID checksums (all reject a wrong-checksum look-alike, accept a valid one): `italian_codice_fiscale_checksum_rejects_broken` (M4-R3), `german_steuer_id_check`, `dutch_bsn_and_portuguese_nif_check`, `latvian_code_random_form_and_reject`, `china_resident_id_check`.
- LOC-08 — `french_nir_special_month_is_not_missed` (M4-R5): an INSEE special-month NIR (month `20`) is still detected.
- LOC-09 — `iban_per_country_length_gates_confidence`: an IBAN is `Verified` only when mod-97 **and** its country's ISO 13616 length both hold; a wrong-length one is `Structural`.
- LOC-10 — `numeric_email_local_part_is_not_hijacked_by_a_national_id`: a numeric national ID that is a *substring of an email local part* merges **into** the email (their union *is* the email span), so `123456789@x.com` masks as one `[EMAIL_1]`, not a fragment. *(Mechanism history: originally a priority ranking (`Email` on top), then the M4-R9 **containment gate**; since **M4-R15 the gate is gone** — `Email` is the lowest structured priority, the union-merge produces the span, and `overlap::name_of`'s "union is *exactly* an `Email` span" exception keeps the label. The asserted behavior never changed.)*
- LOC-11 — `e2e_masking_is_provider_agnostic` (`tests/proxy_e2e.rs`): the same request via the `openai` vs `anthropic` presets yields a byte-identical masked body upstream.
- LOC-12 — `email_beats_a_card_iban_or_secret_local_part` (M4-R7, *mechanism superseded twice — M4-R9, then M4-R15*): a card/IBAN/secret that is only a substring of an email local part (`4111111111111111@x.com`, `DE89…@x.com`, `sk-…@x.com`) masks as one `Email`, never fragmenting off the `@domain`. **Since M4-R15 this is a *naming* rule, not a gate:** the enclosed span merges into the email (identical union either way), and `overlap::name_of` gives the `Email` label back whenever the union is *exactly* an `Email` span. `Email` remains the **lowest** structured priority (the M4-R7 top-priority reorder leaked on partial overlaps — see M4-R9).
- LOC-13 — `bare_numeric_national_ids_are_masked_by_design` (M4-R6): an arbitrary checksum-valid 9-digit number (`524287244`) is masked *by design* (accepted over-mask, privacy-first), while a checksum-failing neighbour (`524287245`) is left in clear — the checksum still filters the majority.
- LOC-14 — `de_steuerid_rejects_a_consecutive_triple` (M4-R8): a DE Steuer-ID with a 3× digit in three *consecutive* positions is rejected even with a valid checksum; the same digits non-consecutive are accepted.
- LOC-15 — **Email enclosure vs *partial* overlap (M4-R9, mechanism now M4-R15's naming rule)** — the two shapes must resolve *oppositely*, so both are pinned:
  - `grouped_forms_attached_to_a_domain_do_not_leak` (recognizers): a **space-grouped** card / IBAN / NINO glued to `@domain` (`4111 1111 1111 1111@example.com`) only *partially* overlaps the email, so the union covers **both** and is named by the checksum-backed structured kind — never fragmented, and the trailing email is masked too.
  - `grouped_pii_glued_to_a_domain_leaks_nothing` (`tests/adversarial.rs`): the same shapes asserted on the **masked body** — no card/IBAN/NINO digit group survives in clear (`!masked.contains("4111")`, …). Also pins the enclosure case (a *continuous* card glued to a domain leaks neither digits nor domain).
  - `email_enclosing_a_structured_span_names_the_union` (`src/pii/overlap.rs`): enclosure at resolver level — the union *is* the email span, so `name_of` returns the `Email` label. *(Its old counterpart `email_partially_overlapping_a_structured_span_loses_it` was retired with the drop-the-loser resolver at `0ed0c7c`; the partial-overlap side is now pinned by `partially_overlapping_structured_spans_merge_into_their_union` — see LOC-16.)*
  - **Non-vacuity:** re-raising `Email` above the structured kinds makes these fail (the pre-fix M4-R7 leak).
- LOC-16 — **Union-merge: no abandoned bytes (M4-R10 / M4-R11)** — a *partial* overlap must mask **both** spans, not pick a winner:
  - `a_partially_overlapping_email_is_never_abandoned` (recognizers): `555 867 5309john.doe@example.com` → one `[PHONE_1]` over the union; the email's **local part** is never left in clear (a bare left-over `@domain` would be acceptable — a local part is not).
  - `a_span_enclosed_by_an_email_is_never_stranded` (recognizers): a card/secret **enclosed** by an email, with a phone partially overlapping that email — the enclosed span must still be covered (under the old gate it was *deleted* before the priority sort, and when the enclosing email then lost, it was masked by *nothing*; the union-merge never deletes it). *(Renamed from `a_span_deleted_by_the_containment_gate_is_never_stranded` in M4-R18 — the gate is gone.)*
  - `partially_overlapping_pii_abandons_no_bytes` + `masking_partially_overlapping_pii_still_round_trips` (`tests/adversarial.rs`): the reviewer's exact repros asserted on the **masked body**, plus exact round-trip through the merged union.
  - `partially_overlapping_structured_spans_merge_into_their_union`, `a_chain_of_overlaps_merges_to_a_fixpoint`, `a_span_enclosed_by_an_email_is_still_covered_and_names_the_union` (`src/pii/overlap.rs`): the resolver itself — union, transitive fixpoint, and enclosure coherence.
- LOC-17 — **ASCII word boundaries (M4-R13)** — `cjk_prose_does_not_hide_structured_pii` (recognizers), `structured_pii_is_detected_in_cjk_and_cyrillic_prose` + `ascii_token_anti_false_positive_guarantee_survives_the_ascii_boundary` (`tests/adversarial.rs`, on the **masked body**), and the `non_ascii_scripts` corpus category. A Unicode `\b` made every anchored recognizer inert in CJK prose; `(?-u:\b)` fixes it without weakening the anti-FP rule inside ASCII tokens. **Non-vacuity:** restoring `\b` makes the detector return `[]` on the Chinese card sentence and fails corpus CJK-01 / RT-05.
- LOC-18 — **Union naming + merge fail-safe (M4-R15 / M4-R14)** — `an_enclosed_secret_names_the_union_not_the_phone`: a `Secret` enclosed by an email names the union (it used to be reported as `[PHONE_1]` — no leak, but the model and the audit log were told the wrong kind). `a_union_ending_inside_a_multibyte_char_still_covers_every_constituent`: a union endpoint falling inside a 3-byte `€` widens out to the char boundary instead of degrading to a single constituent (which would abandon bytes — the very leak class the resolver exists to prevent).

### Algorithmic complexity — masking must stay **linear on both axes** (`tests/complexity.rs`, M4-R19 + M4-R24)

**Availability is a privacy property here.** A proxy that is down forwards nothing — and protects nothing.
The masking path has **two** independent size axes, and closing one says nothing about the other:

| Axis | The quadratic | Guard |
|---|---|---|
| **field size *n*** | M4-R17's fix made candidate generation see *every* overlapping match — O(n²) on the two patterns with no length bound (`Email`, `Secret`). 151 s on a 200 KB field. | DOS-01…03 |
| **entity count *k*** | `Vault::mask` spliced right-to-left with `replace_range`; each splice memmoves the tail, so k entities in n bytes shift Θ(n·k). A field of many *small* values has k growing with n → Θ(n²). **~7 min** on 13 MiB of repeated emails. | **DOS-04** |

Both sat on the **unauthenticated** masking path, under the 16 MiB body limit.

These are **timing** guards: they run the work on a worker thread against a wall-clock budget, so a
super-linear regression fails the suite in **seconds** instead of hanging for hours. Each one *also*
asserts the value is still masked and round-trips, so a "fix" that buys speed with blindness fails too.

- DOS-01 — `a_huge_email_local_part_does_not_blow_up`: `"a"*1_000_000 + "@b.co"` → detected, masked whole
  (`[EMAIL_1]`), exact round-trip, well inside budget.
- DOS-02 — `a_huge_run_of_secret_prefixes_does_not_blow_up`: `"sk-"*350_000` (~1 MB) → one placeholder, no
  `sk-` fragment in clear.
- DOS-03 — `a_long_row_of_card_groups_stays_linear`: 1 MB of 4-digit card groups. The **bounded**
  recognizers *keep* the M4-R17 rescan, so this pins the other half of the claim — a card matches at every
  group boundary (~200 K overlapping windows), yet each match is ≤ 19 chars, so the scan stays O(n · 19).
  **Non-vacuity:** on the pre-fix code DOS-01/02 time out while DOS-03 passes — exactly the split the
  finding predicted.
- **DOS-04** — `masking_many_small_entities_stays_linear` (M4-R24): **600 K entities** (`"a@b.co "`×600 K,
  ~4.2 MB — one entity every 7 bytes), asserting the *splice* completes in budget and still masks +
  round-trips exactly. It times `Vault::mask` **alone**, because that is the code under test and it makes
  the guard decisive rather than marginal: ~0.2 s linear against ~52 s quadratic (~250×, debug profile).
  Detection and de-masking are linear in *n* and pinned by DOS-01…03, so they stay outside the clock. It
  also asserts `entities.len() == reps` — **if the corpus ever coalesced back to k≈1, the guard would be
  blind again and would say so.**
  **Non-vacuity:** on the pre-fix splice DOS-04 times out **while DOS-01/02/03 all pass** — the same
  before/after split M4-R19 used, and a precise reproduction of the blind spot below.
- **Coverage is preserved, and that is tested separately** (dropping candidates is how M4-R17 made PROP-03
  pass *vacuously* — this could not be asserted, it had to be argued and pinned):
  `an_email_chain_leaves_nothing_detectable` (a chained `a@b.com@c.com`, whose second email reaches past
  the first — the **fixpoint** catches the remainder one pass later) and
  `a_secret_hidden_inside_a_secret_is_still_covered` (a nested `sk-…` is always *contained* in the outer
  match, so the rescan added nothing there to begin with).

> **The rule for a new recognizer:** if its match length has **no upper bound**, it must not take
> `Scan::Overlapping`. Bounded recognizers rescan; unbounded ones rely on the fixpoint.

> **The rule for the guards themselves (M4-R24):** **vary the entity *count*, not just the field *size*.**
> DOS-01…03 each pin **one** entity (DOS-03's card row coalesces to k≈1), so they held *k* fixed while
> scaling *n* — and a per-entity quadratic lived right underneath them, invisible, for four milestones. The
> smoking gun: 13.4 MiB as **one** email masks in 219 ms; the **same** 13.4 MiB as many small emails took
> **421 s**. Identical byte size; the only variable is *k*. This is the M4-R13 lesson — *"a corpus has a
> shape, and that shape is a blind spot"* — recurring **on the DoS guards themselves**. Ask what the guard
> holds *constant*, not only what it varies.

> **What is NOT covered here (stated so nobody reads more into it).** These guards bound the **structured**
> (default-build) path. The optional `onnx` NER is a separate, opt-in cost with its own scaling behavior and
> its own guards — it is **chunked** and measured linear in field size (PERF-01; `tests/ner_perf.rs` +
> `src/pii/onnx.rs::chunk_tests`), and it is a **recall** mechanism only, never leak-relevant, because
> structured PII is detected over the whole field, unchunked. The mechanism lives in
> [`ARCHITECTURE.md`](ARCHITECTURE.md) → *NER chunking (M5, PERF-01)*; this file only points at it.
>
> **Why a pointer and not a restatement (M5-R1):** the earlier version of this box asserted the NER was
> unchunked and quadratic in field size — a claim M5 both **disproved** (the real failure was an outright
> position-embedding overflow, not a slowdown) and **fixed**. It went stale precisely *because* the same
> claim lived in two files and only ARCHITECTURE was updated. **One home for a fact** is the project's rule
> for review findings; it applies to design claims identically.

### Fail-closed — masking (`src/pii/anonymizer.rs`, M4-R20)
- FC-06 — `mask_all_blocks_when_it_cannot_reach_a_fixpoint`: exhausting `MAX_MASK_PASSES` must return
  `Err` (→ `PrivacyStage` blocks, 400), **not** forward the text. A synthetic `NeverConverges` detector
  always reports the *first character* as PII, so masking it exposes a new first character and the fixpoint
  is never reached. The bound gives *eventual* convergence, never convergence **within four passes**, so
  the fixpoint is **confirmed**, not assumed. Also asserts the error carries **no input text** (DBG-02's
  never-log-raw-PII rule) and that a normally-converging detector still returns `Ok` — so the guard can't
  pass by breaking masking for everyone.
- FC-07 — `mask_all_converges_even_if_a_detector_tags_its_own_placeholders` (M5-R4 / CC-08): **placeholder
  inertness by construction.** A `CompositeDetector` pairs the real recognizers (which mint `[EMAIL_1]`)
  with a `TagsPlaceholders` detector that re-tags every `[KIND_N]` as a `Person` — the exact NER pathology
  that would loop `mask_all` to a 400. `mask_all` must still converge (`"mail bob@test.com"` → `"mail
  [EMAIL_1]"`, restored intact), because `keep_maskable` drops the placeholder detections before masking.
  The unit `is_placeholder_token_matches_only_our_own_tokens` pins the filter's boundary: our tokens (incl.
  the tolerant `[email-3]`, `[ PERSON 2 ]`) match; a foreign `[TODO_1]`, a partial match, two tokens, and
  real PII do **not** — so it can never drop a genuine value.
- FC-08 — the **runtime suppression canary** (M7-R21), log-captured on a **scoped** subscriber:
  `converging_mask_emits_the_value_free_suppression_canary` runs the FC-07 composite (which suppresses a
  re-tagged placeholder) on a *converging* input and asserts the `debug!` fires carrying
  `placeholder_tags_suppressed` and **no** raw value; `a_clean_convergence_stays_silent` asserts a no-
  suppression run emits nothing. Together they pin that the counter is **value-free** and fires on the
  converging path whenever a detector tags placeholder-shaped text. **Post-S4 caveat (M7-R23):** since the
  NER runs only on pass 0, this counter can *not* observe a model re-tagging masking's own output — it
  reflects only placeholder-shaped text already in the **raw field** (e.g. a client echoing placeholders in
  Run ON). The durable model-swap canary is **NER-INERT-01**, which runs the NER directly on placeholder text.
- FC-09 — **S4: the NER runs only on pass 0** (`src/pii/anonymizer.rs`, CC-05/CC-08). A `Fragmenter` fake
  models the sub-word fragmentation the real NER does (tags the first alphabetic char → masking exposes the
  next, forever). `s4_a_re_running_fragmenter_exhausts_the_bound` (idempotent=false → the default
  `redetect` re-runs) must **400** — the bug; `s4_an_idempotent_ner_lets_masking_converge` (idempotent=true
  → `redetect` empty, the NER after pass 0) must **converge** (`"Slack"` → `"[ORG_1]lack"`, masked once) —
  the fix. Deterministic, no model needed. The live counterpart is `ner_perf.rs::
  m7_s4_dense_org_names_converge_instead_of_400` *(`--features onnx`, `#[ignore]`d)* — real XLM-R, dense
  system-prompt text converges instead of 400-ing — a **model-swap checkpoint**, since S4's no-recall-loss
  rests on the NER not exposing new names when a neighbour is masked.
- CACHE-01 — the **S3 detection cache** (`src/pii/cache.rs`, M7.1) is sound and bounded. Unit:
  `a_repeated_field_is_scanned_once_and_the_hit_matches` (a hit returns the *exact* fresh result — the
  no-mask-less guarantee), `an_error_is_not_cached_and_still_fails_closed` (a detector error propagates and
  is never memoized as success), `redetect_is_never_cached` (later fixpoint passes always delegate — they
  run on per-request masked text), `small_fields_are_not_cached` (below the threshold), and
  `a_hot_key_survives_eviction_of_cold_ones` (the two-generation bound is LRU-ish: the hot key stays live
  while cold keys roll off). E2e: `proxy_e2e.rs::e2e_cache_on_a_repeated_large_field_still_masks_both_times`
  sends a byte-identical PII-bearing large field twice with the cache ON and asserts **both** requests mask
  the email — the pipeline-level proof a cache hit never masks less than a fresh scan.

### M5 / M6 — live provider verification

Everything above runs against a **mock** upstream. These are the checks that need a **real
provider**, and they are the only ones that can catch what a mock cannot: a model that
reformats a placeholder, an auth wiring that only fails against the real endpoint, a client
that sends a shape we never imagined.

**The prefix tells you who runs it** — `E2E-INT-*` is automated, `CC-*` is a human at the
keyboard. (An earlier revision filed the manual battery as "E2E-INT-02", which made that
unreadable; the label is retired.)

**E2E-INT-01 — automated**, real-provider smoke over **both routes**: `tests/anthropic_smoke.rs`.
Strictly opt-in (`#[ignore]`d, needs a real credential, one network call each, never in CI):
`cargo test --test anthropic_smoke -- --ignored --nocapture`.

- `e2e_int01_anthropic_real_provider_roundtrip` — the **OpenAI-compat** route
  (`/v1/chat/completions`). This route still ships (Cline / Continue / opencode / Copilot BYOK),
  so it keeps its own live check — the CC battery below does **not** cover it.
- `e2e_int01_anthropic_native_messages_roundtrip` — the **native** route (`/v1/messages`, M6).
  The automated companion to the manual battery.

<a id="cc-battery"></a>
#### CC-01…CC-09 — manual: a real Claude Code session through the native route

Run by a human, not by cargo. Procedure: [`MANUAL_VERIFICATION.md`](MANUAL_VERIFICATION.md).
Workspace + fixtures: [`tools/claude-code-session/`](../tools/claude-code-session/) — open that
directory and Claude Code routes itself through the proxy. It is manual on purpose: it compares
what two runs *logged* and *returned* to a real client, which needs eyes on the trace.

**Every scenario runs twice**, and the pair is the point:

| run | `PII_DEBUG_SKIP_DEMASK` | proves |
|---|---|---|
| **OFF** (normal) | unset | the client gets the **real** values back — the round-trip restores |
| **ON** | `1` | the client sees the **placeholders** — i.e. exactly what the provider received |

Neither alone proves the chain; together, on the same input, they show the value that left
masked is the value that comes back restored. **Both runs also get the DBG-02 grep**: the raw
values must appear in **neither** log.

The mechanics — starting the proxy as the *hybrid* (`NER_REQUIRED=1`, non-negotiable), flipping
the flag, where the trace goes, and the traps that make a run lie to you — are all in
[`MANUAL_VERIFICATION.md`](MANUAL_VERIFICATION.md). This section only says *what* each scenario
asks and *what* must come back.

**How to read a scenario.** *Ask* is what you type into the session. *Upstream* is what the
proxy's `forwarding masked request body upstream` trace must show — **identical in both runs**,
because request-side masking does not depend on the flag; if OFF and ON differ here, that alone
is a finding. *Client* is the only thing the flag changes. Paths are relative to
[`tools/claude-code-session/`](../tools/claude-code-session/).

---

> <a id="cc-prompt-design"></a>
> **Why these read like ordinary work, and why that is the design (M7/S5).** The first version of
> this battery asked things like *"reply with exactly this sentence: contact jane.doe@example.com,
> IBAN …"*. **Claude Code refused — correctly.** It is an *agent with repo context* (it inherited
> this repo's precisely because the fixture workspace lives inside it), and a stranger dictating a
> sentence full of credentials reads as an injection attempt. CC-01 / CC-02 / CC-08 therefore never
> ran at all.
>
> **The fix is not to argue the agent out of its judgement.** A `CLAUDE.md` telling it to comply
> would work and would be the wrong fix: it makes the test a special case of itself, and what ships
> is then verified under a rule no real user has. The fix is to ask for **plausible work** — read
> this file, format this contact, write this note. Which is *also* what real Claude Code traffic
> looks like, so the rewrite makes the battery both runnable **and** more representative. The PII
> still has to travel; it just travels the way it actually does.

**CC-01 — structured PII in a chat prompt**
- **Ask:** `Formatta questo contatto come JSON, senza commenti: Jane Doe, jane.doe@example.com, IBAN IT60X0542811101000000123456.`
- **Upstream:** `[EMAIL_1]` + `[IBAN_1]`; neither raw value anywhere.
- **Client — OFF:** JSON carrying the real email and IBAN. **— ON:** JSON carrying `[EMAIL_1]`,
  `[IBAN_1]`.
- **Proves:** the baseline round-trip on real traffic. *A formatting request is work; the model
  answers it without objection, and the values must still round-trip verbatim inside the JSON.*

**CC-02 — NER entities** *(the one that catches a half-running product)*
- **Ask:** `Scrivi una nota di rilascio di una riga che ringrazia Mario Rossi di Acme S.p.A. (sede di Milano) per la segnalazione.`
- **Upstream:** `[PERSON_1]`, `[ORG_1]`, `[LOCATION_1]`; no `Mario Rossi` / `Acme` / `Milano`.
- **Client — OFF:** a release note naming the real person, org and city. **— ON:** the same note
  carrying the three placeholders.
- **Proves:** **the hybrid is actually running.** Every other scenario would pass with the NER
  off — email and IBAN are *deterministic* recognizers. Only this one fails. If `Mario Rossi`
  appears upstream in clear, that is a **leak**, not a recall nit.
- **Why this ask:** writing a release note is unremarkable work, so the agent just does it — but it
  cannot do it without carrying a Person, an Organization and a Location into the request, which is
  exactly what must be masked.

**CC-03 — a file with PII → `tool_result`**
- **Ask:** `Leggi fixtures/contacts.csv e dimmi quante righe di dati contiene e qual è l'email della prima.`
- **Upstream:** the `tool_result` block carries `[EMAIL_n]` / `[PHONE_n]` / `[IBAN_n]` /
  `[SSN_n]` / `[PERSON_n]`; **no** `bob@test.com`, `555-111-2222`, `123-45-6789`, …
- **Client — OFF:** "3 righe, `bob@test.com`". **— ON:** "3 righe, `[EMAIL_1]`".
- **Proves:** the core Claude Code workflow — its day is tool calls on files, not chat. The live
  twin of E2E-02.

**CC-04 — `tool_use.input` carries a PII value back to the client**
- **Ask:** `Leggi fixtures/contacts.csv e scrivi in scratch/first-contact.txt soltanto l'email della prima riga di dati, senza altro testo.`
- **Upstream:** the `tool_use.input` the model emits contains `[EMAIL_1]`, never the real value.
- **Client — OFF:** `scratch/first-contact.txt` ends up containing **`bob@test.com`** — the
  de-mask restored the tool argument before the client acted on it.
  **— ON:** the file contains **`[EMAIL_1]`** — literally what the model emitted.
- **Proves:** the `tool_use.input` de-mask (live twin of INT-03). The ON run is the clearest
  visual proof in the battery: the placeholder is *on disk*.

**CC-05 — multi-turn determinism**
- **Ask (turn 1):** `Ricorda questa email, ti servirà dopo: bob@test.com` — then **(turn 2):**
  `Qual è l'email che ti ho dato? Rispondi solo con l'email.`
- **Upstream:** turn 2 re-sends the history and must mask `bob@test.com` to **`[EMAIL_1]` again**
  (same value → same token, in a fresh per-request vault).
- **Client — OFF:** `bob@test.com`. **— ON:** `[EMAIL_1]`.
- **Proves:** deterministic assignment across the stateless re-send a real client does every turn.

**CC-06 — a secret in a file**
- **Ask:** `Leggi fixtures/deploy-config.env e dimmi il valore di APP_SECRET e di OPS_CONTACT.`
- **Upstream:** `[SECRET_n]` + `[EMAIL_1]`; **no** `sk-ant-api01-…`, no `AKIA…`, no `ops@corp.com`.
- **Client — OFF:** the real secret and email. **— ON:** `[SECRET_1]`, `[EMAIL_1]`.
- **Proves:** the SECRET recognizer on real traffic — the category the old proxy's ML model
  missed, and the reason secrets are deterministic here rather than left to a model.

**CC-07 — thinking blocks survive the replay** *(never yet exercised live)*
- **Ask:** with extended thinking on — `Pensa passo passo: leggi fixtures/contacts.csv e dimmi
  quale contatto ha un IBAN tedesco.` Then ask **a follow-up in the same session** (any question)
  so the thinking block is **replayed** with its signature.
- **Upstream:** the replayed `thinking` block is byte-identical to what the model produced
  (re-masking a placeholder is a no-op), so Anthropic accepts its `signature`.
- **Client:** the follow-up answers normally. **A `signature` / `invalid_request_error` on the
  second turn is the failure this scenario exists to catch.**
- **Proves:** M6's `thinking` invariant — masked on the way up, never de-masked on the way down,
  which is what keeps the signature valid.

**CC-08 — a long streamed answer**
- **Ask:** `Leggi fixtures/contacts.csv e genera, per ogni contatto e per ognuno dei 10 mesi da gennaio a ottobre, una riga di promemoria del tipo "<mese>: scrivere a <email>". Solo le righe.`
- **Upstream:** `[EMAIL_1]`…`[EMAIL_3]` — the emails arrive in the **tool result** (the CSV read),
  so this also covers re-anonymization on the way *up*; no raw address anywhere.
- **Client — OFF:** ~30 lines each carrying an **intact** address — none split, mangled, or left as
  a placeholder. **— ON:** the same lines carrying `[EMAIL_n]`.
- **Proves:** the Anthropic SSE hold-back over many real deltas — a placeholder split across
  `text_delta`s is reassembled every time, not just once. **30 restorations of a value the model
  never saw** is the point: one lucky reassembly proves nothing.
- **Why this ask:** generating a reminder list from a CSV is ordinary work, and it yields a long
  streamed answer that repeats the same placeholders dozens of times — the shape CC-08 needs, with
  none of the "repeat after me" framing that got the original refused.

**CC-09 — PII through an MCP SQL tool** *(the old proxy's TC-04)*
- **Setup (once, out-of-band — NOT through the proxy):** run `fixtures/cc09-setup.sql` against a
  **throwaway** DB (a local SQLite file is enough) to create `cc09_customers` with one synthetic row
  (email/phone/ssn/card/iban/secret). This puts the PII in the query **result**, not the query **text**.
  Never point this at a real table — synthetic data only.
- **Ask:** `Con il tool MCP SQL, esegui SELECT * FROM cc09_customers e mostrami il risultato.`
  (equivalently `fixtures/customer-lookup.sql`, which is now exactly that PII-free query).
- **Upstream:** the `tool_result` carries `[EMAIL_n]`, `[PHONE_1]`, `[SSN_1]`, `[CARD_1]`, `[IBAN_1]`,
  `[SECRET_1]`; none of the six raw values.
- **Client — OFF:** the real row. **— ON:** the row of placeholders.
- **Proves:** PII arriving through an **MCP tool result** — a path the proxy never sees coming and
  cannot special-case — is masked like any other. Verified live **2026-07-18** (DEVLOG): DBG-02 = 0 on
  all six values.
- **Why the query text must be PII-free (the original TC-04 shape got this wrong).**
  `SELECT 'bob@test.com'… FROM DUAL` carries the PII as **literals in the query text**, so the agent
  *reading* the `.sql` masks them **before** the query runs and the `tool_result` path is never
  exercised. The PII must ride in the **result** (a table), not the text (2026-07-18).

---
- **E2E-02 / E2E-04** — done; see *End-to-end* above.
- **PERF-01** — system-level load harness, the companion to `tests/complexity.rs` (which pins
  the masking *algorithm's* complexity, no HTTP):
  - `tests/perf.rs::healthz_stays_responsive_under_concurrent_masking_load` — 8 concurrent
    50 K-entity requests (~350 KB each) in flight; `/healthz` (an async-only route) must keep
    answering fast regardless, proving the M4-R19 `spawn_blocking` architecture claim
    end-to-end rather than by hand-measurement. Measured (debug profile): `/healthz` answers in
    ~40 ms while the load is in flight (budget: 2 s).
  - `tests/perf.rs::streaming_throughput_of_repeated_placeholder_restoration_stays_within_budget`
    — ~150 KB of a placeholder repeated ~6000 times, streamed as small SSE fragments; the
    incremental de-anonymizer must restore every occurrence. Measured: ~0.75 s (budget: 20 s).
  - `tests/ner_perf.rs::m4_r21_the_fixpoints_second_pass_roughly_doubles_ner_inference`
    (`--features onnx`, `#[ignore]`d, needs a configured model) — measures `Vault::mask_all`'s
    ≥2 detector calls against a single `detect()`, confirming the M4-R21 finding's ~2× factor
    with a live number rather than the one-off measurement recorded when it was closed
    (measured here: ~1.8–3×, consistent with the ~2× recorded in `docs/reviews/M4.md#m4-r21`).
  - `tests/ner_perf.rs::onnx_ner_latency_and_recall_across_field_sizes` (same gating) — the NER
    chunking measurement; see *Decisions & open points* below and `docs/ARCHITECTURE.md` →
    *NER chunking (M5, PERF-01)*. Confirmed **linear** latency (448 ms / 2.07 s / 7.53 s at
    64/256/1024× a repeated sentence, debug profile) with recall intact, replacing what used to
    be an outright ONNX error past ~500 tokens.
  - `src/pii/onnx.rs::chunk_tests` — unit tests (no model needed) for `chunk_char_ranges`, the
    pure token-window → char-range function chunking is built on. Pins the exact bug the live
    measurement first caught: the sequence's final token is always the closing `</s>`, whose
    offset is the sentinel `(0, 0)` — mistaking it for the real text end silently dropped the
    last window (a third of the entities lost on one measured input), so
    `the_last_window_reaches_the_true_text_end_not_the_closing_token_sentinel` pins it directly.

<a id="m7-latency"></a>
### M7 — the latency harness (`tests/m7_latency.rs`, `--features onnx`, `#[ignore]`d)

**Why it is a separate file from `ner_perf.rs`, and the lesson it encodes.** `PERF-01` measured a
*repeated synthetic sentence* and concluded "linear" — true, and beside the point: it never measured
a **real client's payload**. Then M7's own opening numbers were taken on a blob *densely packed with
names* and reported ~0.96 s/KB — the same mistake, one level down. **A corpus has a shape, and the
shape is the blind spot** (M4-R13 → PERF-01 → here). So this file's fixture is the experiment, and
its shape is **asserted**, not assumed.

- **PERF-M7-01** — `m7_s0_a_realistic_claude_code_turn_measured_per_field`. One realistic turn (112
  fields / **22.3 KiB**) masked with **one vault, field by field**, as production does: one big
  `system`, 10 medium `tools[].description`, 100 tiny `input_schema` descriptions, one ~130 B user
  message carrying all the PII. Reports per-field cost and **fixpoint pass counts**. It **reports,
  it does not assert** — one sample, on a harness whose run-to-run spread is ~40%; the bar lives in
  PERF-M7-05, over repeats (M7-R2).
  - **The fixture's shape guards are load-bearing.** Total size must be 20–50 KB, `system` must be
    exactly one field, and the schema tier must be >50 fields. The first draft tripped the size
    guard at 13.5 KiB (350-byte tool descriptions; real ones are 1–4 KB) — i.e. the guard caught a
    fixture that would have under-measured the product, which is the entire job.
  - **Do not "improve" it with a captured real body.** The trace log has one, but it is already
    **masked**, so its NER pass finds nothing and the measurement lies *optimistically*. Synthesize
    the shape, not the content.
- **PERF-M7-05** — `m7_s2_the_bar_holds_for_every_shipped_shape`. **M7's deliverable, guarded as a
  RATIO**: it measures the **pre-M7 shape (`2×1`) as an in-run calibration leg** and asserts every
  shipped shape is **≥1.5×** it — the single-session default (`NER_POOL_SIZE` unset → 1 × 12, the
  personal shape since the 2026-07-17 flip) *and* the pooled centralized shape (`NER_POOL_SIZE=2` →
  2 × 6), min of 3 reps each. Plus a **loose 15 s** absolute ceiling for order-of-magnitude
  regressions only.
  - **Run it isolated: `--test-threads=1` (M7-R12).** Cargo runs tests concurrently, so the old
    documented command had these benchmarks **measuring each other** — worth **1.50×** on the
    absolute at constant power (4,757 ms isolated → 7,142 ms contended). Three review rounds blamed
    that class of gap on power management before anyone measured the harness itself.
  - **Why a ratio and not the ~3 s bar (M7-R9/M7-R12) — the most useful thing in this catalog entry.**
    Same code, same fixture, same box, two people: **2,462 / 3,943 / 4,724 / 4,757 / 4,841 / 4,933 /
    7,142 ms**, each run internally tight (spread < 7%). **The ordering variable is still not
    identified** — and it is *not* power: the runs once labelled "battery" and "AC" turned out to be
    the same energy-efficiency plan (charger attached or not; M7-R17), so that label ordered nothing,
    which is exactly why a "battery" run could beat three "AC" ones. A wall-clock assert on that is a
    **box-state detector**: red on five of seven runs, and green through a genuine 20% regression. The
    ratio is the part that is about the code: it held ~**1.7–2.3×** across all seven while the absolute
    moved 2.9×. (Quote the guard's **≥1.5× floor**, not a tight band — a faster box compresses the
    ratio toward the floor, so any band keeps being undercut; M7-R18.)
  - **Min-of-N was the wrong fix, and knowing why matters more than the fix.** It answers M7-R2's
    *jitter*, and this is not jitter: all N reps sit inside the regime and agree tightly on the wrong
    number. **Precise, and wrong.** The harness's own footer had already said the drift was *between*
    runs.
  - **What it cannot see, because an honest guard says so (M7-R14).** The 1.5 floor against a worst
    *observed* ~1.7 tolerates a **~13% regression** — materially the blindness the wall-clock bar had.
    The ratio buys **regime-independence, not sensitivity**; it answers the false *positive*, not the
    false *negative*. The floor cannot be tightened toward the worst observation (a fast box
    legitimately compresses the ratio toward it — M7-R18 — so a tighter floor would false-fire), so
    the honest move is to state the limit rather than to imply it away. The **15 s** ceiling is
    order-of-magnitude only — it was 8 s, which fired on the harness's own documented command
    (median 10,391 ms) and blamed the power state for test concurrency.
  - **Its domain (M7-R13 / M7.1).** The guard still **skips below 4 cores and says so**, but the
    2026-07-17 default flip (`pool 2 → 1`) changed *why*. Under the old `pool=2` default `intra`
    floored at 1 there, so the derived default *was* `PRE_M7_SHAPE` and the ratio was 1.0 by
    construction. Under `pool=1` the derivation is `intra = cores`, so that identity holds **only at
    1 core**; between 2 and 3 the derived shapes add threads but too few to clear the 1.5× floor
    reliably, so 4 stays the conservative line. Pinned in
    `onnx::thread_tests::the_default_gives_one_session_the_whole_box`: **the speedup scales with the
    box, and this guard has nothing dependable to say below 4 cores.**
  - **Why both shapes (M7-R1).** The first cut asserted `pool=1` only, while the *server* then
    defaulted to `pool=2` — ~28% headroom on a config nobody ran, none on the one they did. Both now
    resolve through `onnx::resolve_pool_and_intra`, the **server's own** function, so the harness
    cannot drift from production. (Since the flip the default *is* `pool=1`, so PERF-M7-05 now guards
    that default **and** the pooled `NER_POOL_SIZE=2` shape a centralizing operator sets.)
  - **It measures and prints every row before asserting any**, because the first cut asserted inside
    the loop: a failure on the default meant the personal shape never ran and never printed, on a test
    whose entire purpose is the two-row comparison. **A guard must not destroy the evidence needed to
    interpret it.**
- **PERF-M7-02** — `m7_s0_what_the_ner_finds_in_boilerplate_that_has_no_pii`. Prints every entity
  the hybrid finds in text that carries **no PII by construction**. Each hit is a false positive that
  costs its field a **second full NER scan**. Measured: `(Organization, "An")` — a two-character
  fragment of "Anthropic's" — in the system prompt. **This is the test that refuted M7's own premise**
  ("the boilerplate has ~zero PII, so it costs one pass"). Diagnostic: it reports, it does not assert,
  because the *right* number here is a precision question (M4-R6's class), not a latency one.
- **PERF-M7-03** — `m7_s1_how_much_of_the_box_can_one_request_use`. Sweeps `pool × intra`, **3 reps
  per shape**, printing **min / median / spread**. Reports; does not assert, because the numbers are
  box-specific. Confirms scaling is **sublinear** (12 threads → ~2×).
  - **Read the `spread` column, and know it understates the noise (M7-R2).** `spread` is
    within-run; the same configuration also drifts ~40% *between* runs on the reference box. **This
    harness resolves large effects, not small ones**, and its footer says which rows a *mechanism*
    backs. The first cut ran each shape **once** and turned an 18% `1×6`-vs-`1×12` gap into the
    stated conclusion "SMT helps" — which inverts run to run. **SMT is unresolved**; the reps are
    the guard that would have prevented the claim.
  - The pool's inertness at concurrency 1 (`2×1` ≈ `1×1`) is real but should be believed **from the
    code**, not this table: one request occupies one session, so `pool` cannot help it. When the two
    rows differ here, that is the box.
- **PERF-M7-04** — `m7_s1_throughput_under_concurrent_load_must_not_regress`. 4 concurrent turns,
  turns/s per shape. **The guard against optimizing latency by quietly wrecking the shared-proxy
  case** the pool was built for. It is what measured `pool=1` at **−23% throughput** (the reviewer
  independently got −21%), refuting the builder's own "it is not a trade at all". That −23% is
  exactly why the 2026-07-17 default flip to `pool=1` is scoped the way it is: the flip targets the
  **personal** proxy, which has no concurrency to lose that on, and a centralizing operator reclaims
  it with `NER_POOL_SIZE=N` — this test is the reason that stays an override, not the default.
- **THREAD-01** — `src/pii/onnx.rs::thread_tests` (unit, **no model needed**, runs in plain
  `cargo test --features onnx`). Pins the two pure functions the threading rests on, as functions of
  `(pool, cores)` — so the CI runner's core count cannot decide whether they are correct.
  - `derive_intra_threads`: the oversubscription invariant **with its domain** — `pool × intra ≤
    cores` while `pool ≤ cores`, and `intra == 1` beyond it, where the derivation is out of moves.
    **The regimes are split on purpose (M7-R4):** the first version asserted
    `pool * intra <= cores.max(pool)` across both, which passes for `pool > cores` by widening the
    bound to the pool itself — green-lighting 8 threads on a 2-core box under a name claiming the
    opposite. A test may not hide its exception inside a `max`.
  - `resolve_pool_and_intra`: **both** knobs treat `0` and garbage as unset (M7-R5 — M7 shipped that
    guard on the new knob only, leaving `NER_POOL_SIZE=0` safe by two independent clamps while the
    startup log printed `pool_size=0, intra_threads=12`, which no arithmetic reconciles); an explicit
    value wins; and the default is `DEFAULT_POOL_SIZE`, the same constant `server.rs` uses — which is
    what makes M7-R1's harness/server drift structurally impossible.
- **NER-THREAD-01** — `tests/ner_perf.rs::m7_r3_intra_threads_changes_speed_not_detection`
  (`--features onnx`, `#[ignore]`d, needs a model). **`NER_INTRA_THREADS` must change speed, never
  detection** — fingerprints `(kind, span.start, span.end)` over prose, **a field past the
  `MAX_WINDOW_TOKENS` chunking window** (~660 tokens), CJK, and the fragment-prone `"Anthropic's"`
  shape, at intra 1 / 2 / 4 / 6 / all-cores, and asserts every set is **identical**. Measured: **194
  entities**, identical at every count, with a non-vacuity floor so a guard that detects nothing cannot
  pass.
  - **Its chunked input is asserted in *tokens*, through the real tokenizer (M7-R8).** The first cut
    asserted `long_field.len() > 2_000` — **bytes** — for a branch `infer_chunked` takes on **tokens**
    (`> MAX_WINDOW_TOKENS` = 480). The input was 2,360 bytes and cleared it by 18% while being **442
    tokens**: 38 short of the trigger, so the guard covered **zero** chunked inputs — the one case
    M7-R3 named as the whole reason to have it. This is [M5-R10](reviews/M5.md#m5-r10)'s shape a file
    over (*the assert pins a proxy in the wrong unit, not the property the code depends on*), and the
    M4 retrospective's lesson 6: **a quantity a test never varies is a quantity the test cannot see.**
    Third time in this repo. The constant is imported, never hand-copied ([M5-R9](reviews/M5.md#m5-r9)).
  - **Why it exists (M7-R3):** every *recall* guard in this repo pins `intra=1` **on purpose** — a
    score that moves with the runner's core count is worthless — so the one knob M7 changed was the
    one knob nothing exercised. The property is **empirical**: ORT repartitions reduction work across
    threads, float addition is not associative, and the BIO decode is a per-token `argmax` where a
    near-tie flips on nothing but thread count. **This is the guard a DirectML/CUDA EP swap or a
    GLiNER swap must trip over** — see ARCHITECTURE → *NER threading*, and M5-R4 for the same rule
    about the same layer.

### M5 review round 1 — the guards the findings left behind

- **NER-CHUNK-01** — `every_window_is_sliceable_even_when_an_offset_lands_inside_a_multibyte_char`
  (`src/pii/onnx.rs::chunk_tests`, M5-R3). The chunk window edges come from the **tokenizer**, and
  `&input[a..b]` *panics* if one misses a `char` boundary — the one place on the masking path that
  could, while `decode_entities` (M2-R6), `Vault::mask` and `overlap::materialize` all refuse to.
  `chunk_char_ranges` now widens every window through the resolver's own
  `overlap::widen_to_char_boundaries` (M4-R14), so the ranges are sliceable **by construction**.
  Offsets that deliberately cut a 3-byte `€` and a 4-byte `𝄞` in half must still come back
  sliceable. **Carries its own non-vacuity assertion** — it first checks the offsets table really
  does land off a boundary, because a guard that quietly stops exercising its hazard is precisely
  how M4-R13 and M4-R24 survived (*ask what the corpus holds constant*).
- **NER-CHUNK-02** *(compile-time)* — `const _: () = assert!(MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS)`
  and `assert!(CHUNK_OVERLAP_TOKENS < MAX_WINDOW_TOKENS)` (`src/pii/onnx.rs`, M5-R2). Not a test — a
  **build error**. The window must leave headroom for re-tokenization drift, and it must advance or
  chunking wouldn't terminate; get either wrong and the crate does not compile.
- **NER-CHUNK-03** *(live, `--features onnx`, `#[ignore]`d)* —
  `m5_r2_every_retokenized_window_stays_within_the_models_usable_length` (`tests/ner_perf.rs`,
  M5-R2). `MAX_WINDOW_TOKENS` (480) plans windows in the **whole field's** token coordinates, but
  each window is **re-tokenized from its own text** (a middle window needs its own `<s>…</s>`
  framing) — which adds the specials and drifts at the cut edges, so the sequence actually handed to
  the model is `window + specials + drift`. Measured: **481–483 tokens, i.e. always over the
  planning bound**, against a usable ceiling of **512** (`MODEL_MAX_TOKENS`). This drives the
  **real** `chunk_char_ranges` (`pub` for exactly this — a copy in the test would drift from the code
  it guards) with the **real** tokenizer over six adversarial fields (Chinese, Japanese, Cyrillic,
  combining-mark/zalgo, mixed-script, and 4 000 chars of `あ` with no spaces at all), asserting each
  window both chunks (non-vacuity) and stays under the ceiling. `run_and_decode` clamps as a
  last-resort valve; **this is the guard that makes sure the valve never fires.**
- **NER-INERT-01** *(live, `--features onnx`, `#[ignore]`d)* — `m5_r4_the_ner_treats_placeholders_as_inert`
  (`tests/ner_perf.rs`, M5-R4). **Now belt-and-braces, not the sole guarantee.** `Vault::mask_all` masks to
  a fixpoint; placeholder inertness — the reason it converges — is proved **by construction** for the regex
  recognizers, and since CC-08 is **enforced by construction for the NER too**: `keep_maskable` drops any
  detection that is one of our own `[KIND_N]` tokens (FC-07), so a model that tags `[PERSON_1]` can no
  longer stall the fixpoint. This test still asserts XLM-R tags **zero** entities across a 3 040-byte
  placeholder-only field — large enough to exercise the **chunked** path — but its role shifted from *the*
  safety proof to **the model-swap canary**: **GLiNER** ([M8](ROADMAP.md#m8)) is *zero-shot, open-label, context-driven*
  and could read `Contact [PERSON_1] at [ORG_1]` and tag both — this test runs the NER **directly** on
  placeholder text and catches exactly that. Run it on a swap to know **whether** the model leans on the
  filter; correctness never depends on it (S4 converges the fixpoint regardless). **Post-S4 (M7-R23):** the
  runtime `placeholder_tags_suppressed` counter (FC-08) is *not* a substitute — the NER runs only on pass 0,
  so it can't observe a model re-tagging masking's own output; it only flags placeholder-shaped text already
  in the raw field. This test is that canary; the counter is a weaker, separate signal.
- **MSRV-01** *(CI)* — the `msrv` job (`.github/workflows/ci.yml`, M5-R5) **builds** the crate on the
  declared floor, **1.89**, with `--features onnx`. Before this, `rust-version` was a claim nothing
  checked — and it was **false**: the declared `1.82` could not even parse the dependency tree.
  **A declared MSRV with no job building on it is not a floor; it is a comment shaped like a guarantee.**
  > **One floor, not two.** The measured floors *do* differ by feature set — 1.86 default, 1.89 onnx —
  > but the **shipped product runs with `onnx` on** (that is the hybrid: structured recognizers *and*
  > NER). A "default-build MSRV" would be a promise about a configuration nobody deploys: a second number
  > to keep honest, buying nothing. So the manifest declares the floor of the *real* product. The default
  > build still happens to compile on 1.86; we simply don't promise it.

### M6 — native Anthropic `/v1/messages` (Claude Code passthrough)

The native schema is a **new shape on the masking path**, so a missed field is a leak. Coverage is pinned at
three levels: the request/response walk (`src/pipeline/privacy.rs`), the SSE rewriter (`src/stream.rs`), and
the full HTTP round-trip against a **mock native upstream** (`tests/anthropic_messages_e2e.rs`). The mock
echoes the received (masked) body under `upstream_received` and the auth headers under `upstream_auth`, both
outside the `content[]` restore path — so a test inspects exactly what the provider saw. Every round-trip
e2e first asserts the echo is present, so a `!contains(raw)` check can never pass **vacuously** on a 401/400
body (the bug that bit the first draft — a credential-less forward 401s, and `null.to_string()` contains no
PII). Placeholder-**presence** asserts are on the **specific masked field** (or a token like `[PHONE_1]` /
`[EMAIL_6]` **absent from the augmentation prompt**), not the whole body — the prompt itself contains
`[EMAIL_1]` / `[IBAN_1]` as examples, so a whole-body `contains` would be weaker than it reads (M6-R4).

**Unit — request/response walk (`src/pipeline/privacy.rs`)**
- ANT-01 — `anthropic_masks_every_text_bearing_field`: a value in **every** place M6 scans (top-level `system`,
  a `content` string, `text` / `tool_use.input` object leaves incl. nested / `tool_result` / `thinking` blocks,
  `tools[].description` + `input_schema` description) is masked; non-text leaves (image `data`, a numeric tool
  arg, a `thinking.signature`, `redacted_thinking.data`) are untouched. **A miss here is a leak.**
- ANT-02 — `anthropic_masks_document_text_and_content_sources_title_and_context` (M6-R1): a `document` masks
  **every** text-bearing part — a `text` source's `data`, a **`content`** source's nested block array, and the
  `title` / `context` metadata — while a base64 file source and a nested image stay opaque.
- ANT-09 — `anthropic_unknown_document_source_type_fails_closed` (M6-R1): a `document` `source.type` we don't
  model → `Err` (→ 400), the same fail-closed rule as an unknown block type, one level down.
- ANT-03 — `anthropic_masks_nested_tool_result_block_array`: `tool_result.content` as a block array (text /
  image / bare string) is recursed into.
- ANT-04 — `anthropic_unknown_block_type_fails_closed_without_echoing_it`: an unknown block type → `Err`
  (→ 400), and the reason carries **no** client-controlled value (never-log-raw-PII — the type is
  attacker-influenced).
- ANT-05 — `anthropic_block_without_type_and_missing_messages_fail_closed`: a typeless block, a missing
  `messages`, a non-array `messages`, **and a `system` array object with no `text`** (M6-R2) each fail closed.
- ANT-06 — `anthropic_augmentation_covers_absent_string_and_array_system`: the augmentation is created (absent
  `system`), appended (string), or pushed as a trailing text block (array).
- ANT-07 — `anthropic_response_demasks_text_and_tool_use_input_but_not_thinking`: the buffered demask restores
  `content[].text` and `tool_use.input` leaves, and **leaves `thinking` with placeholders** (keeps its
  `signature` valid on replay — the M6 invariant).
- ANT-08 — `anthropic_tool_use_input_demask_stays_valid_json_through_a_quote`: a value with a `"` restored into
  a `tool_use.input` string leaf survives serialization (serde re-escapes; `input` is a real object, so the
  **plain** demask is correct — unlike OpenAI's JSON-encoded `arguments`).

**Unit — Anthropic SSE (`src/stream.rs`)**
- ANT-SSE-01 — `anthropic_text_delta_split_across_events_is_restored`: `[EMAIL_1]` split across two
  `content_block_delta` `text_delta` events is reassembled and de-masked; no placeholder leaks.
- ANT-SSE-02 — `anthropic_input_json_delta_split_is_restored_and_valid`: a placeholder split across
  `input_json_delta.partial_json` fragments is restored and the reassembled JSON parses (JSON-aware demask).
- ANT-SSE-03 — `anthropic_held_back_tail_is_flushed_before_its_block_stop`: a block ending on a
  partial-placeholder tail flushes it at `content_block_stop`, and the flushed delta is emitted **before** the
  stop frame (a delta after the stop is protocol-invalid — the `event:`-line hold-back guards frame ordering).
- ANT-SSE-04 — `anthropic_non_delta_events_pass_through`: `message_start` (and other non-delta events) pass
  through untouched.

**End-to-end — mock native upstream (`tests/anthropic_messages_e2e.rs`)**
- ANT-E2E-01 — `messages_buffered_roundtrip_masks_upstream_and_restores_to_client`: multi-PII round-trip — the
  upstream saw only placeholders, the augmentation merged into the top-level `system`, and the client got the
  real values back in `content[].text`.
- ANT-E2E-02 — `messages_masks_system_content_blocks_and_tool_definitions`: PII in `system` (block array),
  content blocks (`text` / `tool_use.input` / `tool_result`), and tool defs (`description` + `input_schema`)
  is all masked upstream.
- ANT-E2E-10 — `messages_content_source_document_is_masked_upstream` (M6-R1): a `content`-source `document`
  (a nested text-block array — the shape the first cut forwarded in clear) is masked in place; the assertion
  is on the document's own text block, so it can't be satisfied by the augmentation prompt (M6-R4).
- ANT-E2E-03 — `messages_unknown_block_type_fails_closed_400`: an unknown content block → 400
  (`error.type == "blocked"`), nothing forwarded.
- ANT-E2E-04 — `messages_route_is_404_when_provider_is_not_anthropic`: with `provider = openai`, a native body
  to `/v1/messages` → 404 (the route is registered only for `anthropic`).
- ANT-E2E-05 — `messages_client_bearer_is_forwarded_verbatim_never_as_x_api_key`: a client
  `Authorization: Bearer` (OAuth) is forwarded verbatim and wins over the configured proxy key, and is **never**
  copied into `x-api-key`.
- ANT-E2E-06 — `messages_proxy_key_is_injected_as_x_api_key_when_client_sends_none`: with no client credential,
  the configured proxy key is injected as `x-api-key`, and a default `anthropic-version: 2023-06-01` is added.
- ANT-E2E-07 — `messages_no_credential_returns_401_without_forwarding`: no client auth and no proxy key → 401
  (`error.type == "unauthorized"`), nothing forwarded.
- ANT-E2E-08 — `messages_client_anthropic_version_is_forwarded_not_overridden`: a client `anthropic-version`
  passes through unchanged (the default only fills an absent one).
- ANT-E2E-09 — `messages_streaming_deanonymizes_split_placeholder`: a `stream:true` round-trip through a mock
  Anthropic SSE upstream that fragments the reply — the client receives the de-masked value with no `[EMAIL_1]`
  leak, and the upstream saw only the masked body.

> **The live Claude Code smoke has run, and it held** (2026-07-16): a real session round-tripped through the
> proxy against real Anthropic — native route, auth passthrough, in-place masking, response de-mask and DBG-02
> (zero raw PII in the trace, on real traffic). That closed M6's own gate. See DEVLOG 2026-07-16.
>
> **What it did *not* exercise is the hybrid.** No model was configured, so the run was silently structured-only:
> email and IBAN masked because they are *deterministic* recognizers, while the NER never ran. Verifying the
> hybrid is the [CC battery](#cc-battery) (CC-01…CC-09 × Run OFF/ON, `NER_REQUIRED=1` — the flag that makes that
> silent downgrade fatal).
>
> **M7 unblocked it on both counts (2026-07-16), and it is now the last thing before `1.0.0`.** The prompts that
> Claude Code refused as injection attempts are [rewritten as ordinary work](#cc-prompt-design), and a realistic
> turn masks in ~2.5 s at the default (`NER_POOL_SIZE` unset) instead of the claimed 27 s, so 9 scenarios ×
> 2 runs is practical. What remains is
> irreducibly manual: a human, a live key, and eyes on two traces. The automated mock coverage above remains the
> permanent guarantee; the battery is what proves it on real traffic.

### Dependency footprint (M2.5-R1)
- DEP-01 — `tests/dependency_footprint.rs` (`default_build_excludes_the_onnx_and_hf_stack`): `cargo tree` on the **default** features must contain no `hf-hub`/`hf-xet`/`aws-lc`/`ort`/`tokenizers` — the ONNX/HF stack (heavy, native) stays behind the `onnx` feature so the shipped default build is native-dep-free.

### Decisions & open points
- **Coverage scope — DECIDED (2026-07-13; see ROADMAP M4).** *Language (NER):* XLM-R's
  10 languages (ar/de/en/es/fr/it/lv/nl/pt/zh). *Structured — three tiers:* universal
  (email/IBAN/card/secret) always on; national IDs (US SSN, IT CF, GB NINO, ES DNI/NIE,
  FR NIR, DE Steuer-ID, NL BSN, PT NIF, LV code, zh Resident ID — full list in the mapping
  table above) **always on regardless of `PII_LOCALES`**; FP-prone recognizers (national *phone*
  formats) opt-in via `PII_LOCALES` — GB/DE since M8.1 (`phonenumber`-validated), other codes not yet.
  Phone: US + `+CC` are universal.
- **Placeholder format — DECIDED: `[KIND_N]`** (e.g. `[EMAIL_1]`), ASCII. Tests
  still assert invariants (raw absent, typed placeholder present, exact roundtrip)
  rather than literal tokens, so they stay robust to future tweaks.
- **IBAN validation strictness** (open) — mask any IBAN-shaped string
  (privacy-first, catches synthetic values) vs require a valid mod-97. Current
  lean: structure-based detection, mod-97 as a confidence signal only.
- **SECRET patterns** (open) — which key formats to cover (OpenAI `sk-…`,
  Anthropic `sk-ant-…`, AWS `AKIA…`, generic high-entropy tokens?).
- **NER field-size limit — DECIDED (M5, PERF-01): chunk, don't just measure.** `OnnxNerDetector`
  fed a field to the model as one sequence; past the model's `max_position_embeddings` (514 for
  the picked XLM-R int8) the ONNX graph's position-embedding lookup went **out of range** —
  measured as an outright `Expand` op error, not the suspected quadratic slowdown. Any field
  over ~500 tokens (roughly 2 KB of prose) failed NER entirely, a hard **block** under
  `NER_REQUIRED`. Fixed with overlapping-window chunking (`src/pii/onnx.rs`); see
  `docs/ARCHITECTURE.md` → *NER chunking (M5, PERF-01)*.

## Running

- `cargo test` — unit + property + integration (from M1 onward), structured-only.
- `cargo test-onnx` — the same suite **plus** the NER path. A `.cargo/config.toml` alias for
  `--features onnx --target-dir target/onnx`: the hybrid builds into its **own** directory, so a
  later plain `cargo test` cannot overwrite the hybrid binary with a structured-only one (they
  otherwise share `target/debug/llm-proxy-pii-rust.exe` — the footgun that once made a live
  verification test half the product; see [`MANUAL_VERIFICATION.md`](MANUAL_VERIFICATION.md)).
  Run it from the repo root: `--target-dir` is relative to the cwd.
- Live-model tests (EVAL-01, the NER-CHUNK / NER-INERT / perf guards) are `#[ignore]`d and need a
  configured model — `cargo test-onnx --test ner_perf -- --ignored --nocapture --test-threads=1`.
  **`--test-threads=1` is part of the recipe for anything that measures time (M7-R12):** cargo runs
  tests concurrently by default, so without it the benchmarks measure the product *against other
  copies of themselves* — 1.50× on the reference box, at constant power. Recall guards don't care;
  latency guards do, and M7 spent three review rounds blaming power management for it.
- End-to-end against a mock provider — harness added in M1.

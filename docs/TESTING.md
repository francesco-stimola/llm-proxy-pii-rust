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
  masked, and round-tripped. Split across three cases in `tests/pii_properties.rs`: **PROP-01a**
  (detected), **PROP-01b** (masked, no raw value survives) and **PROP-01c** (the round-trip is
  exact). Named individually because CAT-01 checks the id a test *declares*, and a compressed
  catalogue entry is exactly how a missing one hides.
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
  > **The pair still has one hole, and it is worth knowing before you trust it.** PROP-03 quantifies over
  > **accepted** candidates and PROP-04 over what is **still detectable**. Neither sees a value that a
  > recognizer matched, the *validator* then **rejected**, and a later mask **truncated** — the rejected
  > match is outside PROP-03's set, and the surviving fragment is by definition undetectable, so PROP-04 is
  > satisfied by it. That is not hypothetical: it is the shape of [M10-R1](reviews/M10.md#m10-r1). The
  > property that would close it is neither "a winner covers every candidate byte" nor "nothing detectable
  > remains" but **"no byte of a real value remains"** — for a value the detector *did* see, in any form,
  > including one it declined. Any new recognizer whose regex can match a **superset** of the real value
  > (greedy groups, un-anchored starts) needs that third check; the ones whose matches are pinned to where
  > a value begins do not.

- **PROP-05 (M11 Track A) — `a_vat_number_round_trips_exactly_whatever_surrounds_it`: is there an
  input where a VAT number is detected but **not restored, or restored wrong**?** `VAT-13` pins the
  round trip for two hand-written strings and `VAT-18`/`VAT-19` pin it over HTTP; this quantifies
  over **arbitrary neighbours**, which is where truncation and adjacency defects actually live — a
  masked span that eats one byte too many, or a placeholder a neighbouring digit run makes
  ambiguous to the de-masker. Every case carries one of five **real published** VAT numbers, so the
  property can never be satisfied by "nothing was detected": the assertion that the known value is
  *gone from the masked text* is the non-vacuity check and the privacy claim at once. The random
  neighbour is a mix of valid and invalid, prefixed and bare.
  > **It failed on its second case, and the defect was in the generator — worth recording because
  > the distinction is the whole point.** With `before = "a"` the text began `a00905811006`, and
  > `(?-u:\b)` then correctly refuses to match: that is `VAT-08`'s subject, and *not detecting* is
  > the right answer. The property was asserting the known value is always detectable, which tests
  > the generator rather than the product. The known value is now separated by punctuation;
  > arbitrary adjacency is still exercised around the **neighbour**, where it is interesting.
  > The failing seed is checked in to `pii_properties.proptest-regressions`, so that case replays
  > on every run and the separator cannot be quietly removed.
  >
  > **What it does not claim.** The PROP-03/PROP-04 note above describes a third property — *no
  > byte of a real value remains, including one the validator declined* — needed by any recognizer
  > whose regex can match a **superset** of the real value. Every VAT pattern is pinned at both
  > ends with `(?-u:\b)` and none sets `shrink_on_reject`, so by that note's own criterion they are
  > in the group that does not need it. This is the exact-restore property, not that one.

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
- E2E-05 (M10) — the validation-budget refusal, asserted **where the client reads it**
  (`e2e05_budget_refusal_reaches_the_client_intact_and_carries_no_input_bytes`): a 256 KB field of
  distinct phone-shaped groups → `400`, `error.type = "blocked"`, and `error.message` that (a)
  names the budget, (b) carries the actionable clause M10-R27 added — *retrying unchanged fails
  identically*, plus the `LIMIT` — and (c) carries **no digit run except the two integers the code put there**. The
  counting mock upstream proves the request was never forwarded. **(c) is the load-bearing one**:
  this message is the single string the codebase deliberately builds from request bytes, so it is
  where the never-log-raw-PII bar is easiest to breach by accident. Added by M10-R32, whose point
  was that DOS-06's assertions pass byte-for-byte on the message M10-R27 *replaced*, and that the
  survival of the string through `Display` → `ctx.block` → `error_response` was pinned nowhere.
  **(c) is phrased positively, and that was M10-R40:** it first read *"no digit run from the body
  appears, unless it is shorter than four digits"* — and that exemption disabled the assertion
  exactly where the interesting leak sits, since three digits is what a truncation orphans (M10-R1's
  `912`). *A guard phrased "nothing forbidden appears" should be re-phrased "only these appear"
  whenever the allowed set is small and known* — here it is two integers, so the exemption is not
  needed and cannot hide anything.
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

> **The IBAN pattern is uppercase-only, and nothing here says so or checks it (M11-R10, open).**
> `[A-Z]{2}\d{2}[A-Z0-9]{11,30}` means **one** lowercase letter anywhere drops the whole span:
> driven through the real binary, `it60x0542811101000000123456`, the space-grouped lowercase
> form, **and** `IT60x0542811101000000123456` — an otherwise-uppercase IBAN with a single
> lowercase BBAN letter — all reach the upstream **in clear**, while
> `IT60X0542811101000000123456` is masked. `iban_mod97`'s own doc says it folds letters to
> uppercase, i.e. the *validator* was written for input the *regex* can never deliver.
> **Unlike the VAT tier's version of this, the rule here is load-bearing and worth keeping:**
> case-folding the continuous arm costs **+931 masked spans** over 341.1 MB of third-party
> source (2 → 933 hits — hex digests, base64, `Ed25519PublicKey`), and IBAN has no hard
> checksum gate, so every one of those is masked. The cheap way to buy the coverage without the
> cost is measured too: require a **non-canonical-case** rendering to be `Verified` (mod-97 +
> the ISO 13616 length) rather than `Structural`, and **0 of the 931** survive — leaving the
> uppercase path's deliberate "a structurally-valid IBAN is masked even if mod-97 fails" rule
> (M4) untouched. Recorded here rather than fixed, because it changes product-visible coverage.
> The tier is **inconsistent** on this axis, which is the part that makes it an accident rather
> than a policy: the national-ID recognizers (Codice Fiscale, ES DNI/NIE, CN resident id) all
> fold case and mask either way; IBAN and the six VAT patterns do not.

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

### M8.1 / M10 — domestic phone recognizers (`phonenumber`-validated, `src/pii/recognizers.rs`)
Default build, no model. A loose regex proposes candidates and `is_valid()` decides — **precision is the validator's job, not the regex's**. A miss here is a leak, so the cases are adversarial; an over-mask here is a *functional* break (a masked port inside `tool_use.input`), which is why PHONE-OM below measures it on real traffic rather than on look-alikes we imagined.
- PHONE-NAT-01 — `gb_national_phone_detected_when_gb_enabled` / `de_national_phone_detected_when_de_enabled`: the domestic shapes the universal (US 3-3-4 / `+CC`) arm misses — GB 3-4-4 / 5-6 mobile / compact / freephone, DE geographic / mobile / compact — mask as `Phone` under `["gb"]` / `["de"]`. **These two are the M8.1 regression guard:** M10 changed the dispatch shape and the region set, and GB/DE must keep exactly the behaviour M8.1 measured.
- PHONE-NAT-02 — `national_phone_is_gated_by_pii_locales`: a GB 3-4-4 number (`020 7946 0958`, which the universal arm can **not** catch and whose spaces break the contiguous 9/11-digit ID patterns) is masked by **nothing** with no enabling region configured, and masked with `gb` on. **The "off" case is `["us"]` since M10** — it used to be `["it","us"]`, and that is precisely the defect M10 closed: both codes mapped to no recognizer, so the assertion passed while the shipped default masked no domestic number at all. `it` now resolves to a real region whose plan accepts that number, so leaving it here would assert the opposite of what it reads as. Also pins that an unshipped code (`zz`) contributes nothing rather than silently falling back to the default set.
- PHONE-NAT-03 — `national_phone_validator_rejects_lookalikes`: **compact** `0`-leading runs of phone length (`0000000000`, `0123456789`, `0999999999`) reach the validator (the universal arm can't touch a separator-less run) and are rejected by `is_valid()` — the M4-R1 FP concern, defused. Compact on purpose: a 3-3-4-shaped junk number would be masked by the pre-existing universal arm regardless.
- PHONE-NAT-04 — `national_phone_does_not_swallow_an_adjacent_number`: two real GB numbers separated by a **single space, no word** (`020 7946 0958 0161 496 0000`, both `0`-leading) must yield **two** spans — the bounded-group regex can't grab them as one over-long span that `is_valid` would then reject (a leak). Its sibling `adjacent_national_phones_are_both_masked_by_the_fixpoint` (M8-R8) pins the shadowing shape at the `mask_all` level.
- PHONE-NAT-05 — `national_phone_validators_accept_reals_reject_junk`: direct `national_phone_valid(Id, …)` unit tests (reals accepted, compact junk rejected). Documents that the validator is **not a locale discriminator** — a GB mobile also validates as DE (numbering plans overlap; privacy-safe) — while a London geographic number is *not* a valid DE number.
- **PHONE-NAT-06 (M10) — `the_default_configuration_detects_a_domestic_number_in_every_shipped_region`: the floor the whole milestone exists to establish.** Builds the detector the way a proxy started with **no environment** does, and asserts a real number from each of the nine regions is masked. The regression it stops is the one that shipped for two milestones and that *every other phone test was structurally blind to*, because they all passed their region in explicitly.
- PHONE-NAT-07 (M10) — `the_default_region_set_is_the_vetted_table`: spelling out every region gives the same result as the default, so `PII_LOCALES` really is an override of a live default and not the only path to coverage.
- **PHONE-NAT-08 (M10) — `the_un_anchored_family_never_reaches_a_trunk_prefix_region`: the single most important precision decision, and the easiest to undo by accident.** libphonenumber accepts a national number *with or without* its trunk prefix, so validating un-anchored digit groups against DE/FR/NL/GB asks "could this be a same-area local dial in Germany?" — true of an enormous slice of ordinary numeric text (measured: DE alone turned 7 of 24 digit-shaped non-phones into `Phone` spans). This asserts those four never see an un-anchored candidate.
- **PHONE-NAT-09 (M10-R1) — `adjacent_phones_leave_no_digit_of_either_number`: the predicate *is* the finding.** Its sibling PHONE-NAT-04 asserts `detect(masked).is_empty()`, and that is **satisfied by** the defect it needs to catch: when an un-anchored candidate starts mid-value, masking *truncates* the neighbour, and the orphaned digits are not detectable — which is exactly why they survived. "Nothing detectable survives" and "no byte of a real value survives" are different properties, and only the second is the privacy guarantee. Runs on the **default** region set (testing only `["gb"]` tests only the case where the M8-R8 fixpoint argument still holds).
- **PHONE-NAT-10 (M10-R13) — `a_valid_number_is_never_lost_between_the_regex_and_the_validator`: differential recall.** For a candidate **generated from a shape family's own grammar**, if `is_valid` accepts it for a region that *declares that family*, the detector must produce a `Phone` span covering the whole thing. Anything less is a **miss**. **Its predecessor asserted the same idea over 30 hand-written literals and could not fail** — they were all domestic renderings, so all ≤ 13 digits, and the defect (a digit-count pre-filter that ignored the international prefix and country code `phonenumber` also strips) lived at 14–15. *An assertion made only where it cannot fail is not an assertion.* Three details are load-bearing: the premise is **per family** (asking "valid for *any* region" would report the deliberate per-region shape restriction as a miss); each generated string is checked to **match its family whole** (the first generator emitted a four-pair French form where the family needs five, then reported the result as a detector bug); and an `accepted > 100` floor stops it passing by never reaching its assertion. Sized for the debug profile — 3,000 rounds took 93 s.
  > ⚠️ **What its name claims, it does not test — [M10-R13](reviews/M10.md#m10-r13) is open.** Those 30 literals are all domestic renderings, hence ≤ 13 digits by construction, and the gate's mask covers 5–13: the assertion is made only on inputs where it cannot fail. It does not hold in general, because `parse` strips an international prefix and a bare country code before validating, so a real number can carry more digits than the mask allows. *An invariant is only ever as strong as the set it quantifies over* ([M4-R17](reviews/M4.md#m4-r17)) — and a hand-listed set quantifies over what the author already believed. **The form that works is differential:** generate candidates from each shape family's own regex and assert `gate(s) || !any(is_valid(s))`. That version fails on today's code within a few thousand samples, and it is the only form that survives a libphonenumber metadata update.
- **PHONE-NAT-11 (M10-R3) — `every_declared_shape_is_needed_by_a_real_rendering`.** Each shape a region declares widens that region's candidate set, so a shape listed "for symmetry" costs false positives for no coverage — which is what handing China the Italian-mobile `LongBlock` rendering did (0.250 of the offsets pool). Removing any declared shape must change what a rendering of that country detects. **Named for what it reads:** the renderings are a list in `recognizers.rs`, not the corpus — a unit test cannot read `tests/corpus/pii_cases.json`, and the earlier name claimed otherwise.
- **PHONE-NAT-12 (M10-R3) — `known_over_masks_are_still_over_masks`.** Three digit runs a real numbering plan accepts (`512 1024 2048` — Suzhou; `28 01 2026` — a Latvian mobile; `02 2026` — Milan), asserted to *still* be masked. They are pinned rather than deleted from the negative corpus, because quietly dropping a case that stopped passing is how a measured cost becomes an unmeasured one — and if one stops being masked, someone re-reads the published numbers instead of finding them silently stale.

### M10 — domestic phone coverage, over-mask and measurement
- **PHONE-COV — `phone_regions_all_have_corpus_cases` (`tests/pii_corpus.rs`): a region cannot be switched on silently.** Enumerates the regions the **code** enables (`PHONE_REGIONS`) and fails if any lacks a positive *and* a negative corpus case — coverage derived from the enabled set, not from a list in the test. Checks the other direction too: a case whose `locale` names a region the code does not enable runs, matches nothing in particular, and reads as coverage it isn't. The corpus itself (`national_phone`, 35 positives / 20 negatives) carries per region every area-code length that country really uses, plus mobile and toll-free, and that country's own look-alikes.
- **PHONE-OM — `the_realistic_turn_yields_exactly_the_expected_phone_spans` (`tests/phone_overmask.rs`): the over-mask guard, on text nobody curated.** The negative corpus contains only the false positives *we thought of*, and M10 widened the candidate set twice over. So this runs the shipped default over the **M7 latency fixture** — a real ~22 KiB Claude Code turn already in the repo, written for a different purpose — and asserts the `Phone` spans are exactly the expected set (today: **empty**). The fixture moved to `tests/common/m7_turn.rs` so both this (default build) and `m7_latency.rs` (`onnx`-only) use *one* copy. Non-vacuity is asserted three ways: the fixture must still be >20 KB, its **shape** is pinned here (M10-R12 — `m7_latency.rs`'s full shape assertion is `#[ignore]`d *and* `onnx`-gated, so it never runs in CI), and a sibling positive control splices in **one number per shape family**. That last part is the M10-R7 fix and it is load-bearing: with a single trunk-anchored control, deleting both un-anchored families left every assertion green — fewer recognizers cannot find more, so the empty expectation still held — while the two families the guard exists to bound were gone.
- **PHONE-BUD (M10-R28 / M10-R30) — `a_real_claude_code_turn_spends_almost_none_of_the_request_budget` (`tests/phone_overmask.rs`): the headroom the threshold rests on, measured instead of asserted.** `MAX_PHONE_VALIDATIONS_PER_REQUEST` is defended by the claim that ordinary traffic cannot come near it, and M10 made that claim without being able to show it — the allowance was per *call*, `mask_all` re-minted it up to five times per field, so "the budget" was not a number any single quantity had, and nobody noticed for three rounds. This charges **every field of the 22 KiB turn against one budget**, exactly as the request path does, and pins the total under 1% of the allowance. Measured: **0 units** — the fixture has no phone-shaped candidate at all. The assertion is an order of magnitude rather than the digit, because what must never regress is the *margin*; a fixture-exact constant would fail on any fixture edit and teach nothing.
- **PHONE-EVAL *(`tests/phone_eval.rs`, `#[ignore]`d)* — the measurement that *is* M10's deliverable.** `phone_precision_per_region_and_for_the_union` scores recall and false positives per region and for the union, over 20 curated negatives plus ~433 generated digit-shaped non-phones. **FP is reported per category (dates · tables · ports · sizes · offsets · money · codes · refs), never blended** — one rate over a pool whose composition you chose is a number about the pool. `phone_latency_per_enabled_region` measures ms/turn for 0…9 enabled regions over the same 22 KiB turn. Run it `--release --test-threads=1` (M7-R12: precision is build-independent, milliseconds are not). Results and the decision they drove: DEVLOG 2026-07-29 and ARCHITECTURE → *Domestic phone coverage*.
  - **Two assertions inside the harness, both from M10 round 1.** (i) The pool must **reach every shape family** before the harness reports on one — the first version's non-date entries almost never began with a 2–3-digit token, so half of what M10 added was structurally untestable and its `0.000` meant *unmeasured*, not *clean*. (ii) `union_hits == ∪ singles` is asserted, not printed: under this dispatch it is a **structural identity** (the validator is `.any()` over a superset), so a number that can only be zero belongs in an assertion — it was being read as evidence that adding a region is marginally free, which it is not.


### M11 Track A — VAT / tax identifiers (`src/pii/recognizers.rs`, default build, no model)
Always-on, checksum-gated, and modelled on the `PHONE-NAT` corpus rule unchanged: **a category
ships when it is measured**, so five countries ship (🇮🇹 🇩🇪 🇬🇧 🇳🇱 🇵🇹) and three that are real but
unverified here (🇪🇸 🇫🇷 🇱🇻) do not. The new kind is `PiiKind::TaxId` → `[TAXID_n]`.

**Read VAT-14 first.** The whole tier hangs on one collision: a bare Partita IVA is `\d{11}`, and
two *other* always-on tiers already claim that shape.

- **VAT-01 — `italian_piva_accepts_real_published_numbers`: the anchor for everything else.** Six
  real published P.IVAs (ENI, Ferrari, TIM, Luxottica, Enel, Stellantis Italy) against the mod-10
  position-doubling check. Six independent real check digits is what distinguishes *an
  implementation of the scheme* from *a plausible transcription of it* — one hand-picked number
  would prove neither, and a corpus generated from the validator would prove nothing at all.
- **VAT-02 — `vat_check_digits_reject_a_moved_digit`: the negative half.** Every shipped country's
  real number with its final digit moved by one, plus wrong lengths. Without it the recognizers
  would be "accepts anything of the right length" — the shape M4-R1 named FP-prone, and the reason
  this tier is checksum-gated at all.
- **VAT-03 — `vat_numbers_are_detected_for_every_shipped_country`: end to end, not validator-only.**
  Real VIES-form numbers through the whole recognizer set, so the regex, the ASCII word boundaries
  and the overlap resolver are all in the path.
- **VAT-04 — `unmeasured_vat_countries_are_absent_rather_than_guessed`: the gap is asserted.** ES,
  FR and LV VAT numbers are real and well-formed and are **not** recognized, because their
  checksums are not measured here. Asserting the absence keeps it a documented decision rather than
  something a future reader finds and "fixes" by guessing an algorithm.
  **The ES arm could not fail, and a review round proved it (M11-R1).** It read `"ES B12345678"` — *with a space* — and the tier documents and enforces that there is no space between prefix and body, so that input was absent for a reason having nothing to do with ES not shipping. A live, checksum-less ES recognizer — exactly the half-shipped outcome this guard forbids — left it green, and adding the one control needed to satisfy `VAT-16` left the **whole suite at 239/239** with an unmeasured Spanish scheme shipping. FR and LV were fine (stubs for both turned it red). The negatives are now the canonical forms `ESB12345678` / `ES12345678Z` / `ESX1234567L`, and — the durable half — each input is first checked for **reachability** against the tier's grammar: two uppercase ASCII letters then an unbroken alphanumeric run. A negative the grammar could never match is now red on the spot rather than quietly true, which is the same "prove the corpus can express the thing" move PROP-04 made for M4-R17.
  > **Decided limit, measured in review round 2 and accepted rather than chased.** The reachability
  > helper's length floor is **9**, but the shortest token any shipped pattern can match is **11**
  > (`DE` + 9 digits). A negative written 9 or 10 characters long would therefore be judged *reachable*
  > and its absence believed — M11-R1's failure mode again, two characters out of reach. It is latent:
  > every VAT-04 negative today is at least 11 characters. Same family as M11-R1, so it is recorded
  > here rather than filed — a guard on a guard is worth one level, not four. If the floor is ever
  > touched, 11 is the number, and the doc comment's stated reason (*"the shortest shipped form is
  > `DE` + 9 digits"*) is what makes 9 look right.
- **VAT-05 — `lowercase_country_prefix_is_not_a_vat_number`: the country prefixes are
  uppercase-only.** The recognizers match `IT`/`DE`/`GB`/`PT`/`NL` in uppercase only, so a
  lower- or mixed-case rendering matches **nothing at all** — the letters are ASCII word
  characters, so there is no `(?-u:\b)` between them and the digits for the bare recognizer to
  use either.
  > **The stated reason for that does not survive the tier's own grammar, and the measured cost
  > of the rule is not what the reason claims (M11-R10, open).** The rationale in
  > `recognizers.rs` and in this entry's first version was *"lowercased, `call it <11 digits>`
  > would produce a 14-character span swallowing an ordinary word"* — but the tier also forbids
  > **any space between prefix and digits**, so `it 00905811006` cannot match whatever the case
  > rule says. Mutation: making the `IT` pattern case-insensitive turned exactly **one**
  > assertion red in 149 lib tests — `kinds("it00905811006").is_empty()`, the one that pins the
  > miss itself. The `"call it …"` assertion, which carries the rationale, stayed **green under
  > the very change it is quoted as forbidding**. Measured cost of folding case on all five
  > prefixed schemes, over 341.1 MB / 16 380 files of uncurated third-party source: **0
  > additional matches** (0 uppercase, 0 case-insensitive, for every scheme). Measured cost of
  > *keeping* it, driven through the real binary: **7 of 13 renderings of real published VAT
  > numbers reach the upstream in clear**, including `NL111222333b01` — an uppercase prefix with
  > a lowercase internal `B` separator, which no stated decision covers at all, and whose case
  > **no test pins in either direction** (changing `B` to `[Bb]` leaves the suite at 246/0/5).
  > The rule may well be right; it has never been *decided*. Tracked as
  > [M11-R10](reviews/M11.md#m11-r10) — a product-visible labelling/coverage change, so the
  > maintainer's call.
- **CASE-01 (M11-R10, rebuilt for M11-R11) — `every_letter_bearing_recognizer_answers_the_case_axis`: the set comes from the registry, not from a list.** M11-R10 was not one bug — **seven of thirteen renderings of values already in this repo's own corpus went upstream in clear**, because the VAT prefixes and the IBAN pattern spelled their letters `[A-Z]` while the Codice Fiscale, the ES DNI/NIE and the CN resident id folded case. Nobody had ever decided the axis: `iban_mod97`'s doc comment has promised to fold letters since **M1**, for input the regex could not deliver — the validator was written for a case the pattern made impossible, which is how it survived ten milestones.
  **Its first form was the instance-shaped fix wearing the chokepoint's words, and M11-R11 is that being found.** A nine-row hand-written `const` promised that "a new letter-bearing recognizer cannot ship without an entry here" while deriving nothing from the recognizer table, so **four** letter-bearing recognizers were outside it — `Secret`, `Email`, the GB NINO and the CN resident id. Measured: narrowing NINO to uppercase-only left the whole library suite green at **154 / 1**, the single red being a temporary probe, while `ab123456c` went from masked to forwarded in clear; the same held for the CN id and for `Secret`. Adding four rows would have closed four instances and left the class open.
  So the **set** now comes from `StructuredRecognizers::shipped_patterns()` — every recognizer the scan is actually built from — filtered by `pattern_can_match_a_letter`, which parses the pattern to the same HIR `regex` compiles and asks each literal and class whether it covers `A-Za-z`. A textual scan cannot answer this: a word boundary and a digit class are both spelled with letters that match none. `CASE_ANSWERS` then says only *what* the answer is, and it can say **`Fixed`** — deliberately does not fold — which is what makes M11-R10's decision 3 (`Secret`'s `sk-`/`AKIA` are formats, not conventions) expressible at all. For a `Folds` answer a known positive is checked uppercase, lowercase **and with exactly one letter flipped**, the last being the sharpest case and the one a corpus of lowercase strings would miss (`IT60x0542811101000000123456` was forwarded in clear).
  **Two decided limits, both measured rather than assumed.** (1) The axis is **ASCII** case, so a recognizer whose letters were non-ASCII would not be asked — the guard asserts every shipped pattern is itself ASCII, which makes that residue **0 patterns** today and turns the day it is not into a red test rather than a silent skip. (2) An empty *unanswered* list is also what a derivation that stopped seeing letters would produce, so every recorded answer must itself be about a pattern the derivation calls letter-bearing; the two lists are therefore an equality — the letter-bearing recognizers this build ships are precisely the ones the answers name — with **no count for anyone to keep current**.
- **CASE-02 (M11-R10) — `a_lowercase_iban_is_masked_only_when_it_verifies`: folding case on IBAN without opening the door it was keeping shut.** The IBAN half could not simply be folded. An IBAN has **no hard checksum gate** — M4 decided that a structurally valid one is masked even when mod-97 fails — so widening `[A-Z]` to `[A-Za-z]` sweeps in hex digests and base64 blobs. **Measured over 341.1 MB / 16 380 files of third-party source: 1 match uppercase, 150 case-folded**, and masking a hex digest inside a `tool_use.input` is the functional harm M10 spent nine rounds bounding. So `iban_case_gate` splits the rule by rendering: canonical uppercase keeps M4's behaviour untouched, while a rendering carrying **any** lowercase letter must be fully verifiable — mod-97 *and* the ISO 13616 length. **Measured residue: 0 of the 149 added.** Note `iban_length_ok` returns `true` for a country code it does not know, so for 145 of those the gate is mod-97 alone — the zero survives that weaker reading, which is the one the code implements. Both halves of the split are pinned here, because getting either wrong is silent.
- **CASE-03 (M11-R11) — `the_case_axis_audit_notices_a_recognizer_with_no_answer`: the half that gets skipped.** `CASE-01` proves the recorded answers are **right**; this proves the question is **reached**, and it exists because skipping this half is exactly how a nine-row `const` came to be documented as a chokepoint and survived a full review round. Both pure decisions are driven directly. The derivation gets a matrix of eleven patterns including the readings a substring scan gets wrong — a word boundary and `\d` (no), letters in a *literal* rather than a class (yes), one letter at the end of a digit run (yes), a byte class (yes, the walker arm no shipped pattern reaches), and `\p{Greek}` (no — the measured ASCII residue above). The audit gets the **real shipped registry plus a stand-in recognizer**, not a synthetic list, because a synthetic registry would only prove the function works on invented input; a digits-only stand-in must *not* be asked, and an answer naming no shipped recognizer must be reported **stale**, so a deleted recognizer cannot leave its answer behind asserting a closed question.
- **VAT-06 — `vat_is_always_on_regardless_of_locales`: the posture, and the guard against a config
  variable appearing.** The national-ID posture (M4-R1), not the FP-prone phone tier's. **There is
  no `PII_LOCALES`-style gate for this tier and there is deliberately not going to be one**
  (ROADMAP M11 Track A, decision 2 — gating it would also have inherited `PII_LOCALES`'s
  *narrowing* semantics, the subtlety M10 had to set in bold in the changelog). This fails if one
  is ever introduced by accident.
- **VAT-07 — `vat_is_not_inert_in_cjk_prose` (M4-R13).** A VAT number glued to a Han character is
  the natural rendering, not an evasion. Under Rust `regex`'s default Unicode `\b` a Han character
  is a word character, so there is no boundary before the `I` and the entire tier would be
  **silently inert** in CJK text — which is precisely how this repo once shipped inert card and ID
  recognizers. `(?-u:\b)` is what keeps it alive, and this is what would notice if it were lost.
- **VAT-08 — `vat_does_not_swallow_or_get_swallowed_by_an_adjacent_token`.** Two VAT numbers one
  space apart stay two spans; a VAT number inside a longer ASCII run is not a match at all, so a
  hash, a UUID or a base64 blob cannot contain one.
- **VAT-09 (measured) — `vat_over_mask_rate_on_arbitrary_eleven_digit_numbers`: the published FP
  cost.** A mod-10 check accepts about one arbitrary 11-digit number in ten, so the bare Italian
  form masks a share of ordinary 11-digit tokens. **Measured: 10 000 / 100 000 = 0.100.** The cost
  is accepted on purpose — over-mask, never leak, and the vault restores the value byte-identically
  on the response path — the same trade M4-R6 took for the 9- and 11-digit national IDs. What is
  *not* acceptable is quoting a rate nobody measured, so the number is produced by a deterministic
  sweep and pinned to a band: moving the shipped FP cost should have to edit this test to land.
  **What the 0.100 does and does not pin (M11-R5).** The sweep is over 100 000 *consecutive* 11-digit numbers, and for **any** scheme whose eleventh digit is a function of the first ten, exactly one value per block of ten passes — so `0.100` is structural, not a measurement of the arithmetic. Replacing `it_piva_valid`'s final comparison with `d[10] == 7` still printed `0.100` and still **passed**. What sweep 1 really pins is the *shape*: eleven digits, the last a check digit, no context required. So a **second sweep** now holds the check digit constant while the first ten digits vary — there a correct mod-10 still accepts ~1 in 10, while a comparison against a fixed digit accepts **all or nothing** (0.0 or 1.0). Two sweeps agreeing is evidence about the checksum; one was evidence about the format. (Seven other VAT guards did go red on that mutation, so this was a claim-accuracy defect rather than a coverage hole — the algorithm is pinned by VAT-01/02/03.) **And the rate survives contact with real text:** measured over 104 uncurated `\d{11}` tokens found in ~304 MB of third-party source, the shipped recognizer accepts **9 — 0.0865**, about **one over-mask per 34 MB** of code-shaped traffic, and the four values it masks are memory and byte-size constants: exactly the "database id or order number inside `tool_use.input`" harm the ROADMAP names.
  > **Decided limit, measured in review round 2: two sweeps agreeing is evidence that the check
  > digit *depends on the body*, not that the arithmetic is right.** Sweep 2's rate is **also** 0.100
  > by construction — over a contiguous run of the first ten digits the last of them contributes a
  > bijection mod 10, so exactly one value in ten yields any given check digit. Measured: swapping
  > `it_piva_valid`'s doubling **parity** — wrong arithmetic, still body-dependent — turns **9 tests red**
  > and leaves VAT-09 **green at 0.100 on both sweeps**. What sweep 2 does close is the M11-R5
  > stub (`d[10] == <const>` gives 0.0 or 1.0), and that is worth having. **The arithmetic itself is
  > pinned by the eight real-published-number guards that went red** — VAT-01, VAT-03, VAT-05,
  > VAT-06, VAT-07, VAT-08, VAT-13 and VAT-14 — so this is claim reach, not a coverage hole. Same
  > family as M11-R5; recorded, not chased.
- **VAT-10 (measured) — `vat_and_natid_collision_rate`: is the naming rule load-bearing or a
  curiosity?** **Measured: 1 997 / 20 000 = 0.0998** of valid P.IVAs also satisfy a DE Steuer-ID or
  LV personal-code check, so ~10% are named `[NATID_n]` and ~90% `[TAXID_n]`. Both are masked
  either way — this is a *labelling* statistic, not a coverage one — but it is the number that says
  the other 90% is real coverage this recognizer adds, not a relabelling of what the national-ID
  tier already caught.
  **It measures the smaller of two collisions and was quoted as though it were the only one (M11-R2).** Its sweep runs `10_000_000_000 + i`, so every value starts with `1` — while the only phone family that can claim a bare digit run is the `Trunk` arm `\b0\d{6,11}\b`, which needs a leading `0`. The phone collision is outside this guard's range *by construction*. See VAT-17.
- **VAT-17 (measured, M11-R2) — `bare_piva_phone_collision_rate_under_the_shipped_default`: the collision the milestone fixed the direction of without measuring the size of.** VAT-14 established that `TaxId` ranks below `Phone`, and that ordering stands — a numbering-plan lookup confirming an *assigned* number is better evidence than a mod-10 check. What nobody had was the magnitude, so nobody chose it. **Measured against `StructuredRecognizers::new()`, the configuration that ships: 1163/1500 = 0.775** of issuable bare P.IVAs are named `[PHONE_n]`, not `[TAXID_n]`. (An independent probe in review round 1, on a different sample, got 0.732 — same conclusion, different draw.) **The split by leading pair is the whole explanation:** `00xx` **5/152 = 0.033**, `0[1-9]xx` **1158/1348 = 0.859**. A leading `00` reads to libphonenumber as the international access code and is rejected — and all five real published P.IVAs this repo's corpus is built on are `00…`-leading, so VAT-01, VAT-03, VAT-05, VAT-06, VAT-14 and VAT-16's controls all sit **inside the immune sub-shape**. *A corpus has a shape, and that shape is a blind spot* (M4's lesson 2), landing again in the milestone that quotes it. Both sub-rates are asserted, not just the total, because the `00…` immunity is what every other VAT guard silently depends on. Like VAT-10 this is a **labelling** statistic — every byte is masked under either name — but what it costs is decision 1's purpose: telling a consumer *business* rather than *person*. **The maintainer settled it on 2026-09-03 (decision 4, ROADMAP → M11 Track A): the order stands**, with the ~77% cost accepted explicitly. The obvious refinement — yield the separator-free arm to `TaxId` — was rejected because `VAT-14`'s own two strings (`02079460958`, `03012345678`) *are* separator-free, so it undoes that guard rather than narrowing it. What this guard now does is hold the accepted number still: a change to the trade has to edit it to land.
- **VAT-18 (M11 Track A) — `vat_numbers_round_trip_over_http_in_every_rendering`
  (`tests/proxy_e2e.rs`): the new kind over real HTTP.** M11's headline feature shipped with
  **no end-to-end coverage at all** — every VAT guard tested the detector, and review round 1
  confirmed the HTTP round trip *by hand*. A capability verified by hand is one nothing will
  notice losing, and `[TAXID_n]` is new vocabulary on the wire: exactly what a serde or
  restore-path change breaks silently. This pins for `TaxId` what `E2E-01` pins for
  email/phone/IBAN. One value per rendering family, because the resolver and the vault see them
  differently — the bare Italian form, the VIES prefixed form, a second country's prefixed form,
  and the format-only NL form whose `Confidence` is `Structural`. `00…`-leading P.IVAs on
  purpose: that is the sub-shape the phone tier cannot claim (`VAT-17`), so the guard measures
  the VAT path rather than the collision.
  > **The vacuity trap it is built around, with the number.** The injected instruction
  > *exemplifies* `[TAXID_1]` — that is `AUG-01`'s subject — so it contributes **one**
  > `[TAXID_n]` token to the upstream body all by itself. A guard asserting the placeholder
  > appears *somewhere* in that body would be satisfied by boilerplate while the user's VAT
  > number went upstream in clear. Measured: four renderings give **5** tokens body-wide and
  > **4** in the user field, so with one value leaking a body-wide count of 4 would still have
  > passed. Every claim here is therefore made about the **field the PII was in**
  > (`upstream_last_user`), never about the whole body.
- **VAT-19 (M11 Track A) — `a_split_taxid_placeholder_is_restored_in_a_stream`.** The streaming
  de-masker buffers across SSE events because a placeholder can straddle them, and `E2E`'s
  split-placeholder guard proves that for `[EMAIL_1]`. `[TAXID_1]` is a **different length**, and
  the buffering logic is length-sensitive by nature — it has to decide how much to hold back — so
  "email works" does not imply "taxid works". The mock fragments the reply into 4-character
  pieces, splitting a 9-character token three ways.
- **VAT-11 — `nl_vat_confidence_splits_verified_from_format_only`: the one scheme with nothing to
  check.** The 2020 Dutch sole-trader `btw-id` is randomized by design; a legal entity's 9-digit
  body is an RSIN that satisfies the 11-proef. So NL is accepted on **format** (14 chars, mandatory
  `NL`, a literal `B` pinned at position 11) and `Confidence` tells the truth about which one it
  got — `Verified` on a mod-11 pass, `Structural` otherwise, exactly as an IBAN whose mod-97 fails
  is masked and flagged (M4). Both are masked; only the claim differs.
- **VAT-12 — `a_vat_inside_an_email_keeps_the_email_label`.** The resolver's naming rule
  (M4-R10/R11): a union that is *exactly* an `Email` span is named by the email even though `TaxId`
  outranks it. Every byte of both spans is masked regardless — priority names a union, it never
  drops one.
- **VAT-13 — `a_masked_vat_number_is_inert_and_restores_exactly`.** `[TAXID_1]` must not look like
  PII to the next pass or `mask_all` would not converge (ARCHITECTURE → *mask to a fixpoint*), and
  the round trip must be byte-exact — which is what makes an over-mask harmless downstream.
- **VAT-14 — `a_bare_piva_never_outranks_a_phone_or_a_national_id`: the defect this track nearly
  shipped, pinned from the side the change would come from.** A bare P.IVA is `\d{11}`, and so are
  the **compact domestic phone** shapes M10 measured: `02079460958` is a real London number,
  `03012345678` a real Berlin one, and *both satisfy the P.IVA mod-10*. With `TaxId` ranked above
  `Phone` — the first thing that looks right — every compact GB and DE number M10 measured silently
  became `[TAXID_n]`. No leak: the bytes are masked either way. But a **fidelity regression on a
  shipped, measured capability**, telling the model a phone number is a tax identifier. PHONE-NAT-01
  caught it; this pins it from the VAT side, where the change that would reintroduce it lives.
  It also pins the national-ID half (Enel's real P.IVA is a valid Latvian personal code, and keeps
  `[NATID_n]`). The ordering and its two justifications live on `PiiKind::priority`.
- **VAT-OM (M11 Track A) — `tests/vat_overmask.rs`, the over-mask family**, the tax tier's counterpart to `PHONE-OM` and named the same way: the two guards below run the shipped default over text nobody curated for them, because a negative corpus contains only the false positives somebody imagined. It was declared in that file's `//!` module doc and catalogued **nowhere** until M11-R4, which is the live instance that finding is about — `CAT-01` could not see a `//!` declaration, and `VAT-OM` was in no prefix list either.
- **VAT-15 (measured) — `the_realistic_turn_yields_exactly_the_expected_taxid_spans`
  (`tests/vat_overmask.rs`): the real-traffic half of VAT-09.** VAT-09 measures what the bare
  P.IVA does to a *uniform stream* of 11-digit numbers; it says nothing about how many 11-digit
  numbers real agent traffic contains. M10 shipped the domestic phone tier with **both** halves —
  a synthetic rate and a proof of zero hits over a real turn (PHONE-OM) — and the maintainer's
  decision of 2026-09-03 (ROADMAP → M11 Track A) held the VAT tier to the same bar, declared
  before the number existed: **zero** bare-form `TaxId` spans over the same ~22 KiB Claude Code
  turn in `tests/common/m7_turn.rs`. **Measured: 0 `TaxId` spans in 22 823 bytes** — the bar is
  clear and `0.100` stands as a published *synthetic worst case*.
  Two assertions, because the first can be satisfied for the wrong reason: no `TaxId` span (the
  bar verbatim), **and** no masked span of any kind that is exactly 11 ASCII digits. `TaxId` ranks
  *below* `NationalId` and `Phone` (VAT-14), so an 11-digit run the bare recognizer claims can be
  **named** by another tier and vanish from the first assertion while still being masked;
  filtering on the label would measure the naming rule, filtering on the shape cannot be fooled.
  > **The residue, and it is the honest limit of this guard.** The denominator is **zero**: the
  > turn contains **no 11-digit token at all** — 11 all-digit tokens, longest run **5 digits** —
  > so mod-10 was never actually invoked on this traffic. The zero therefore says *this traffic
  > offers the bare P.IVA nothing to bite*, which is a real and reassuring fact about agent
  > turns, but it is **not** a precision result: it is weaker than PHONE-OM's zero, whose
  > candidate set is every plausible digit group. What stays uncovered is traffic that *does*
  > carry 11-digit database ids or order numbers, where VAT-09's 0.100 is the number that
  > applies. The guard prints the denominator on every run so this cannot quietly turn into a
  > precision claim.
  >
  > **That residue now has a number, measured off-repo (M11 Round 1).** No fixture here can
  > supply one, so it was taken over text nobody in this project wrote: **303 772 514 bytes
  > across 12 859 `.rs`/`.md`/`.toml`/`.json`/`.txt` files** of the crates.io registry source
  > cache — code-shaped text, the closest available stand-in for a tool result. It holds **104**
  > eleven-digit tokens (**0.34 per MB**), of which **9 satisfy the mod-10 → 0.0865** — the
  > synthetic 0.100 survives contact with uncurated text. The four distinct values are
  > `00024414062`, `17179873280`, `34366720000`, `41789270000`: memory and byte-size constants,
  > i.e. exactly the harm named above. **Expect roughly one bare-P.IVA over-mask per 34 MB of
  > code-shaped traffic.** The measurement is not a test — it needs a corpus this repo cannot
  > vendor — so it is recorded here rather than guarded; re-derive it before quoting a different
  > number.
  >
  > **The prefixed schemes measured zero on the same corpus, and that is the answer to
  > “why does format-only NL belong in a checksum tier?”** Against a denominator of
  > **1 075 001** uppercase-alnum tokens of length 6–16: `NL\d{9}B\d{2}` → **0**, the
  > relaxed `NL\d{9}[A-Z]\d{2}` (dropping the literal `B` anchor) → **0**, and
  > `[A-Z]{2}\d{9}` — the whole prefixed family's shape, country code ignored — → **0**.
  > The plate families M11 **refused** for having no verifier fire **204** times in that same
  > corpus (`[A-Z]{2}\d{2,4}[A-Z]{1,2}` → 24, `[A-Z]{1,3}-[A-Z]{1,2}\d{1,4}` → 180). So the
  > tier did not accept what it had just turned away: NL pins 14 positions with three literals,
  > and unlike a plate its body *is* checked — the 11-proef result is reported as
  > `Verified` vs `Structural` (VAT-11) rather than claimed.
- **VAT-16 — `the_guard_would_notice_a_vat_number_that_really_is_there`: the positive control,
  one per shipped scheme.** "No VAT spans" and "no detection at all" produce the same empty
  vector (M9-R28, BENCH-01), so VAT-15 needs a control — and M10-R7 is why it is one control **per
  scheme**: PHONE-OM's first version covered one shape family of three, and deleting the other two
  left every assertion green while the guard's whole subject was switched off. The count is read
  from `shipped_tax_recognizer_count()` rather than kept as a literal here, so a seventh
  recognizer landing without a seventh control is **red** instead of a silent narrowing. Each
  control is a real published number spliced into the real turn, so it also proves the scheme is
  reachable in context and not only in a two-word test string.

### The `PiiKind` enum itself (`src/pii/mod.rs`)
- **KIND-01 (M11-R3, rebuilt for M11-R6) — `all_is_the_list_every_other_guard_is_driven_from`:
  one kind, one line, four things generated.** M11 added an eleventh kind and found a *twelfth*
  could be added with the whole suite green, because every per-variant guard watched a hand-typed
  list. The first fix made `PiiKind::ALL` the single list and proved it complete with a **successor
  chain** whose `match` the compiler checks. **Review round 2 broke that in one move:** the compiler
  demands an *arm*, not a place in the walk, so `Vin => return None` compiles, the walk never
  reaches `Vin`, `ALL` and the walk still agree, and the guard passes with a kind unlisted. Rust
  cannot enumerate an enum's variants, so **no test could have closed it** — a guard was the wrong
  shape of answer.
  > **So the chain is gone and the question is removed instead of watched.** The `pii_kinds!` macro
  > generates the enum, `ALL`, `label` and `from_label` from **one list**: adding a kind is one line
  > and all four move together, and declaring a variant outside that list is a *parse error*.
  > Deliberately **not** generated are `priority` and `is_structured` — those are judgements about a
  > new kind rather than restatements of it, and the compiler already stops you at their exhaustive
  > `match`, which is the right place to be stopped. Four mutations, all red: a kind added to the
  > list (compile error at `priority`), a variant declared without a label (macro parse error —
  > round 2's exact route), two kinds sharing a label, and a lowercase label. What KIND-01 still
  > asserts is the part a generator cannot: that `ALL` is the eleven-entry list every other guard is
  > driven from, and that the priority judgement was actually made rather than defaulted.
- **KIND-02 (M11-R3) — `every_kind_round_trips_through_its_label`.** Both halves are now generated
  from the same macro literals (M11-R6), so a kind cannot be in one and missing from the other.
  What a generator still **cannot** check is that each literal is **uppercase** — a lowercase one is
  unreachable after `to_ascii_uppercase` — and that the literals are **distinct**. This checks both,
  and they are the two mutations it goes red on. That is not cosmetic: `from_label` gates
  `anonymizer::is_placeholder_token`, which is what makes a placeholder **inert** to the next
  detection pass — the mechanism `Vault::mask_all`'s fixpoint rests on (M5-R4) — and it also gates
  the M1.5 warning that an unresolved known-kind placeholder is logged rather than silently
  shipped. So a kind in `label` but not `from_label` degrades convergence *and* observability with
  no failing test. Asserts the round trip for every variant in `PiiKind::ALL`, case-insensitively
  (the model may echo a token back in any case), and that no two kinds share a label — which would
  make the inverse ambiguous and collapse them in the vault.

### M11 Track A — the augmentation prompt and the detection cache (`src/pipeline/privacy.rs`)
- **AUG-01 — `the_augmentation_prompt_names_every_placeholder_kind_it_should`.** Adding a `PiiKind`
  without teaching the model to read its token is a **silent** degradation: the mask works, the
  round trip works, and the model simply handles `[TAXID_1]` worse than the four kinds the prompt
  names. Nothing fails, so nothing tells you. Also asserts every exemplified label is a real
  `PiiKind` label, so the prompt cannot teach a vocabulary the de-masker does not speak.
  **The first version of this guard had the exact defect it describes (M11-R3):** it iterated five
  string literals, so it watched the five kinds somebody typed rather than the enum — a twelfth
  `PiiKind` could be added and the whole suite stayed green. It is now driven from `PiiKind::ALL`
  through an **exhaustive `match`**, so a new variant is a *compile error* here and its author has
  to answer whether the prompt exemplifies it. Either answer is fine; not being asked is not.
  The count of worked examples (5) is asserted too, because each one is paid for on every request.
- **AUG-02 — `the_cache_cannot_serve_an_instruction_because_it_never_sees_one`.** The worry the
  M7.1 content-keyed cache invites whenever the instruction changes: turn 1 caches something derived
  from the old text, turn 2 is served it, and a fresh binary keeps emitting an instruction that
  never mentions the new placeholder — a failure with no symptom. It cannot happen, and this pins
  **why** rather than asserting the conclusion: a recording detector captures every text the stage
  submits, and none contains the prompt, because the instruction is appended *after* masking has
  already run. The cache keys on exactly those texts, so a text it never sees is a text it cannot
  serve. **If the injection ever moved before masking this fails** — and that reordering is the only
  way the stale-instruction bug could become real.

### M9 — execution providers & the provider benchmark (feature `onnx`, no model needed)
`src/pii/onnx.rs::ep_tests` — unit, runs in plain `cargo test-onnx`. The runtime knob is parsed here, and the two *failure modes must stay distinct*: a **typo** fails startup, a real-but-absent accelerator **falls back to CPU**. Conflating them would silently run CPU while the operator believed a GPU was engaged.
- EP-01 — `parses_known_providers_case_insensitively_and_trims`: every provider name plus the aliases (`dml`, `trt`, `vino`) and `""` → `Cpu`, case- and whitespace-insensitive.
- EP-02 — `an_unknown_provider_is_an_error_not_a_silent_cpu`: a typo (`directl`, `gpu`) is an `Err` naming the bad value — never a silent CPU run. Pins that **`vulkan` is rejected**: it is *not* an ONNX Runtime backend (WebGPU is the nearest cross-vendor EP), so it must not look valid to an operator who assumes it is.
- EP-03 — `as_str_round_trips_through_parse`: `parse(as_str(p)) == p` for every variant, so the value logged at startup is always one an operator can paste back into `NER_EXECUTION_PROVIDER`.

- EP-04 — `requesting_an_unavailable_provider_yields_cpu_as_the_effective_one`: `Cpu` produces no dispatch (never an accelerated build), and every accelerator *does* produce one — availability is ORT's answer to give, not ours to pre-empt by skipping the attempt.

**CLI surface (`tests/binary_smoke.rs`, spawns the real `.exe`).** M9 gave the binary its first arguments, and unexpected input must refuse rather than serve.
- CLI-01 — `unknown_cli_argument_is_refused_and_never_binds` (M9-R4): `--bench-provider` (the singular typo) exits **non-zero** *and* the port **never becomes reachable**. Both halves are load-bearing — an exit-code-only assertion would pass for a binary that served for ten seconds first, which is exactly how the defect was found.
- CLI-02 — `help_prints_usage_and_exits_zero` (M9-R4): `--help` prints usage listing the flags and exits 0, so the natural way to ask "what does this take?" is not itself an error.
- CLI-04 — `version_prints_the_manifest_version_and_exits_zero` (M10): `--version` prints `CARGO_PKG_VERSION`, the target triple and whether the ML layer is compiled in, and exits 0. It runs with a deliberately useless `UPSTREAM_BASE_URL`, so a `--version` handled *after* `Config::from_env` would fail the test — asking what a binary is must not require a valid upstream. Before M10 there was no such flag *and* unknown arguments are refused (CLI-01), so `--version` did not merely print nothing: it **failed to start**.
- CLI-05 — `help_names_every_environment_variable_the_code_reads` (M10): extracts every literal `env::var` / `env_var_os` / `env_flag` / `env_or` key from `src/config.rs`, `src/server.rs` and `src/main.rs`, and asserts each is **named** in `--help`. Configuration here is entirely environment variables with no config file, so a help text that defers to `README.md` means you need the repo to run the program — and a hand-written one drifts. **Scope limit, stated so the guard doesn't read stronger than it is:** it proves each key is *named*, not that its description or default is accurate; that is review's job. It also asserts the extractor matched ≥20 keys, so a change in the source's shape fails loudly instead of passing vacuously (M4-R13's lesson).
- CLI-06 — `default_log_level_is_info_and_timestamps_carry_an_offset` (M10): spawns the binary with `RUST_LOG` **removed** (`env_remove`, never a mutation of the test process's own env) and asserts (a) it still logs `listening on` — the shipped default was ERROR-only, so a released proxy started **completely silent** and looked identical to a broken one, which breaks the one check this project asks operators to make — and (b) every such line's timestamp carries an **explicit** offset (`Z` or `[+-]HH:MM`). (b) is the regression that local timestamps invite: a bare `12:37:04` reads correctly on the author's box and is ambiguous everywhere else. It asserts the *offset*, not the wall-clock value, because the value depends on the machine.

<a id="algorithmic-complexity-guards"></a>
**Complexity guards — a third axis (`tests/complexity.rs`).** The file's model above has two dimensions (field *size*, entity *count*). M10 added three more, each found only after a guard had already reported green.

> **The axis a guard cannot see is the one *above* its own unit (M10-R28), and this is the third time
> that has been the finding.** Look at what each guard varies against what it holds fixed:
>
> | guard | varies | holds constant |
> |---|---|---|
> | DOS-01…03 | field size *n* | entity count, alphabet, periodicity, **field count** |
> | DOS-04 (M4-R24) | entity count *k* | alphabet, periodicity, **field count** |
> | DOS-05 (M10-R2) | alphabet | periodicity, **field count** |
> | DOS-06 (M10-R20) | periodicity | **field count** |
> | DOS-07 (M10-R28) | field count | **fixpoint pass** |
> | DOS-08 (M10-R35) | fixpoint pass | **detector composition** |
> | DOS-08 + DOS-09 (M10-R42) | detector composition | **the input's rendering** |
> | **DOS-BUD (M10-R54)** | **how the number is written** | *(a measurement, not a guard)* |
>
> **And DOS-07 inherited the next blind spot in turn, which is the pattern itself.** It holds the
> **fixpoint pass** constant: it refuses on pass 0 and never asks whether passes 1–4 are charged at
> all. They were not — `StructuredRecognizers` overrode one budget-taking method and not the other,
> so every later pass started from a full allowance and a legal 15.63 MiB body answered `200` in
> 17.2 s (M10-R35). **DOS-08** varies that axis. Six findings, six guards, and each one written *by*
> the blind spot of the last.
>
> **Seven rows now, and the seventh was found the same way as the sixth (M10-R53/R54).** DOS-08 and
> DOS-09 vary the detector composition and hold **the input's rendering** constant, which is what let
> two consecutive rounds publish an availability claim measured on `347 1234567` — the one rendering
> that is the global minimum. DOS-BUD varies that axis now, and its own row carries what it holds
> constant rather than a dash. *The table is not a record of what has been tested; it is the list of
> quantities somebody has thought to name, and the next finding is always in the column on the right.*
>
> **DOS-08's first version held the seventh quantity constant, and its row said `—` (M10-R42).** It
> constructed a bare `StructuredRecognizers`, so the identical defect in a shipped **wrapper** —
> `CachingDetector::redetect` — left all 218 tests green while the request under-charged 21×. *A guard
> written to prove that a defect is now unrepresentable must quantify over the shape the defect can
> take, not over the instance that had it.* So DOS-08 loops over every chain `AppState::new` can build
> and DOS-09 drives the cached one end to end. **The dash was the finding**, which is the discipline
> to carry forward: a guard's row must name what it holds constant, and `—` is never the honest
> answer.
>
> Every guard before DOS-07 measured **one string**, because `try_detect(&str)` is the shape each was
> handed. The masking path takes a **body**. Each of these guards was written *by* the finding the
> previous blind spot produced, and each inherited the next blind spot from the shape of the thing it
> was given to test — so *ask what a guard holds constant, not only what it varies* (M4's
> retrospective lesson 6) has a sharper companion: **when the guard's unit is smaller than the
> attack's unit, no number of axes inside its own unit will reach the attack.** Its counterpart on
> the design side is in [ARCHITECTURE](ARCHITECTURE.md): *a budget scoped to a unit the client can
> multiply is a rate, not a bound.*
- **DOS-05 (M10-R2) — *what the bytes are*.** DOS-01…04 hold the **character class** constant (`"a"*1M`, `"sk-"*350k`, `"4111 1111…"`, `"a@b.co "*n`) and **not one produces a phone candidate** — the card row's groups are four digits, and the domestic-phone families need a 2–3-digit group at an ASCII word boundary. So M10's whole change was invisible to the DoS guard while a legal 12 MiB body cost 105 s. Two cases: a dense run of real numbers (validation *and* the splice) and a run of arbitrary groups (mostly rejections, the expensive verdict). Deliberately **not** asserted: that the arbitrary run masks nothing — repeating any pattern yields rotations of itself, and with nine plans some rotation is usually somebody's real number.
- **DOS-06 (M10-R20) — *how often the same bytes come round again*, and this one indicted every number the milestone had published.** M10 bounded the validation cost with a per-scan memo and measured the fix on `unit.repeat(n)` bodies — as did DOS-05, and as did every figure in the docs. A memo keyed on the matched bytes only helps candidates that **recur**, so all of it measured the input's periodicity rather than the code: same shape, same 4 MiB, varying only the distinct-candidate count moved the cost **207 ms → 17,049 ms**, and a legal 15 MiB body answered in **64.5 s** at the default configuration. The generator is an **odometer, not a modular hash** — the first draft used `(i * 7) % 9000`, looked distinct, silently repeated after its period, and reported "the budget was never reached" as if that were the product's doing. Asserts both halves: bounded work under the budget, and a body that exhausts it is **refused** (`try_detect` → `Err`) rather than forwarded with a partial scan. The refusal case is deliberately outside the wall-clock wrapper — sizing the product's bound to fit the test profile is backwards.
- **DOS-07 (M10-R28) — *how many fields the body has*, the first axis that is not a property of the text at all.** `a_body_that_splits_digit_dense_text_across_many_fields_is_refused_as_one_request` drives `PrivacyStage::on_request` rather than `try_detect`, with **20 fields that each pass on their own**, and asserts the *request* is refused. It opens by proving a single field really does pass — without that it could be green because one field already exceeds the allowance, which would say nothing about the request being the unit. Also asserts the refusal names *"per request"*: a message that blames one field sends an agent to shrink the wrong thing, which is what M10-R29 cost.
- **DOS-08 (M10-R35, widened by M10-R42) — *which fixpoint pass*, over every wrapper position the chain can take.** `every_fixpoint_pass_is_charged_to_the_request_budget` asserts (a) `redetect` — the fixpoint's passes 1..n — refuses on a **one-unit** allowance exactly as `try_detect` does, and (b) a whole `mask_all` costs strictly more than its own pass 0. This is the guard whose absence let R35 through: the budget was threaded via a `_within` seam whose default silently minted a fresh allowance, the leaf detector overrode one of the two methods, and **the entire suite stayed green on both sides of the one-line difference**. The seam is deleted now — both methods *take* a budget — so the defect is unrepresentable rather than untested; this says so out loud. It loops over `shipped_chains()` — bare, composed, cached, cached+composed, and the three `FailOpen` positions — because its first version constructed a bare `StructuredRecognizers` and the same defect in a shipped **wrapper** left all 218 tests green (M10-R42). **Its second half took two attempts, and the first one is worth recording:** the obvious two-pass input is M4-R17's `4111111111111111555 867 5309`, where masking the phone exposes a Luhn-valid card — but a *card* is validated by a checksum, which M10-R29 made deliberately free, so the total spend equalled pass 0's and the assertion failed on correct code. What is needed is masking that exposes a **phone**, and an ASCII word boundary provides it: `…com347 1234567` has no boundary between `m` and `3`, so `[EMAIL_1]347 1234567` creates the candidate. *A guard for "the later passes cost something" has to be built from work the budget actually charges.*
- **DOS-09 (M10-R42) — *identical* fields through the **cached** chain, the complement of DOS-07.** `identical_fields_through_the_cached_chain_are_charged_on_every_pass` drives `PrivacyStage` with 20 byte-identical 20 KB fields and the detection cache **on**. DOS-07 uses distinct fields and no cache — that is the M10-R28 attack; this is the M10-R35 one, and the two are complements rather than duplicates. Identical fields make pass 0 free from the cache (correctly: a hit did no work), so the entire cost lands on `redetect`, which is deliberately never cached because later passes run on per-request masked text. A `redetect` that drops the allowance is therefore **invisible to every guard without a cache in it** — which is exactly how the defect survived one level up. The body's lead `11` group is rejected by every enabled plan, so nothing is masked and every pass re-validates the whole field: 205,065 units charged against 9,765 with the defect present, a 21× differential that decides refuse-versus-forward at any allowance between the two. **Its first draft repeated one literal group**, so the per-scan memo collapsed the field to a single validated candidate and the case failed against correct code — the fifth appearance of this milestone's oldest trap. *Identical to each other, distinct within.*
- **DOS-BUD (M10-R30) — the measurement, `#[ignore]`d and run on `--release`.** `budget_refusal_line_and_cost` prints where the refusal line falls and what it costs: one field at increasing sizes, one request at increasing *field counts*, a realistic SQL tool result with one phone column, and the same body with the phone tier off. This exists because the published bound was quoted from a constant for three review rounds and was wrong in three ways at once — the wrong unit, a figure 2–4× optimistic, and a claim about "phone-shaped" text that bare 9-digit tokens also satisfied. The number in `MAX_PHONE_VALIDATIONS_PER_REQUEST` now comes from **here**, and its realistic-payload row is what set it: at 50,000 units an ordinary 367 KB database result was refused. Also the source of the ~3 µs per `parse()` figure. **Two rows exist because of what they cost to omit:** the SQL shape at `MAX_BODY_BYTES` in **two legal renderings** (M10-R53/R54 — `347 XXXXXXX` is masked at 16 MiB, the same numbers as `3XX XXX XXXX` are **refused**), and a per-rendering unit table showing the 1-to-29 spread. Twice this harness generalized an availability claim from a single point of its own grid; varying the rendering is what turns *"can real traffic reach the allowance?"* from a guess into a band. **Its SQL rows go through `Vault::mask_all`, not `try_detect`** — measuring one pass is how the published figure came to be per-pass while claiming to be per-field (M10-R30), and after M10-R35 the later passes are charged, which roughly doubles a masking body's spend. A final row measures what the budget does **not** bound: 16 MiB with the phone tier off, the unbudgeted linear floor, so the per-request ceiling can be published as *validation + linear work* instead of the validation term alone wearing the word "ceiling".
- **The R29 sibling of CFG-01 — `a_digit_dense_field_is_masked_not_refused_when_no_phone_region_is_enabled`.** CFG-01 pins that `PII_LOCALES=` means *no region*; this pins what that must **cost**: nothing. The budget was decremented by every recognizer with a validator, including the nine always-on national-ID checksums (arithmetic over ≤ 18 bytes), so 800 KB of bare 9-digit tokens was a `400` in 45 ms **with the phone tier not loaded at all** — where the previous release masked and forwarded it in 150 ms. It also asserts the vault is non-empty, because an unrefused but unscanned field would be the same leak wearing a `200`.

> **All three refusal guards cross an explicit *test* allowance, not the shipped one, and that is a deliberate trade (M10).** 500,000 units is ~1.5 s in `--release` and ~25 s unoptimized; three `cargo test` cases each exhausting it would add over a minute to every run, and a slow guard is a guard somebody eventually marks `#[ignore]` — which in this milestone alone has hidden three findings. What these guards assert is the **unit** of the allowance and the **policy** on exhausting it (refuse, don't truncate); neither claim is about the size of the number. The number is pinned separately by DOS-BUD on the shipped constant. The seam is `PrivacyStage::with_validation_budget` / `Config::pii_max_phone_validations`, which is **not** an environment variable on purpose: a fail-closed CPU bound an operator can raise is not a bound.

> **The wall-clock budget is 30 s, not 10 s, and the reason is contention (M10).** Measured serially in debug: splice 0.32 s · email 0.82 s · dense-real 0.89 s · secret 1.16 s · arbitrary-groups 1.41 s · distinct-groups 1.78 s · **cards 2.25–3.34 s**. At 10 s the slowest had a 3× margin, and `cargo test --features onnx` runs every test binary at once — the ONNX cases saturate the cores and a 1.9 s case was observed crossing 10 s. **A guard that reds falsely on a busy machine teaches its reader to re-run rather than to look**, which is worse than no guard. 30 s keeps the separation it exists for: the linear side needs a 9× slowdown to reach it, the quadratic side is already past it *idle* (52 s splice, 151 s for DOS-01's 200 KB field). The ceiling is set by the fastest quadratic case on record — this is not "big enough to never fail".

**Dependency footprint (`tests/dependency_footprint.rs`).**
- DEP-01 — `default_build_excludes_the_onnx_and_hf_stack` (M2.5-R1): the ONNX/HF stack stays behind the `onnx` feature. A **regression pin on one known stack**, which is all it ever was.
- **DEP-02 (M10-R9) — `the_default_build_compiles_no_native_code`: the *property*.** DEP-01's six literal names were being cited as the native-dep-free *guarantee* the project states three times — but a denylist cannot observe a **new** native dependency, so "absent from the list" carries no information. DEP-02 asserts nothing reachable in the default build compiles or wraps C (`cc` / `cmake` / `bindgen`, or a `*-sys` crate) **beyond the TLS the host OS provides**. The edge filter `normal,build` is load-bearing: `-e build` alone descends only through build-dep edges and returns a one-line tree from a root with no build-deps, i.e. it would pass seeing nothing. Validated against `--features onnx`, where it correctly surfaces `ort-sys`, `aws-lc-sys`, `cc`, `cmake`.
  > **Both DEP guards query the release matrix, not the host — and that is why (2026-07-31).** They used to ask `cargo tree` about the machine running them, with `windows-sys` as the single allowance. On Windows: green. On CI's first Linux run: **red**, naming `openssl-sys` and `cc` — because `native-tls` is the *platform's* TLS (schannel / OpenSSL / Security.framework), so "the default build is native-dep-free" was a claim about one point of a grid. The fix is the M10 shape: iterate the five released targets, each with an explicit allowance and the reason beside it. Verified by mutation — deleting the Linux allowance reds the guard **on Windows**, naming the target, which is the property that was missing. `DEP_GUARD_HOST_ONLY=1` checks the host alone for offline work, and says so; CI never sets it.

**The catalogue's own guard (`tests/test_catalogue.rs`).**
- **CAT-01 (M10-R55 / M10-R59) — `every_declared_guard_id_appears_in_the_test_catalogue`.** Extracts every guard id declared in a `#[test]`'s own doc comment **or in a file's `//!` module doc**, and asserts each is **named in this file**. Deliberately the same shape as **CLI-05**: extract the claims from the *source*, assert each appears in the *document*. **This document is hand-maintained, and it drifted five times in M10** (R51 twice, R55 three times) — each time a guard was added, catalogued nowhere, and remembered only by the round that asked for it. M10-R55 prescribed this test; M10-R59 pointed out it had not been built, which was itself the tenth instance of that class. **M11-R4 then found the guard had its own defect, and it was the same one.** Declarations were recognised only if the family appeared in a hand-kept `ID_PREFIXES` array, so a *new* family was invisible: removing `"VAT"` and `"AUG"` and scrubbing all 24 of their ids from this file left it green, and the non-vacuity floor meant to prevent that was **20 against 54 declared ids**. There is no prefix list any more — an id is recognised by its **shape** (`CAT-02`), so a family is in scope the moment somebody writes one. **The non-vacuity floor then had to be rebuilt too (M11-R7), and the way it was wrong is the interesting part:** it read `>= 70` against a stated "measured 73" that was never a reading of this extractor at all — the extractor counted **(id, file) pairs**, not distinct ids, and the real figure was 90 at the commit that wrote 73. The consequence was exact: deleting the `//!` walk left **70** pairs against `>= 70`, so the mutation meant to prove that walk alive passed by one. A single total cannot notice one of two independent mechanisms dying while the other grows, so each walk now asserts **its own** liveness (`>= 4` module-doc declarations, `>= 60` test-doc ones) and the aggregate (`>= 90`) is left to catch the shape rule narrowing outright. Three assertions, three distinct mutations, three different lines.
  > **What the three floors are measured against, read from this extractor at this commit
  > (M11 review round 3) — and the reason a number in prose is the wrong place for it.** The set
  > gained an `Origin` component in the same fix, so `declared.len()` no longer counts (id, file)
  > pairs: it counts **(id, file, origin) triples**, and six pairs are declared in both a `//!`
  > and a `///` block of the same file. Live values: **module-doc 32 · test-doc 70 · total 102**
  > (distinct (id, file) pairs **96**, distinct ids **79**). So the aggregate floor's real slack
  > is 12, not the 6 its own comment implies — it notices *less* than the number beside it
  > claims. **Decided limit, not chased:** this is the third statement of the same shape inside
  > one milestone (M11-R4's "20 against 54", M11-R7's "70 against a measured 73", this one), and
  > the class is closed — the rule that comes out of it is *a guard's non-vacuity number belongs
  > in its **failure message**, computed, never in prose beside the literal*, because every prose
  > copy is a number that has to be re-read on a commit nobody remembers to re-read it on. The
  > per-mechanism assertions here already print theirs; the aggregate's does not. Adopting the shape rule brought **nine real families under the check that never were**: `CC`, `DBG`, `NER-EP`, `PERF`, `PHONE-COV`, `REG`, `THREAD` and two more, all catalogued here and none of them previously verified. **Scope limits:** it proves each declared id is *named*, not that its description is accurate and not that every test has an id — accuracy is review's job; "named" means named *anywhere*, so a cross-reference from a sibling entry satisfies it even when the id's own entry is gone (M11 round 0, mutation M7); and nothing here can catch an id deleted from *both* source and document in one change, which is a deliberate act rather than the drift this guard is for. *It found four on its first run* — `LOG-03` and `PROP-01a/b/c`, catalogued only inside the compressed forms `LOG-01/02/03` and `PROP-01` — and four more the day the shape rule landed: `VAT-OM`, `KIND-01`, `KIND-02` and `CAT-02` itself.
- **CAT-02 (M11-R4) — `the_guard_id_shape_rule_separates_ids_from_prose`.** Replacing a hand-kept list with a rule moves the risk into the rule: one that quietly narrowed would take CAT-01 down with it and leave it green, since the floor catches a collapse but not a narrowing. So the rule is pinned directly, by a matrix of the cases it must separate — 18 real ids accepted (`VAT-15`, `PERF-M7-03`, `DOS-BUD`, `FAILOPEN-BUD`, `PHONE-NAT-01`, `VAT-OM`, …) against **17** things that look like them and are not. The two exclusions that carry the precision are structural rather than enumerated: **a first segment shorter than two characters** rules out the NER's eight BIO tag names (`B-PER`, `I-ORG`, `B-DATE`, …) quoted in the eval tests' module docs, and **a last segment that is neither all digits nor all letters** rules out every review reference (`M4-R13`, `M10-R55`) — which matters because those appear in nearly every doc comment in this repo, and mistaking one for a guard id would demand a catalogue entry for every finding ever filed. Residue, measured and accepted: acronym-plus-number tokens such as `UTF-8` or `AGPL-3` would be accepted; none occurs in any scanned doc comment today, and if one ever does the symptom is a failure naming it.

**Fail-open boundary (`src/pii/composite.rs`, unit).**
- **FAILOPEN-BUD (M10-R41, added by M10-R47) — `fail_open_swallows_a_failed_detector_but_never_an_exhausted_allowance`.** `FailOpen` exists to swallow a **detector** failure; it must never swallow an exhausted **request allowance**, because the text was not fully examined and `Ok(vec![])` there forwards a partially scanned body with a clean bill of health. **That line had no witness for two rounds** — deleting it left the whole suite green while a body the request path must refuse was forwarded through `Caching(Composite([FailOpen(Structured)])))`, and M10-R41's own *"test that would have caught it"* had prescribed exactly this case. Asserts **both** directions, because either alone is satisfied by a wrapper that has stopped distinguishing them: swallow everything and (a) passes, propagate everything and (b) passes. It also asserts the budget was genuinely spent, so it cannot pass against a case that never reached a real refusal. *A guard for a distinction has to exercise both sides of it.*

**Configuration (`src/config.rs`, unit).**
- **CFG-01 (M10-R5) — `an_empty_pii_locales_is_off_not_everything`.** Three spellings, three behaviours: unset → all nine regions, `PII_LOCALES=` → **none**, an explicit list → that list. The middle one is why it exists — an empty value used to fall back to the default set, so the value ARCHITECTURE named as the response to a `phonenumber` advisory turned the tier fully **on**. Driven through `parse_header_list` / `default_locales`, never by mutating process env in a parallel run.

**Log subscriber (`src/logging.rs`, unit).** The two defaults CLI-06 pins end-to-end, tested at the seam so the failure is legible.
- LOG-01 · LOG-02 · LOG-03 — `an_unset_rust_log_defaults_to_info` · `a_blank_rust_log_is_treated_as_unset` · `a_set_rust_log_still_wins`: `log_filter` takes the `RUST_LOG` **value**, not the environment, so these run in a parallel test binary without a process-env data race. LOG-02 is the one worth keeping: an exported-but-blank `RUST_LOG` parses to *no directives*, which is ERROR-only — the silent binary through a second door.
- LOG-04 — `the_offset_lookup_is_stable`: the offset query is pure. It is also **all** a portable test can assert — under `cargo test` the process is already multi-threaded, so on Linux/macOS `time` legitimately refuses to answer and the honest result is `None`. Asserting a wall-clock offset here would assert the test runner's threading model.
- LOG-05 — `the_timestamp_always_carries_an_explicit_offset`: formats a **fixed** instant (formatting *now* would not be deterministic) and pins the exact rendering — `2026-07-29T12:37:04.844780+02:00` — so the format keeps every part the UTC default had (date, time, six sub-second digits) and only swaps `Z` for a numeric offset. Also drives the real timer to prove it is built from that description.
- BENCH-01 — `bench::report_tests::the_no_accelerator_guidance_never_names_a_cargo_feature` (M9-R20) + `the_header_reports_the_resolved_thread_shape` (M9-R3): unit tests over `format_report`. **These exist because CLI-03 alone was vacuous in the `onnx` build** — the spawned binary bails at model resolution before reaching the report, and on a box whose platform accelerator *is* present the `!measured_an_accelerator` branch never fires anyway. Driving `format_report` with a CPU-only result set is the only way to reach that branch deterministically on any machine. Note the assertion shape: it checks the **seven concrete feature names**, not the substring `ep-`, because the report legitimately says *"enabling several `ep-*` features does not combine them"* and that sentence must survive. It opens with a non-vacuity assertion that the branch was reached at all — the lesson from M4-R13/M4-R24, applied to a guard written *to fix* a coverage gap.
- BENCH-02 — `bench::report_tests::every_platforms_guidance_is_sound_not_only_this_ones` (M9-R25): the no-accelerator guidance is a five-way platform chain, and a test that just *calls* it sees only the arm compiled for the running machine — three of the five ship on platforms CI never runs. The pure `guidance_for(Platform)` is split out of the `cfg!` precisely so all five are reachable here; the test asserts each is non-empty, names no `ep-*` feature, and that the five messages are **distinct** (a collapsed chain would satisfy every other assertion while telling a Mac user about DirectML). This is M9-R16's shape — a message wrong for macOS/Linux surviving because nothing on Windows could observe it — closed structurally rather than by re-reading.
- BENCH-03 — `bench::report_tests::platform_classification_covers_every_target_and_orders_linux_correctly` (M9-R27): the platform **selection**, which M9-R25's refactor left on the untestable side — it moved the *messages* out of `cfg!` and stopped. `platform_from(os, arch)` takes `std::env::consts` instead (behaviour-identical, both fixed at compile time), so the mapping is a pure function: `windows`/`macos` on both arches, and — the one that bites — `linux`+`x86_64` matched **before** bare `linux`. Invert that order and every x86_64 Linux operator is told no accelerator is wired for their architecture, on a build that has CUDA.

> **A test that captures logs must prove the capture is live, and `tracing` capture is process-global (M9-R28).** `anonymizer::tests::capture_debug_logs` used a *scoped* (`with_default`, thread-local) subscriber, which looks race-free and is not: `tracing` caches per-callsite interest and derives it from a **process-global** max level, so a callsite evaluated while no debug subscriber existed stays disabled and the capturing thread gets a **completely empty** buffer — not a partial one. Measured: **168/1500 (11.2%)** failures concurrent, **0/1500** alone; a `Mutex` serializing the two callers did **not** help (7/30), because the interference comes from the other ~114 tests in the binary, not the sibling. The fix is a **process-global subscriber installed once** at DEBUG with **per-thread routing** — every callsite stays enabled for the whole binary, and each thread reads only its own buffer (0/30, then 0/10 full-suite runs).
>
> The dangerous half was not the flake. **`a_clean_convergence_stays_silent` asserts only an *absence*, and its correct outcome is also a near-empty buffer** — so it could never distinguish "correctly silent" from "capture dead", and it reported `ok` in a run where its sibling proved the capture had failed. It now emits a liveness probe through the same subscriber and asserts that first. **Any assert-absence test needs a positive control**; the same non-vacuity discipline as M4-R13/M4-R24 and BENCH-01.

> **How to verify a flake is fixed: validate the instrument before you trust a clean run (M9-R28, review round 8).** "I ran it N times and it passed" is worth nothing on its own — an intermittent failure that never reproduces on *your* machine yields exactly the same green as a real fix, which is the same vacuity trap one level up. So **reproduce the failure on the pre-fix revision first**, on the same box and harness, and only then measure the fix against that baseline. Done here with a detached scratch **worktree** at the parent commit (never the working tree): pre-fix **3/40 (7.5%)**, fix **178/178 clean** across default, CPU-saturated, `onnx`, and eight `--test-threads` settings. The same applies to the *guard*: to show a de-vacuified test really can fail, **mutate the thing it observes** — here, routing the capture to a throwaway buffer — and run that mutation against **both** revisions. The old test reported `ok`; the new one failed on its liveness probe. A fix for a flake and a fix for a vacuous guard are both claims about what happens when something *breaks*, so neither is verified by a run in which nothing broke.

> **When a fix changes mechanism, re-read the surviving prose against the new code — not against the old defect (M9-R29).** The M9-R28 fix landed with the helper carrying *two* accounts of the same race: a body comment describing the process-global subscriber that shipped, and a rustdoc header 35 lines above still describing the `Mutex` that was tried, measured at **7/30**, and discarded. Both were written in the same commit; the wrong one came first and is what an IDE shows on hover. Note the specific failure mode — a wrong **description of adjacent code** is worse than the wrong **inference** it replaced, because the inference at least described something that had happened, whereas this restated as fact the exact model ("the race needs a second concurrent caller, and there are exactly two") that the builder's own measurement had *disproved*. A maintainer following it would have reverted to `with_default` and reinstated an 11% flake. Two rules follow. **Prose is part of the diff:** when a commit changes the mechanism, every comment that survives it is unreviewed until re-read against the new code. And **keep one account, not two** — the duplication is what let the wrong one survive; the header now carries the explanation and the body only a pointer plus the one instruction that matters (*do NOT make this scoped again*).

> **A distinctness assertion is invariant under permutation — assert positively, per case (M9-R27).** BENCH-02 first checked only that the five messages were *different*. The reviewer mutated `guidance_for` by swapping the Windows and macOS bodies and the test **passed**, while a Mac user was told about DirectML: precisely the failure the assertion was written to prevent. It now asserts what each arm must *say*. The general form: **a guard that only pins a relationship between outputs cannot catch a permutation of them.** The replacement was re-verified against the same mutation — it fails it.

- CLI-03 — `bench_providers_never_advises_an_ep_feature_rebuild` (M9-R14, M9-R16): `--bench-providers` output must contain **no `ep-*` feature name**. The platform's accelerator is wired per-target, so naming a feature sends the operator to rebuild what they already have — and `ep-directml` is Windows-only, so naming it on macOS/Linux points at a backend their hardware cannot provide. **What it actually covers, precisely (M9-R20/M9-R23):** the `cfg(not(onnx))` message, end-to-end through the real binary — which no unit test in `bench` can reach, since that module is `onnx`-gated. In the **`onnx`** build this test is *near-vacuous*: with no `NER_*` configured the binary exits at model resolution before `format_report` runs, and on a box whose platform accelerator is present the no-accelerator branch would not fire anyway. **BENCH-01 covers that side.** Neither guard alone is sufficient, and believing one was is how this defect shipped twice.

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

> **What it needs, stated once so no summary has to guess: a working Claude Code and a human.**
> **Not** a credential on the proxy — the proxy holds none and forwards the client's own
> (`src/proxy.rs::messages_auth`; M6's live run got 200 on the first try with nothing configured).
> Every time a doc has compressed this to "needs a live `ANTHROPIC_API_KEY`" it has made the battery
> look blocked on something unobtainable instead of on an hour of the maintainer's time — most
> recently in M10. If you are reading a summary that says otherwise, the summary is stale.

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
  Never point this at a real table — synthetic data only. **The wiring is four steps, not one, and the
  one-line version of it was wrong for two milestones** — see
  [`MANUAL_VERIFICATION.md` → *CC-09's setup*](MANUAL_VERIFICATION.md#cc-09s-setup-which-is-not-one-line).
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
  shipped shape clears **its own floor** — the single-session default (`NER_POOL_SIZE` unset →
  `1 × 6` since M11 Track B, `1 × 12` before it; the personal shape since the 2026-07-17 flip) at
  **≥1.5×**, and the pooled centralized shape (`NER_POOL_SIZE=2` → `2 × 3`, was `2 × 6`) at
  **≥1.3×** — min of 3 reps each. Plus a **loose 15 s** absolute ceiling for order-of-magnitude
  regressions only.
  - **Why the floors are per-shape (M11 Track B), and why that is not a weakened bar.** One floor
    described both shapes while both divided the same base — at `pool=2` the derivation gave `2×6`,
    the same six threads per session the default got. M11 made the base *physical* cores, so the
    default is `1×6` and the centralized shape `2×3`: **half** the per-session threads, and a shape
    that differs 2× in thread count cannot honestly share a floor. Measured over four isolated runs
    on the reference box: default **1.95 / 2.04 / 1.80 / 2.08×**, centralized **1.46 / 1.56 / 1.56 /
    1.69×**. The old shared 1.5 floor sat *inside* the centralized band — a guard that fails
    intermittently on correct code, which is the worst kind: it trains the reader to re-run until
    green. Lowering *both* to 1.3 would have discarded the default shape's real ~1.9× headroom;
    dropping the centralized shape from the assertion would have been **M7-R1's own failure class**,
    a harness watching one shipped configuration while another ships unwatched. The floor now travels
    *with* the shape in `bar_shapes()`, so a third shipped shape cannot silently inherit a floor
    nobody measured for it.
  - **Read the centralized row against PERF-M7-04 or it reads as a regression it is not.** The pool
    exists for **concurrent throughput**, not single-request latency, and on that axis `2×3` measured
    **0.664 turns/s vs `2×6`'s 0.558 (+19%)** — the fastest shape in that table. It got better at its
    job and slower at the one it was never the right tool for.
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
  - **Its domain (M7-R13 / M7.1, restated by M11 Track B).** The guard still **skips below 4 and says
    so**, but the 2026-07-17 default flip (`pool 2 → 1`) changed *why*. Under the old `pool=2` default
    `intra` floored at 1 there, so the derived default *was* `PRE_M7_SHAPE` and the ratio was 1.0 by
    construction. Under `pool=1` the derivation is `intra = base`, so that identity holds **only at a
    base of 1**; between 2 and 3 the derived shapes add threads but too few to clear the floor
    reliably, so 4 stays the conservative line. **What M11 changed is the quantity compared against
    it:** the threshold now reads the *thread base*, not the logical core count, so on an SMT box it
    is half what it was and a 6-thread / 3-core machine falls below the line where it used to clear
    it. That is the honest reading — the guard's subject is the thread count the default actually
    gets, and that halved. Pinned in
    `onnx::thread_tests::the_default_gives_one_session_the_whole_base`: **the speedup scales with the
    thread base, and this guard has nothing dependable to say below 4.**
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
  - **M11 Track B's re-run, recorded here because the sweep RECORDS the change and never justified
    it.** Reference box, isolated, min of 3: `2×1` 4,934 ms (1.00×) · `1×1` 3,685 (1.34×) · `1×2`
    2,407 (2.05×) · `1×4` 2,048 (2.41×) · **`1×6` 1,992 (2.48×, the new default)** · **`1×12` 2,042
    (2.42×, the old default)** · `2×6` 2,009 (2.46×) · `4×3` 2,500 (1.97×). The default's move from
    `1×12` to `1×6` is **inside the noise** on single-request latency — which is precisely what
    M7-R2 predicted this harness could and could not resolve, and why the base was decided on
    mechanism rather than on these rows.
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
  - **M11 Track B's re-run — and the number that reframes Track B's whole latency story.** Reference
    box, 4 concurrent turns: `2×1` 0.314 turns/s · `1×12` 0.419 (the old default) · **`1×6` 0.485
    (the new default, +16%)** · `2×6` 0.558 (the old centralized shape) · `4×3` 0.645 · **`2×3` 0.664
    (the new centralized shape, +19%, and the fastest row measured)**. So the physical-core base
    **improved throughput on both shipped shapes**, and the centralized shape's single-request
    latency drop that forced PERF-M7-05's per-shape floor is the *other* side of the same trade —
    made on the axis the pool does not exist for.
- **THREAD-01** — `src/pii/onnx.rs::thread_tests` (unit, **no model needed**, runs in plain
  `cargo test --features onnx`). Pins the pure functions the threading rests on, as functions of
  `(pool, base)` and `(logical, physical)` — so the CI runner's own machine cannot decide whether
  they are correct. **That purity is the whole design**, and it is why M11 Track B could pin a hybrid
  P+E part, an SMT-disabled box and a cgroup-limited container without owning any of them.
  - `derive_intra_threads`: the oversubscription invariant **with its domain** — `pool × intra ≤
    base` while `pool ≤ base`, and `intra == 1` beyond it, where the derivation is out of moves.
    **The regimes are split on purpose (M7-R4):** the first version asserted
    `pool * intra <= cores.max(pool)` across both, which passes for `pool > cores` by widening the
    bound to the pool itself — green-lighting 8 threads on a 2-core box under a name claiming the
    opposite. A test may not hide its exception inside a `max`. **M11 restated it on the new base
    without unsplitting the regimes**, and the bases it sweeps are now *derived* from
    `(logical, physical)` pairs rather than a list of plausible integers: the composition
    `derive_thread_base → derive_intra_threads` is what actually runs, and a base of `0` or one above
    the granted parallelism would have broken the bound exactly where nobody was looking.
  - `derive_thread_base` / `thread_base_source` (M11 Track B): the `(logical, physical)` grid —
    SMT box `(12, 6) → 6` · SMT off in firmware `(6, 6) → 6` · hybrid P+E `(20, 14) → 14` ·
    container or affinity mask `(2, 32) → **2**`, the `min` case that keeps a physical-core count
    from overriding a cgroup quota · detection unavailable `(12, None) → 12`, which is the pre-M11
    behaviour exactly. Plus: the base never returns `0`, and **the base and the provenance string
    logged beside it cannot disagree** — two pure functions feed one startup-log line (M7-R5), and a
    line whose reason contradicts its number is worse than no line.
  - `resolve_pool_and_intra`: **both** knobs treat `0` and garbage as unset (M7-R5 — M7 shipped that
    guard on the new knob only, leaving `NER_POOL_SIZE=0` safe by two independent clamps while the
    startup log printed `pool_size=0, intra_threads=12`, which no arithmetic reconciles); an explicit
    value wins; and the default is `DEFAULT_POOL_SIZE`, the same constant `server.rs` uses — which is
    what makes M7-R1's harness/server drift structurally impossible. **M11 changed only the base it
    divides**, so `GLINER_POOL_SIZE`/`GLINER_INTRA_THREADS` inherited the new base with no second
    path to keep in step — the payoff of the single home M7-R1 bought.
- **NER-THREAD-01** — `tests/ner_perf.rs::m7_r3_intra_threads_changes_speed_not_detection`
  (`--features onnx`, `#[ignore]`d, needs a model). **`NER_INTRA_THREADS` must change speed, never
  detection** — fingerprints `(kind, span.start, span.end)` over prose, **a field past the
  `MAX_WINDOW_TOKENS` chunking window** (~660 tokens), CJK, and the fragment-prone `"Anthropic's"`
  shape, at intra 1 / 2 / 4 / 6 / all-cores, and asserts every set is **identical**. Measured: **194
  entities**, identical at every count, with a non-vacuity floor so a guard that detects nothing cannot
  pass.
  - **It sweeps to the LOGICAL core count, and M11 Track B deliberately did NOT narrow it to the new
    thread base.** Since M11 nothing *ships* at `intra = available_cores()` on an SMT box — the
    default derives from physical cores — and narrowing the sweep to match would still be wrong. This
    is a **detection-inertness** guard, not a shape-of-the-default guard: its claim is that *however*
    ONNX Runtime partitions the work the detections do not move, so **more partitions is strictly
    more coverage**, and the logical count is the largest partitioning `NER_INTRA_THREADS` can
    actually be set to on that box. A guard narrowed to the shipped default would stop covering the
    override that is documented as supported.
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
> irreducibly manual: **a human with a working Claude Code, pointed at the proxy, and eyes on two traces.** The
> automated mock coverage above remains the permanent guarantee; the battery is what proves it on real traffic.
>
> **It does not need a key configured on the proxy — and this line used to say it did.** The proxy holds *no*
> credential: it forwards the client's own, which is why M6's live run returned 200 on the first try with nothing
> configured. "A live key" was a compression of M5's true constraint (*a live provider*, back when the proxy was
> OpenAI-compat-only and Claude Code could not route through it at all), and it survived the milestone that made
> it false. It then cost real decisions — M10 recorded the battery as blocked on a credential nobody had, rather
> than on ten minutes at the keyboard. **`MANUAL_VERIFICATION.md` is the source of truth for *how to run* this;
> when a summary here disagrees with the runbook, the runbook wins.**

### Dependency footprint (M2.5-R1)
- DEP-01 — `tests/dependency_footprint.rs` (`default_build_excludes_the_onnx_and_hf_stack`): `cargo tree` on the **default** features, **for every released target**, must contain no `hf-hub`/`hf-xet`/`aws-lc`/`ort`/`tokenizers` — the ONNX/HF stack (heavy, native) stays behind the `onnx` feature.

### Decisions & open points
- **Coverage scope — DECIDED (2026-07-13; see ROADMAP M4).** *Language (NER):* XLM-R's
  10 languages (ar/de/en/es/fr/it/lv/nl/pt/zh). *Structured — three tiers:* universal
  (email/IBAN/card/secret) always on; national IDs (US SSN, IT CF, GB NINO, ES DNI/NIE,
  FR NIR, DE Steuer-ID, NL BSN, PT NIF, LV code, zh Resident ID — full list in the mapping
  table above) **always on regardless of `PII_LOCALES`**; the **domestic-phone** tier (numbers with
  no `+CC`) gated by `PII_LOCALES` — GB/DE since M8.1, and since **M10 nine regions
  (`de es fr gb it lv nl pt cn`), all on by default**, with the variable replacing that set rather
  than adding to it. Phone: US + `+CC` are universal.
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

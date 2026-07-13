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
- E2E-01 (TC-01) — multi-PII chat message, IT + EN.
- E2E-02 (TC-02) — PII in a CSV `tool_result`.
- E2E-03 (TC-03) — secret + email in a shell command output.
- E2E-04 (TC-04) — all categories in a `SELECT … FROM DUAL` result.
- E2E-BIN — `tests/binary_smoke.rs`: boots the **compiled binary** (`main` → `from_env` → `run`) against a mock upstream for one PII round-trip; the only test that exercises the real process (kept to a single case).

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

### Algorithmic complexity — detection must stay **linear** (`tests/complexity.rs`, M4-R19)

**Availability is a privacy property here.** M4-R17's fix made candidate generation see *every*
overlapping match — correct, and **O(n²)** on the two patterns with no length bound (`Email`, `Secret`):
a ~1 MB `content` field, far under the 16 MiB body limit, pegged a core for **minutes** on an
**unauthenticated** path (151 s at 200 KB). A proxy that is down forwards nothing — and protects nothing.

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
- **Coverage is preserved, and that is tested separately** (dropping candidates is how M4-R17 made PROP-03
  pass *vacuously* — this could not be asserted, it had to be argued and pinned):
  `an_email_chain_leaves_nothing_detectable` (a chained `a@b.com@c.com`, whose second email reaches past
  the first — the **fixpoint** catches the remainder one pass later) and
  `a_secret_hidden_inside_a_secret_is_still_covered` (a nested `sk-…` is always *contained* in the outer
  match, so the rescan added nothing there to begin with).

> **The rule for a new recognizer:** if its match length has **no upper bound**, it must not take
> `Scan::Overlapping`. Bounded recognizers rescan; unbounded ones rely on the fixpoint.

> **The blind spot these guards had (M4-R24, open).** DOS-01…03 all pin *one* entity (DOS-03's card row
> coalesces to k≈1), so they measure **detection** scaling in the field *length* — and were **blind** to
> `Vault::mask` being **O(n²) in the entity *count***: its right-to-left `replace_range` splice shifts the
> tail once per entity, so a 13 MiB field of many small values ≈ 7 min of CPU while the *same* 13 MiB as one
> entity masks in ~0.2 s. **A complexity guard must vary entity count, not just field size** — this is the
> M4-R13 "the corpus has a shape, and that shape is a blind spot" lesson, recurring on the DoS guards
> themselves. The missing guard is **DOS-04** (e.g. `"a@b.co "`×1 M, or an SSN/phone repeat): `mask_all`
> within budget, still masked and round-tripping. Add it with the M4-R24 fix (single-pass splice).

### Fail-closed — masking (`src/pii/anonymizer.rs`, M4-R20)
- FC-06 — `mask_all_blocks_when_it_cannot_reach_a_fixpoint`: exhausting `MAX_MASK_PASSES` must return
  `Err` (→ `PrivacyStage` blocks, 400), **not** forward the text. A synthetic `NeverConverges` detector
  always reports the *first character* as PII, so masking it exposes a new first character and the fixpoint
  is never reached. The bound gives *eventual* convergence, never convergence **within four passes**, so
  the fixpoint is **confirmed**, not assumed. Also asserts the error carries **no input text** (DBG-02's
  never-log-raw-PII rule) and that a normally-converging detector still returns `Ok` — so the guard can't
  pass by breaking masking for everyone.

### M5 — integration & performance (planned)
- E2E-INT-01 *(planned)* — real-provider smoke against **Anthropic** (OpenAI-compat endpoint; opt-in, needs a key, never in CI): a PII round-trip returns the restored value while the request left masked.
- E2E-INT-02 *(planned, manual)* — the **dual-run** check with `RUST_LOG=…=trace`: Run A (`PII_DEBUG_SKIP_DEMASK=1`) → client gets the placeholders; Run B (normal) → client gets the restored values. Proves the whole chain end-to-end; trace logging re-checks DBG-02 (never-log-raw-PII) on **real** data.
- E2E-02 / E2E-04 *(to implement in M5)* — the two cataloged old-proxy scenarios (CSV `tool_result`; `SELECT … FROM DUAL`) against a mock — still pending.
- PERF-01 *(planned)* — load / throughput harness: concurrent connections, large bodies, streaming throughput; latency / RAM of the mask → forward → de-mask path (NER on/off).

### Dependency footprint (M2.5-R1)
- DEP-01 — `tests/dependency_footprint.rs` (`default_build_excludes_the_onnx_and_hf_stack`): `cargo tree` on the **default** features must contain no `hf-hub`/`hf-xet`/`aws-lc`/`ort`/`tokenizers` — the ONNX/HF stack (heavy, native) stays behind the `onnx` feature so the shipped default build is native-dep-free.

### Decisions & open points
- **Coverage scope — DECIDED (2026-07-13; see ROADMAP M4).** *Language (NER):* XLM-R's
  10 languages (ar/de/en/es/fr/it/lv/nl/pt/zh). *Structured — three tiers:* universal
  (email/IBAN/card/secret) always on; national IDs (US SSN, IT CF, GB NINO, ES DNI/NIE,
  FR NIR, DE Steuer-ID, NL BSN, PT NIF, LV code, zh Resident ID — full list in the mapping
  table above) **always on regardless of `PII_LOCALES`**; FP-prone recognizers (national *phone*
  formats) opt-in via `PII_LOCALES` — none yet. Phone: US + `+CC` are universal.
- **Placeholder format — DECIDED: `[KIND_N]`** (e.g. `[EMAIL_1]`), ASCII. Tests
  still assert invariants (raw absent, typed placeholder present, exact roundtrip)
  rather than literal tokens, so they stay robust to future tweaks.
- **IBAN validation strictness** (open) — mask any IBAN-shaped string
  (privacy-first, catches synthetic values) vs require a valid mod-97. Current
  lean: structure-based detection, mod-97 as a confidence signal only.
- **SECRET patterns** (open) — which key formats to cover (OpenAI `sk-…`,
  Anthropic `sk-ant-…`, AWS `AKIA…`, generic high-entropy tokens?).

## Running

- `cargo test` — unit + property + integration (from M1 onward).
- End-to-end against a mock provider — harness added in M1.

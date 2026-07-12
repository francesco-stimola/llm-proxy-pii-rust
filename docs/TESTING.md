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
| SECRET | `Secret` *(to add to `PiiKind`)* | structured |
| PRIVATE_PERSON | `Person` | ML NER (M2) |

## Test catalog

The concrete battery the tool must pass, grouped by level. Data-driven
structured-PII cases live in `tests/corpus/pii_cases.json` (consumed by the Rust
tests); the integration / e2e / regression cases are behavioral and described here.

### Unit — recognizers (`tests/corpus/pii_cases.json` → `recognizers`)
Per category, positives (detected with the right `PiiKind`) and negatives (no
false positive): `email`, `phone`, `ssn`, `credit_card`, `iban`, `secret`.

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
- EVAL-01 — `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d): scores a live model against `ner_cases.json` through the hybrid resolver. Run with `-- --ignored --nocapture` once a model is configured.

### Decisions & open points
- **Locale coverage — DECIDED: IT + US.** Italian and US phone numbers; IBAN
  including Italian; US SSN. The corpus carries both.
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

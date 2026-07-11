# Development log

Newest first. One entry per meaningful change — note *what* and *why*, not just
*what*. This is the running history so context is never lost between sessions.

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

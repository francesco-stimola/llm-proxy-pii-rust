# llm-proxy-pii-rust — working notes for Claude

A privacy proxy in front of an OpenAI-compatible LLM: it masks PII locally
**before** requests leave, and restores it in the response. See `README.md` and
`docs/ARCHITECTURE.md`.

## Read these first
- **`docs/ROADMAP.md` is the single source of truth for what's next.** Read it
  before starting work; keep the checkboxes current as work lands.
- `docs/ARCHITECTURE.md` — design & the fail-closed model. `docs/TESTING.md` —
  test strategy & catalog. `docs/DEVLOG.md` — running history (append here).
  `docs/SETUP.md` — no-admin toolchain.

## Non-negotiables (quality bars)
- **Fail closed.** For a privacy tool the failure mode *is* the product: on any
  unexpected input, shape, or endpoint, block or scrub — never forward raw PII.
- **Never log raw PII.** Log kinds/counts, never values (the `Config` Debug and
  the `Confidence` audit log already follow this).
- **Textbook & lean.** Idiomatic Rust, low RAM/CPU, no over-engineering.
- Hybrid detection: deterministic recognizers for **structured** PII, ML NER (M2)
  only for **unstructured** entities. CPU-first. Placeholders `[KIND_N]`. Locales
  **IT + US** (M5 broadens). Detection engines sit behind the `PiiDetector` trait.

## Build & test
- Toolchain is MSVC, no-admin — see `docs/SETUP.md`. Every shell needs the MSVC
  env (auto-applied in-project via `.claude/settings.local.json`, or by
  dot-sourcing the Build Tools `devcmd.ps1`).
- `cargo test` must be **green with no warnings** before a change is "done".
  Add tests for every behavior change; add **adversarial** cases for detection
  changes (a miss = a leak). Catalog new tests in `docs/TESTING.md`.

## Git
- **Never commit with a real/work email.** A masked `user.email` is set
  repo-locally — keep it.
- **Batch commits** into coherent chunks (avoid spam); keep feature commits
  separate from docs/infra commits. **Don't push without asking.**
- `.claude/settings.local.json` is gitignored (machine-specific) — never commit it.

## Docs policy
English is canonical. Only **root-level** docs get an Italian `<name>.it.md`
mirror; everything under `docs/` stays English-only.

## Workflow — build → review → formalize
Development runs as a two-role loop:
- **Builder** implements the current ROADMAP milestone + tests, appends to
  DEVLOG, and commits the feature work.
- **Reviewer** (`.claude/agents/reviewer.md`) verifies independently, reviews
  source + docs, and records findings back into `docs/ROADMAP.md` as new `[ ]`
  items with **fix + test each** — which the builder then closes. The reviewer
  edits docs, **not** source, and keeps the working tree clean.
- **Findings lifecycle:** OPEN findings live in ROADMAP as `[ ]`; once a review's
  findings are all closed, collapse them to a one-line `docs/DEVLOG.md` pointer so
  ROADMAP stays forward-looking — the detail lives in DEVLOG.

A milestone is done when: its ROADMAP items are checked, tests are green,
DEVLOG and the affected docs (ARCHITECTURE/TESTING) reflect reality.

# llm-proxy-pii-rust — working notes for Claude

A privacy proxy in front of an OpenAI-compatible LLM: it masks PII locally
**before** requests leave, and restores it in the response. See `README.md` and
`docs/ARCHITECTURE.md`.

## Read these first
- **`docs/ROADMAP.md` is the single source of truth for what's next.** Read it
  before starting work; keep the checkboxes current as work lands. Its **milestone
  status table** at the top is the whole backlog at a glance — one row per milestone,
  each linking to its section — so you never have to scroll for "what's open?".
  **Keep that table lean and it stays useful:** one row *per milestone* (a release tag is
  noted next to the milestone it was cut from, never its own row), and the Status cell is a
  **single label** — ✅ complete · 🔨 code-complete · 📋 planned — never a prose paragraph.
  All detail (findings, counts, dates, caveats) lives in the milestone's own section, not the
  table. It's easy to forget and let a row grow verbose — don't.
  **A tag not yet cut is written `tag `vX.Y.Z` (planned)`** — so the table can name the version it
  is heading for without claiming something false, and you never face the choice between a premature
  claim and a silent release. Drop `(planned)` when the tag exists. Don't rely on remembering:
  `release-build-publish.yml` **refuses to publish** a `v*` tag this table doesn't name as cut.
- `docs/ARCHITECTURE.md` — design, the fail-closed model, and **the invariants the
  detection layer rests on** (no-abandoned-bytes, mask-to-a-fixpoint, ASCII word
  boundaries). `docs/TESTING.md` — test strategy & catalog, incl. the property
  tests that pin those invariants. **Read these before changing detection** — they
  exist because each one was a leak first.
- `docs/reviews/` — the full record of every review finding, one file per
  milestone. You need it only when a ROADMAP ledger row is unclear.
- `docs/DEVLOG.md` — running history (append here). `docs/SETUP.md` — no-admin toolchain.

## Non-negotiables (quality bars)
- **Fail closed.** For a privacy tool the failure mode *is* the product: on any
  unexpected input, shape, or endpoint, block or scrub — never forward raw PII.
- **Never log raw PII.** Log kinds/counts, never values (the `Config` Debug and
  the `Confidence` audit log already follow this).
- **Textbook & lean.** Idiomatic Rust, low RAM/CPU, no over-engineering.
- Hybrid detection: deterministic recognizers for **structured** PII, ML NER (M2)
  only for **unstructured** entities. CPU-first. Placeholders `[KIND_N]`. Locales
  **IT + US** (M5 broadens). Detection engines sit behind the `PiiDetector` trait.

## Decisions that are the human's
Some choices are not the builder's to make. On any of these, **stop and ask** —
present **numbered options** with what each one costs and gives up, say which you'd
pick, and wait:
- **Product-visible behavior** — a new refusal, reduced coverage, a changed meaning
  for a config variable, a different error body or exit code.
- **A threshold with functional consequences** — a budget, a limit, a confidence
  cutoff. "It seemed a reasonable default" is not authority to pick one.
- **Anything blocking for the tag** — if the answer decides whether a release ships.

Everything else: **proceed**. That is the default, and asking permission for routine
work is its own failure. Nor should one open question stall the whole task — do
everything that doesn't depend on the answer first, *then* ask.

## Build & test
- Toolchain is MSVC, no-admin — see `docs/SETUP.md`. Every shell needs the MSVC
  env (auto-applied in-project via `.claude/settings.local.json`, or by
  dot-sourcing the Build Tools `devcmd.ps1`).
- **Only ever kill a process you started yourself, by the PID you got when you
  started it.** The machine also runs a long-lived instance of *this same binary*
  that isn't yours — so killing by name or by port (`Get-Process <name> |
  Stop-Process`, `taskkill /IM`, "whatever is holding that port") is destructive and
  leaves no trace in the test output. If the port you wanted is taken, bind a
  different one; never free it.
- `cargo test` must be **green with no warnings** before a change is "done".
  Add tests for every behavior change; add **adversarial** cases for detection
  changes (a miss = a leak). Catalog new tests in `docs/TESTING.md`.

## Git
- **Never commit with a real/work email.** A masked `user.email` is set
  repo-locally — keep it.
- **No `Co-Authored-By: Claude` trailer, and no "generated with" footer** — not in
  commits, not in PR bodies. This **overrides** the default Claude Code behavior, so
  drop the trailer even when the harness suggests it. The commit message says *what
  changed and why*; authorship of this repo is the human's.
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
  source + docs, and records findings — which the builder then closes. The reviewer
  edits docs, **not** source, and keeps the working tree clean.

### Findings lifecycle — ledger vs record
A finding has **one home for its whole life**. It is never copied, and never moved.

- **`docs/ROADMAP.md` = the ledger.** A table row per finding: id · title · severity ·
  `[ ]`/`[x]`. Nothing more. This is what makes "what's open?" answerable at a glance.
- **`docs/reviews/M<n>.md` = the record.** The finding in full: repro, mechanism,
  `file:line`, the **fix**, the **test**, and — once closed — the closure note appended
  *to the same entry*. Give each an `<a id="m4-r19">` anchor so the ledger can link it.
- **Closing a finding** = flip the box in the ledger + append the closure note to its
  entry in the record. That's it.
- **Never** paste a finding's text into ROADMAP, and never leave both an "original
  finding" and a closure note side by side — that double-recording is what bloated this
  file to 1000+ lines once already.

**Promote what outlives the review.** If a finding taught the codebase a *rule* — an
invariant, a guard, a tradeoff we now accept on purpose — write it into
`ARCHITECTURE.md` or `TESTING.md`, **next to the thing it governs**. The review archive
is history; those two files are the design. A lesson left only in the archive is a lesson
the next builder will re-learn the hard way.

A milestone is done when: its ROADMAP items are checked, tests are green, and DEVLOG +
the affected docs (ARCHITECTURE / TESTING) reflect reality.

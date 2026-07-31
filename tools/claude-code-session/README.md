# Claude Code → this proxy: the live-verification workspace

Open **this directory** as its own VS Code workspace (or `cd` here and run `claude`) and Claude
Code routes its native `POST /v1/messages` traffic through the local proxy instead of
`api.anthropic.com`. `.claude/settings.json` sets **only** `ANTHROPIC_BASE_URL`, and project
settings are workspace-scoped — your global `~/.claude/settings.json` and every other project are
untouched.

That is all this directory does. The tests themselves live in one place each:

| | |
|---|---|
| **What to ask, and what to expect** (per scenario, de-mask OFF and ON) | [`docs/TESTING.md` → CC-01…CC-09](../../docs/TESTING.md#cc-battery) |
| **How to run them** (proxy setup, the dual-run, the DBG-02 grep, the traps) | [`docs/MANUAL_VERIFICATION.md`](../../docs/MANUAL_VERIFICATION.md) |

> **`CC-*` is you at the keyboard; `E2E-INT-*` is cargo** (`tests/anthropic_smoke.rs`, the
> automated real-provider smoke over both routes). Run both before cutting a tag — they prove
> different things.

## The fixtures this directory hosts

| path | used by | what it is |
|---|---|---|
| `fixtures/contacts.csv` | CC-03, CC-04, CC-07 | a contacts export: names, emails, phones, IBANs, SSNs |
| `fixtures/deploy-config.env` | CC-06 | fake API keys (`sk-ant-…`, `sk-…`, `AKIA…`) + an ops email |
| `fixtures/cc09-setup.sql` | CC-09 | creates a synthetic `cc09_customers` (one row, six categories). Run it **out-of-band** against a throwaway DB, never through the proxy |
| `fixtures/customer-lookup.sql` | CC-09 | the **PII-free** `SELECT * FROM cc09_customers`. The values must ride in the *result*, not the query text — otherwise reading the file masks them before the query runs and the `tool_result` path is never exercised |
| `fixtures/cc09-mcp.example.json` | CC-09 | template for `.mcp.json`, the SQL tool the scenario needs. Copy up one level and fill in the two absolute paths |
| `.mcp.json` | CC-09 | **gitignored** (machine-specific paths). Wires a SQL MCP server that sees *only* the throwaway SQLite — so this session cannot reach a real database even by mistake |
| `scratch/` | CC-04, CC-09 | gitignored. Point tool-*writing* scenarios here: with the de-mask skipped, a tool call writes `[EMAIL_1]` into a file — worth seeing, never worth committing. Also holds `cc09.db` and its connections file |

**Every value in every fixture is synthetic.** Never put real PII in this directory — the whole
point is to watch it travel.

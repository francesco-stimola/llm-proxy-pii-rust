# Claude Code → this proxy: the live-verification workspace

Open **this directory** as its own VS Code workspace (or `cd` here and run `claude`) and Claude
Code routes its native `POST /v1/messages` traffic through the local proxy instead of
`api.anthropic.com`. That is what makes the **CC battery** runnable.

| | |
|---|---|
| **Which tests run here** | **CC-01…CC-09** — the manual battery: [`docs/TESTING.md`](../../docs/TESTING.md#cc-battery) |
| **How to run them** | [`docs/MANUAL_VERIFICATION.md`](../../docs/MANUAL_VERIFICATION.md) — the step-by-step, incl. the OFF/ON dual-run and the DBG-02 grep |
| **The automated twin** | `tests/anthropic_smoke.rs` (E2E-INT-01) — `cargo test --test anthropic_smoke -- --ignored`. Different thing: it checks the *routes*, this checks a real *client*. |

**By prefix: `CC-*` is you at the keyboard, `E2E-INT-*` is cargo.** Run both before a tag.

## What's in here

| path | used by | what it is |
|---|---|---|
| `.claude/settings.json` | all | sets **only** `ANTHROPIC_BASE_URL` → the local proxy. Workspace-scoped: your global `~/.claude/settings.json` and every other project are untouched. |
| `fixtures/contacts.csv` | CC-03 | a contacts export: names, emails, phones, IBANs, SSNs. Ask Claude Code to read it → the PII arrives as a `tool_result`. |
| `fixtures/deploy-config.env` | CC-06 | fake API keys (`sk-ant-…`, `sk-…`, `AKIA…`) + an ops email — the SECRET category the old proxy's ML model missed. |
| `fixtures/customer-lookup.sql` | CC-09 | `SELECT … FROM DUAL` returning every structured category. The old proxy's **TC-04**, reproduced through a real MCP SQL tool — no schema or real data needed. |
| `scratch/` | CC-04, CC-06 | gitignored. Point tool-writing scenarios here. |

**Every value in every fixture is synthetic.** Never put real PII in this directory — the whole
point is to watch it travel.

## The two-minute version

```powershell
# repo root — the PRODUCT is the hybrid, so build and run it as such
cargo build --features onnx
$env:LISTEN_ADDR='127.0.0.1:8787'
$env:UPSTREAM_PROVIDER='anthropic'
$env:UPSTREAM_BASE_URL='https://api.anthropic.com'
$env:RUST_LOG='llm_proxy_pii_rust=trace'
$env:NER_MODEL_REPO='jiting/xlm-roberta-base-ner-hrl_onnx'
$env:NER_REQUIRED='1'      # non-negotiable — see below
.\target\debug\llm-proxy-pii-rust.exe
```

Then open this directory, start Claude Code, and work through
[`MANUAL_VERIFICATION.md`](../../docs/MANUAL_VERIFICATION.md).

## Three things that will bite you

- **`NER_REQUIRED=1`, always.** Without it a missing model degrades to structured-only **in
  silence** (the deliberate fail-open for names). The first live run was done that way by
  accident: email and IBAN masked perfectly — they are deterministic recognizers — so it looked
  green while the NER **never ran**. A `Person` would have gone upstream in clear. This flag
  turns that into a fatal startup error.
- **`ANTHROPIC_BASE_URL` has no fallback.** If the proxy is down, the session fails outright —
  it does not quietly go back to `api.anthropic.com`. That is exactly why this lives in a
  workspace and not in your global config: close this window and the effect is gone.
- **`count_tokens` 404s** — deliberately out of M6's scope. It was never called in an ordinary
  session, so it has not bitten yet. Note its body would carry the *whole conversation*, so
  serving it would mean masking it too, never a blind passthrough.

## Auth: you don't need a token

Measured 2026-07-16: a subscription-logged-in Claude Code, pointed here with **nothing**
configured, got **200 first try** — it forwards its own credential, and the proxy passes it
verbatim while holding no key at all. (The docs and several write-ups claim otherwise; they are
wrong for 2.1.211. See `MANUAL_VERIFICATION.md` → *Auth*.) If a session ever 401s, set
`ANTHROPIC_AUTH_TOKEN` or `ANTHROPIC_API_KEY` — the proxy accepts either.

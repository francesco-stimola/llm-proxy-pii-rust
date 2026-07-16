# Claude Code → this proxy: the live-verification fixture (M6)

Open **this directory** as its own workspace (a second VS Code window, or
`cd` here and run `claude`) and Claude Code routes its native `POST /v1/messages`
traffic through the local proxy instead of `api.anthropic.com`.

That is the last open gate for `1.0.0` — the one thing no mock can prove (see
[`docs/ROADMAP.md`](../../docs/ROADMAP.md#m6) and
[`docs/MANUAL_VERIFICATION.md`](../../docs/MANUAL_VERIFICATION.md)). This fixture
exists so whoever holds credentials can run it in two minutes instead of
reconstructing the setup from prose.

## What it is

`.claude/settings.json` here sets exactly one thing:

```json
{ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787" } }
```

`ANTHROPIC_BASE_URL` is scheme+host+port only — Claude Code appends `/v1/messages`
itself. Project settings are scoped to the workspace you open, so this affects
**only** a session started in this directory: your global `~/.claude/settings.json`
and every other project are untouched.

## Run it

1. **Start the proxy** (from the repo root). No key — the client passes its own
   credential, which is the whole point of M6's auth design:

   ```powershell
   $env:LISTEN_ADDR='127.0.0.1:8787'
   $env:UPSTREAM_PROVIDER='anthropic'          # gates the /v1/messages route
   $env:UPSTREAM_BASE_URL='https://api.anthropic.com'
   $env:RUST_LOG='llm_proxy_pii_rust=trace'    # trace = the masked body, for DBG-02
   cargo run
   ```

2. **Open this directory** in a second VS Code window and start Claude Code.

3. **Send a prompt carrying fake PII** (never real data):

   > Reply with exactly this sentence and nothing else: contact
   > jane.doe@example.com, IBAN IT60X0542811101000000123456.

4. **Check both ends:**
   - The reply you see contains the **real** values → the response de-mask works.
   - The proxy's `forwarding masked request body upstream` trace shows
     `[EMAIL_1]` / `[IBAN_1]` → the provider never saw the real ones.
   - Grep the proxy's **stdout** for `jane.doe@example.com` → **nothing**
     (never-log-raw-PII / DBG-02, on real traffic rather than the synthetic case
     `tests/log_safety.rs` pins).

   For the full Run A / Run B dual-run (`PII_DEBUG_SKIP_DEMASK=1` first, then
   normal), follow [`docs/MANUAL_VERIFICATION.md`](../../docs/MANUAL_VERIFICATION.md).

## Known rough edges (measure them, don't assume them)

- **Auth.** Whether Claude Code forwards a *subscription* OAuth credential to a
  custom base URL is not something we could settle from the docs. If the session
  401s, set `ANTHROPIC_AUTH_TOKEN` (→ `Authorization: Bearer`, forwarded verbatim)
  from `claude setup-token`, or `ANTHROPIC_API_KEY` (→ `x-api-key`). The proxy
  accepts **either**, and never puts an OAuth token in `x-api-key` (Anthropic 401s
  that).
- **`count_tokens` 404s.** `POST /v1/messages/count_tokens` is deliberately out of
  M6's scope, so the proxy 404s it (fail-closed). It may degrade context
  compaction / token budgeting. Worth knowing before you conclude something is
  broken — and note that body carries the **whole conversation**, so serving it
  would mean masking it too, never a blind passthrough.
- **No fallback.** With a custom base URL, Claude Code does not fall back to
  `api.anthropic.com`. If the proxy is down the session simply fails — which is
  exactly why this lives in its own directory and not in a global config.

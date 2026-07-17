# Manual verification — a real Claude Code session through the proxy

The runbook for **CC-01…CC-09** in [`TESTING.md`](TESTING.md#cc-battery) — the **manual** half of
the live verification. It drives a **real Claude Code session** against **real Anthropic**
through the proxy, and compares what two runs *logged* and *returned to the client*. No automated
harness asserts on human-inspected trace logs; this replaces that inspection with a repeatable
checklist.

> **Manual vs automated, by prefix.** `CC-*` (here) is a human at the keyboard. `E2E-INT-01`
> (`tests/anthropic_smoke.rs`) is the **automated** opt-in real-provider smoke over both routes —
> `cargo test --test anthropic_smoke -- --ignored --nocapture`. Run both before a release: they
> check different things, and only this one exercises a real *client*.

The workspace and the fixtures live in [`../tools/claude-code-session/`](../tools/claude-code-session/)
— open **that directory** and Claude Code routes itself here. The scenario list is in
[`TESTING.md`](TESTING.md#cc-battery); this file is *how* to run it.

## What this proves that automated tests can't

The mock-upstream e2e suites (`tests/proxy_e2e.rs`, `tests/anthropic_messages_e2e.rs`) prove the
proxy's own logic is correct. They cannot prove the **real** round-trip holds: a real model may
reformat a placeholder or refuse to echo it; a real client sends shapes we never imagined; auth
wiring can fail in ways only the real endpoint shows. The first live run made that concrete —
it surfaced three things no mock had:

- Claude Code sends `POST /v1/messages?beta=true` (a query param), a `system` **block array**
  carrying a billing header, and a `metadata.user_id` we do not scan (Anthropic's own device /
  account / session ids, going to Anthropic — outside this tool's threat model, but worth
  knowing it is there).
- It **never called** `count_tokens` in an ordinary session, so the 404 we return for it (out of
  M6 scope) did not bite.
- The auth question below, which the documentation had exactly backwards.

## Auth — the client's own credential, measured (not assumed)

> **Claude Code forwards its already-authenticated credential to a custom `ANTHROPIC_BASE_URL`.**
> Measured 2026-07-16: a subscription-logged-in session, pointed at this proxy with **no token
> configured anywhere**, got **200 on the first request, no 401, no retry** — Anthropic accepted
> the credential the proxy forwarded verbatim, while `UPSTREAM_API_KEY` was **unset**.

That is worth stating loudly because the previous revision of this file asserted the opposite
("a Claude subscription login is an OAuth token scoped to Claude Code's own flow… not usable"),
and third-party write-ups still claim a custom base URL *requires* setting
`ANTHROPIC_AUTH_TOKEN`. Both are wrong for Claude Code 2.1.211. **We only know because we tested
it instead of believing it** — the same reason the rest of this checklist exists.

So the recommended mode is **the proxy holds no credential at all**. The auth rule
(`src/proxy.rs`, `messages_auth`) is:

> client `Authorization` (forwarded **verbatim**) → client `x-api-key` → the configured
> `UPSTREAM_API_KEY` as `x-api-key` → **401**.

An OAuth token is **never** placed in `x-api-key` (Anthropic 401s that). If a session ever does
401, set `ANTHROPIC_AUTH_TOKEN` (→ `Authorization: Bearer`, from `claude setup-token`) or
`ANTHROPIC_API_KEY` (→ `x-api-key`) in the session's env — the proxy accepts either.

## Prerequisites

**Run the product, not a subset.** The shipped proxy is the *hybrid* — structured recognizers
**and** the NER. Verify with anything less and CC-02 tests nothing.

```powershell
# from the repo root — the aliases' --target-dir is relative to your cwd
$env:LISTEN_ADDR='127.0.0.1:8787'
$env:UPSTREAM_PROVIDER='anthropic'                       # gates the /v1/messages route
$env:UPSTREAM_BASE_URL='https://api.anthropic.com'
$env:RUST_LOG='llm_proxy_pii_rust=trace'                 # the masked-body trace, for DBG-02
$env:NER_MODEL_REPO='jiting/xlm-roberta-base-ner-hrl_onnx'   # revision-pinned, cached after the first fetch
$env:NER_REQUIRED='1'                                    # see the box below
# UPSTREAM_API_KEY deliberately UNSET — the client passes its own credential
cargo run-onnx     # rebuilds, THEN runs target\onnx\debug\ — see the box below
```

Confirm **both** lines before trusting a single result:

```text
INFO … ONNX NER detector loaded model="…model_quantized.onnx" pool_size=1 intra_threads=…
INFO … listening on http://127.0.0.1:8787
```

(`pool_size=1` is the default since 2026-07-17 — one session, the whole box; `intra_threads` is the
derived per-session count. A centralizing operator who set `NER_POOL_SIZE=2` would see that here.)

> **`NER_REQUIRED=1` is not optional here, and this is why.** By default a missing or
> unloadable NER degrades to structured-only **silently** (the deliberate fail-*open* posture
> for names). The first live run was done that way by accident: email and IBAN masked fine — they
> are *deterministic* recognizers — so everything looked green while **the NER was never
> running**. A `Person` would have gone upstream in clear. `NER_REQUIRED=1` turns that silent
> downgrade into a fatal startup error, which is the only way "is the hybrid actually on?" stops
> being a question you can get wrong. (DEVLOG 2026-07-16.)

> **The footgun this used to carry — now closed by construction: `cargo test` silently un-did
> your onnx build.** `cargo build` and `cargo build --features onnx` write the **same** file
> (`target/debug/llm-proxy-pii-rust.exe`), so *any* default-features command — `cargo test`,
> `cargo clippy --all-targets`, a plain `cargo build` — overwrote the hybrid binary with a
> structured-only one, with no warning. It was not hypothetical: it happened while writing this
> file, minutes after the paragraph above was added, and `NER_REQUIRED=1` is what caught it.
>
> **The fix is a build directory, not a discipline** (2026-07-16). Cargo cannot name a binary per
> feature — features are additive and crate-wide, `required-features` gates *whether* a bin builds
> and not what it is called, and "not onnx" is inexpressible. So the hybrid gets its **own target
> dir** via the `.cargo/config.toml` aliases:
>
> | Path | Contains |
> |---|---|
> | `target/onnx/debug/llm-proxy-pii-rust.exe` | **the hybrid — always.** Only the aliases write here, so no default-features command can reach it |
> | `target/debug/llm-proxy-pii-rust.exe` | whatever the last default-features command left. Structured-only *by convention* — an explicit `cargo build --features onnx` still writes a hybrid here |
>
> **But read the asymmetry, because it is the whole point.** Only the first row is a guarantee. The
> second is a habit, and the guarantee runs in the safe direction: the *dangerous* mistake
> (structured-only masquerading as the hybrid) is what `NER_REQUIRED=1` makes fatal.
>
> **And the clobber was loud — this trades it for something quiet.** Overwriting *destroyed* the
> hybrid, so `NER_REQUIRED=1` caught it instantly. A binary in its own directory is never
> destroyed; it goes **stale**, and a stale hybrid loads the NER and prints both green lines above.
> `NER_REQUIRED` detects a *missing feature*, never *old code*. That is why the recipe runs
> **`cargo run-onnx`** rather than launching the path directly: cargo rebuilds first, so staleness
> is impossible by construction instead of by remembering to rebuild. Belt and braces — the alias
> makes the right build automatic, the flag makes the wrong one fatal.

## Procedure

For **each** scenario CC-01…CC-09 in [`TESTING.md`](TESTING.md#cc-battery):

1. **Run OFF** — proxy started as above (`PII_DEBUG_SKIP_DEMASK` unset).
   Open [`../tools/claude-code-session/`](../tools/claude-code-session/) as its own VS Code
   workspace, start Claude Code, run the scenario's prompt.
   - ✅ The reply you see carries the **real** values.
   - ✅ The proxy's `forwarding masked request body upstream` trace shows `[EMAIL_1]` /
     `[PERSON_1]` / … — never the real ones.

2. **Run ON** — restart the proxy with `$env:PII_DEBUG_SKIP_DEMASK='1'`, same everything else,
   and send the **same** prompt.
   - ✅ The reply you see carries the **placeholders** — this is literally what the provider got.
   - ✅ The trace is **identical** to Run OFF's (request-side masking does not depend on the flag).
   - Work in `scratch/` for tool-writing scenarios: the session will act on placeholders.

3. **DBG-02 grep, on both runs** — search the proxy's **stdout** for every raw value the scenario
   used. Expected: **zero** hits.

   ```powershell
   $esc = [char]27
   $clean = Get-Content <proxy-stdout-log> | ForEach-Object { $_ -replace "$esc\[[0-9;]*m","" }
   'bob@test.com','IT60X0542811101000000123456' | ForEach-Object {
     "{0}: {1}" -f $_, ($clean | Select-String -SimpleMatch $_ | Measure-Object).Count
   }
   ```

## What "holds" means

Run OFF alone proves the client got a sensible reply. Run ON alone proves the request left
masked. Neither alone proves they are the **same** round-trip. Together, on the *same* input
against the *same* real provider, they show: the value that left masked (ON) is the exact value
the client gets restored (OFF) — the full chain, not two independently-plausible halves.

## If it doesn't hold

- **Raw PII in the outgoing body or the trace** → a masking gap. Treat it as a **leak**, not a
  quality issue: record it in [`reviews/`](reviews/) at the highest severity, like any other
  finding.
- **A placeholder reaches the client on Run OFF** → the response de-mask missed a field (or, on
  the streamed path, the hold-back). A finding, not a retry.
- **The model paraphrased instead of echoing** → not necessarily a broken round-trip; some models
  reword despite the augmentation prompt. Retry with a more literal instruction before concluding
  anything.
- **CC-02 finds no `Person`** → check `ONNX NER detector loaded` really appeared. That is the
  trap this file now warns about twice for a reason.

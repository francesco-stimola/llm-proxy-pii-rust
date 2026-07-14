# Manual verification — does the whole structure hold end-to-end?

**E2E-INT-02** in `docs/TESTING.md`. This is deliberately a **manual** procedure, not
a `#[test]`: it compares what two separate runs *logged* and *returned to the
client*, against a **real** provider (Anthropic — the only one we hold credentials
for). No automated harness asserts on human-inspected trace logs; this replaces that
inspection with a repeatable checklist. For an automated (but still opt-in,
`#[ignore]`d) real-provider check, see `tests/anthropic_smoke.rs` (E2E-INT-01).

## What this proves that automated tests can't

The mock-upstream e2e tests (`tests/proxy_e2e.rs`) prove the proxy's own logic is
correct. They can't prove the **real** provider round-trip holds — a real model may
reformat a placeholder, refuse to echo it, or the account/auth wiring may be wrong
in a way a mock can't catch. Running the same prompt twice, once with the response
de-mask **skipped** and once normal, gives a direct, human-visible before/after: Run
A shows exactly what left the box and what the provider saw; Run B shows the client
still gets the real values back.

## Prerequisites

- A real `ANTHROPIC_API_KEY`.
- Build with logging enabled (default; no special feature needed for this check).

## Procedure

1. **Start the proxy for Run A** — response de-mask skipped, trace logging on:

   ```text
   set ANTHROPIC_API_KEY=sk-ant-...
   set UPSTREAM_PROVIDER=anthropic
   set UPSTREAM_BASE_URL=https://api.anthropic.com
   set PII_DEBUG_SKIP_DEMASK=1
   set RUST_LOG=llm_proxy_pii_rust=trace
   cargo run
   ```

2. Send a request carrying real PII, forwarding the header Anthropic's compat
   layer needs and picking a cheap model:

   ```text
   curl http://127.0.0.1:8080/v1/chat/completions ^
     -H "content-type: application/json" ^
     -H "anthropic-version: 2023-06-01" ^
     -d "{\"model\":\"claude-3-5-haiku-latest\",\"max_tokens\":64,\"messages\":[{\"role\":\"user\",\"content\":\"Reply with exactly this sentence and nothing else: contact jane.doe@example.com for details.\"}]}"
   ```

3. **Check Run A:**
   - The JSON reply's `choices[0].message.content` contains a placeholder like
     `[EMAIL_1]`, **not** `jane.doe@example.com` — proof the client-visible path is
     wired to the masked value (de-mask is off, so this is what the provider itself
     received and echoed).
   - The terminal's `trace!` line (`forwarding masked request body upstream`) shows
     the **same** placeholder in the outgoing body, and grepping the whole terminal
     output for `jane.doe@example.com` finds **nothing** — the never-log-raw-PII
     rule (DBG-02) holds on real data, not just the synthetic case
     `tests/log_safety.rs` pins.

4. **Restart for Run B** — same env, but drop `PII_DEBUG_SKIP_DEMASK`:

   ```text
   set PII_DEBUG_SKIP_DEMASK=
   cargo run
   ```

   Send the **same** request again.

5. **Check Run B:** `choices[0].message.content` now contains the real
   `jane.doe@example.com`, restored — while the trace log line still shows only the
   masked placeholder (request-side masking is identical in both runs; only the
   response de-mask differs).

## What "holds" means

Run A alone proves the request left masked. Run B alone proves the client got a
sensible reply. Neither alone proves they're the **same** round-trip. Together,
on the *same* input against the *same* real provider, they show: the value that
left masked (Run A) is the exact value the client gets restored (Run B) — the
full chain, not two independently-plausible halves.

## If it doesn't hold

- Run A shows the raw email in the outgoing body or the trace log → a masking gap;
  treat as a **leak**, not a perf/quality issue — file it the way any other finding
  is recorded (`docs/reviews/`), highest severity.
- Run B doesn't restore the placeholder → check the model actually echoed
  `[EMAIL_1]` verbatim (some models paraphrase despite the system-prompt
  instruction; retry with a more literal prompt before concluding the round-trip
  is broken) and that `PII_DEBUG_SKIP_DEMASK` was really unset.

-- CC-09 — the customer lookup the agent runs through the MCP SQL tool.
--
-- **PII-free by design.** The query names columns, never values, so reading this file
-- masks nothing — the PII rides in the *result*, which comes back as a `tool_result` and
-- is masked by the proxy before it reaches the provider. That is the whole point of CC-09.
--
-- Prerequisite: the synthetic table exists — run `cc09-setup.sql` once, out-of-band,
-- against a throwaway DB (a local SQLite file is enough). Never point this at a real table.
--
-- (History: this fixture used to be `SELECT 'bob@test.com'… FROM DUAL`, which put the PII
-- in the query *text* — so the agent reading it masked the literals *before* the query ran
-- and the tool_result path was never exercised. Fixed 2026-07-18; see docs/DEVLOG.md.)

SELECT * FROM cc09_customers;

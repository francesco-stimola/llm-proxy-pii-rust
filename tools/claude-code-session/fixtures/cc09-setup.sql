-- CC-09 setup — the tool_result masking test (the old proxy's TC-04).
--
-- Run this ONCE, OUT-OF-BAND (directly against a throwaway DB — a local SQLite file
-- is enough), NOT through the proxy. It creates a synthetic customers-shaped table so
-- the PII rides in the query *result*, not the query *text*.
--
-- The agent's actual ask is then the PII-FREE query in `customer-lookup.sql`:
--     SELECT * FROM cc09_customers;
-- whose result comes back as a `tool_result` — exactly the shape that must be masked
-- before it reaches the provider.
--
-- Why not `SELECT 'bob@test.com'… FROM DUAL` (the original TC-04): the PII would sit in
-- the query *text*, so the agent reading the file masks it *before* the query runs, and
-- the tool_result path is never exercised. Put the PII in a table, and the query text
-- has nothing to mask. (See docs/DEVLOG.md 2026-07-18 and docs/TESTING.md → CC-09.)
--
-- Every value is synthetic. Never point this at a real table.
--
-- Portable across SQLite / Oracle / Postgres (plain CREATE + INSERT). On Oracle drop the
-- `IF NOT EXISTS` and quote as needed.

CREATE TABLE IF NOT EXISTS cc09_customers (
  id     INTEGER PRIMARY KEY,
  email  TEXT,
  phone  TEXT,
  ssn    TEXT,
  card   TEXT,
  iban   TEXT,
  secret TEXT
);

INSERT INTO cc09_customers (email, phone, ssn, card, iban, secret) VALUES
  ('bob@test.com', '555-111-2222', '123-45-6789', '4111111111111111',
   'IT60X0542811101000000123456',
   'sk-ant-api01-test0000000000000000000000000000000000000000000000000');

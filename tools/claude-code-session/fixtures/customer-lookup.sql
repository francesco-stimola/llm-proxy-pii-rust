-- CC-09 fixture — the old proxy's TC-04, reproduced on real infrastructure.
--
-- Returns every structured category in one tabular result WITHOUT needing a schema
-- or any real data: it selects literals from DUAL. Run it through the MCP SQL tool
-- from a Claude Code session pointed at the proxy; the result comes back as a
-- `tool_result` content block, which is exactly the shape that must be masked
-- before it reaches the provider.
--
-- Every value is synthetic.

SELECT 'bob@test.com'                  AS email,
       '555-111-2222'                  AS phone,
       '123-45-6789'                   AS ssn,
       '4111111111111111'              AS card,
       'IT60X0542811101000000123456'   AS iban,
       'sk-ant-api01-test0000000000000000000000000000000000000000000000000' AS secret
  FROM DUAL;

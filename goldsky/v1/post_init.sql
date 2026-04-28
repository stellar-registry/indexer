-- Views that expose `resolved_channel` — the friendly channel name from
-- v1.registries when available, otherwise the raw emitter contract_id.
--
-- The v1 pipeline writes emitter contract_ids into the `channel` column
-- of v1.published_wasms and v1.registered_contracts (the `contract_id as
-- channel` projection in goldsky/v1/index.yaml). Callers of the API
-- pass friendly names like "root" or "soroswap", so every read needs to
-- translate contract_id → name via v1.registries. These views
-- encapsulate that translation so individual queries don't each repeat
-- the LEFT JOIN + COALESCE.

CREATE OR REPLACE VIEW v1.published_wasms_with_channel AS
SELECT
  w.*,
  r.registry_channel AS channel
FROM v1.published_wasms w
JOIN v1.registries r ON r.contract_id = w.emitter_contract_id;

CREATE OR REPLACE VIEW v1.registered_contracts_with_channel AS
SELECT
  c.*,
  r.registry_channel AS channel
FROM v1.registered_contracts c
JOIN v1.registries r ON r.contract_id = c.emitter_contract_id;

-- Scopes v1.contract_upgrades to upgrades of contracts we care about:
-- registries themselves, contracts deployed via a registry, and contracts
-- registered with one. The pipeline sinks every network upgrade event
-- because Goldsky only allows one dynamic_table_check consumer per dynamic
-- table; this view does the scoping in Postgres at query time.
CREATE OR REPLACE VIEW v1.contract_upgrades_scoped AS
SELECT u.*
FROM v1.contract_upgrades u
WHERE u.upgraded_contract_id IN (
  SELECT contract_id FROM v1.registries
  UNION
  SELECT contract_id FROM v1.deployed_contracts
  UNION
  SELECT contract_id FROM v1.registered_contracts
);

-- Views that expose `resolved_channel` — the friendly channel name from
-- v4_registries when available, otherwise the raw emitter contract_id.
--
-- The v4 pipeline writes emitter contract_ids into the `channel` column
-- of v4_published_wasms and v4_registered_contracts (the `contract_id as
-- channel` projection in registry-turbo-v4.yaml). Callers of the API
-- pass friendly names like "root" or "soroswap", so every read needs to
-- translate contract_id → name via v4_registries. These views
-- encapsulate that translation so individual queries don't each repeat
-- the LEFT JOIN + COALESCE.

CREATE OR REPLACE VIEW v4_published_wasms_with_channel AS
SELECT
  w.*,
  r.registry_channel AS channel
FROM public.v4_published_wasms w
JOIN public.v4_registries r ON r.contract_id = w.emitter_contract_id;

CREATE OR REPLACE VIEW v4_registered_contracts_with_channel AS
SELECT
  c.*,
  r.registry_channel AS channel
FROM public.v4_registered_contracts c
JOIN public.v4_registries r ON r.contract_id = c.emitter_contract_id;

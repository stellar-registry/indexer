-- v1 post-init views and helpers.
--
-- search_path makes every unqualified name in this file resolve to the
-- v1 schema — for CREATE VIEW / CREATE FUNCTION (where the object lands)
-- and for FROM/JOIN/function-call references (which Postgres rewrites
-- against search_path at view-creation time and stores as schema-
-- qualified, so the views remain stable when callers later query them
-- under a different search_path).
--
-- Cross-schema references stay explicit: archive.* lives in its own
-- schema (see goldsky/archive/index.yaml) and must not be hidden by
-- search_path resolution.

SET search_path TO v1;

-- extension is enabled on v1 schema
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- create trigram index on wasm_name
CREATE INDEX wasm_name_trgm_idx_published_wasms ON published_wasms USING GIN (wasm_name gin_trgm_ops);

-- Channel views: translate emitter_contract_id → friendly channel name
-- via the registries table.

CREATE OR REPLACE VIEW published_wasms_with_channel AS
SELECT
  w.*,
  r.registry_channel AS channel
FROM published_wasms w
JOIN registries r ON r.contract_id = w.emitter_contract_id;

CREATE OR REPLACE VIEW registered_contracts_with_channel AS
SELECT
  c.*,
  r.registry_channel AS channel
FROM registered_contracts c
JOIN registries r ON r.contract_id = c.emitter_contract_id;

-- Absolute latest publish per wasm_name, across all channels. Tie-break
-- on wasm_version (semver text) when two publishes land in the same
-- ledger.

CREATE OR REPLACE VIEW latest_published_wasms AS
SELECT *
FROM (
  SELECT
    w.*,
    ROW_NUMBER() OVER (
      PARTITION BY wasm_name
      ORDER BY ledger_sequence DESC, wasm_version DESC
    ) AS rn
  FROM published_wasms_with_channel w
) sub
WHERE rn = 1;

-- One row per (uploaded wasm × publish event); bare uploads kept with
-- NULL publish-side columns. Read-side view over the upload-addressable
-- archive joined to publish metadata — same shape as v1.wasm_versions.

CREATE OR REPLACE VIEW wasm_versions AS
SELECT
  u.wasm_hash,
  u.id                         AS upload_id,
  u.ledger_sequence            AS uploaded_ledger_sequence,
  u.closed_at                  AS uploaded_at,
  u.transaction_hash           AS upload_transaction_hash,
  p.id                         AS publish_id,
  p.transaction_hash           AS publish_transaction_hash,
  p.ledger_sequence            AS publish_ledger_sequence,
  p.created_at                 AS published_at,
  p.author,
  p.wasm_name,
  p.wasm_version,
  p.emitter_contract_id        AS publish_registry_contract_id
FROM archive.uploads u
LEFT JOIN published_wasms p ON p.wasm_hash = u.wasm_hash;

-- Contract version history: the contract's actual deploy wasm
-- (kind='initial') plus each subsequent executable_update event
-- (kind='upgrade'), ordered chronologically within each contract.
--
-- Initial rows come from archive.deploys, which is now sourced from
-- the ledger_entries dataset (ContractData entries with
-- change_type='created' and key=LedgerKeyContractInstance). This
-- captures both top-level CreateContract operations AND factory
-- sub-invocations, so every wasm-backed contract has its real deploy
-- row regardless of how it was created.
--
-- Why not registered_contracts.wasm_hash? Because that records the
-- wasm a contract was running at REGISTRATION TIME, not at deploy
-- time. A contract deployed via a non-registry path and only later
-- registered would lose its actual initial wasm history.
-- archive.deploys captures the on-chain truth.
--
-- Upgrade rows come from archive.upgrades (host-emitted
-- executable_update system events). Both archive.deploys.contract_id
-- and archive.upgrades.upgraded_contract_id are StrKey form, so the
-- UNION ALL is direct with no encoding bridge.
--
-- wasm_name / wasm_version / wasm_channel come from a DISTINCT ON join
-- against published_wasms (and through it to registries). All three
-- are NULL when the wasm was uploaded but never published.

DROP VIEW IF EXISTS contracts_enriched;
DROP VIEW IF EXISTS versions;

CREATE VIEW versions AS
SELECT
  t.contract_id,
  ROW_NUMBER() OVER (
    PARTITION BY t.contract_id
    ORDER BY t.ledger_sequence, t.source_id
  ) - 1 AS version_index,
  t.kind,
  t.wasm_hash,
  p.wasm_name,
  p.wasm_version,
  r.registry_channel AS wasm_channel,
  t.source_id,
  t.ledger_sequence,
  t.transaction_hash,
  t.created_at
FROM (
  SELECT
    d.contract_id,
    'initial'::text   AS kind,
    d.wasm_hash       AS wasm_hash,
    d.id              AS source_id,
    d.ledger_sequence AS ledger_sequence,
    d.transaction_hash,
    d.closed_at       AS created_at
  FROM archive.deploys d
  WHERE d.wasm_hash IS NOT NULL
  UNION ALL
  SELECT
    u.upgraded_contract_id,
    'upgrade'::text,
    u.new_wasm_hash,
    u.id,
    u.ledger_sequence,
    u.transaction_hash,
    u.created_at
  FROM archive.upgrades u
) t
LEFT JOIN (
  SELECT DISTINCT ON (wasm_hash) wasm_hash, wasm_name, wasm_version, emitter_contract_id
  FROM published_wasms
  ORDER BY wasm_hash, ledger_sequence DESC
) p ON p.wasm_hash = t.wasm_hash
LEFT JOIN registries r ON r.contract_id = p.emitter_contract_id;

-- One row per known contract — the JOIN of registered_contracts (the
-- registry's `register` events, which give the contract a name) and
-- deployed_contracts (the registry's `deploy` events, which record who
-- triggered the deploy). Anchored on registered_contracts because the
-- API addresses contracts by name; deployed-only contracts (deploy
-- event but no register event) are out of scope.

DROP VIEW IF EXISTS contracts;

CREATE VIEW contracts AS
SELECT
  registered.id,
  registered.transaction_hash,
  registered.ledger_sequence,
  registered.created_at,
  registered.contract_id,
  registered.contract_name,
  registered.channel,
  registered.sac,
  registered.wasm_hash,
  deployed.deployer,
  deployed.registry_contract_id
FROM registered_contracts_with_channel registered
LEFT JOIN (
  SELECT DISTINCT ON (contract_id) contract_id, deployer, registry_contract_id
  FROM deployed_contracts
  ORDER BY contract_id, ledger_sequence DESC
) deployed ON deployed.contract_id = registered.contract_id;

-- Contracts decorated with the metadata of the wasm they're CURRENTLY
-- running — the latest version row from v1.versions per contract.
-- wasm_name / wasm_version / wasm_channel reflect the latest upgrade
-- (or the initial deploy if the contract has never been upgraded).
-- Backs /v1/contracts list and /v1/contracts/{name} detail endpoints.

CREATE VIEW contracts_enriched AS
SELECT
  c.id,
  c.transaction_hash,
  c.ledger_sequence,
  c.created_at,
  c.contract_id,
  c.contract_name,
  c.channel,
  c.sac,
  c.deployer,
  latest.wasm_version,
  latest.wasm_name,
  latest.wasm_channel
FROM contracts c
LEFT JOIN (
  SELECT DISTINCT ON (contract_id)
    contract_id, wasm_name, wasm_version, wasm_channel
  FROM versions
  ORDER BY contract_id, version_index DESC
) latest ON latest.contract_id = c.contract_id;

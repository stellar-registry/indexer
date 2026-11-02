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
-- schema (see goldsky/archive/index.template.yaml) and must not be hidden by
-- search_path resolution.

SET search_path TO v1;

-- extension is enabled on v1 schema
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- create trigram index on wasm_name
CREATE INDEX IF NOT EXISTS wasm_name_trgm_idx_published_wasms ON published_wasms USING GIN (wasm_name gin_trgm_ops);

-- create trigram index on contract_name
CREATE INDEX IF NOT EXISTS contract_name_trgm_idx_registered_contracts ON registered_contracts USING GIN (contract_name gin_trgm_ops);

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

-- The change-feed views below depend on versions, so they go first.
DROP VIEW IF EXISTS changes;
DROP VIEW IF EXISTS contract_registrations;
DROP VIEW IF EXISTS contract_name_events;
DROP VIEW IF EXISTS contract_name_history;
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

-- used to store details extracted from wasm binaries
-- of published wasms
CREATE TABLE IF NOT EXISTS extracted_wasm_details (
  wasm_hash text NOT NULL PRIMARY KEY,
  contract_spec JSONB,
  contract_meta JSONB,
  ledger_sequence int8
);

-- Stellar Expert's "verified build" attestation, fetched once per
-- contract_id (via verify_build_webhook, triggered off `register` events)
-- rather than on every API request. NULL status/repository/etc. means the
-- contract was checked and Stellar Expert has no verification for it yet
-- — checked_at still distinguishes "checked, not verified" from "not
-- checked yet" (no row).
-- commit_hash, not "commit" — COMMIT is a SQL keyword, and this is also
-- clearer about what it holds (a git commit SHA, not a DB transaction).
CREATE TABLE IF NOT EXISTS contract_verifications (
  contract_id text NOT NULL PRIMARY KEY,
  status text,
  repository text,
  commit_hash text,
  package text,
  path text,
  checked_at timestamp,
  ledger_sequence int8
);

-- ── /v1/changes feed ────────────────────────────────────────────────
--
-- Registry entries are keyed by name, and names move: `rename` moves an
-- entry to a new name, and `update_address` / `update_owner` refer to
-- the entry by whatever name it has at that point. A registration
-- (its `register` event id) is the stable identity; these views fold
-- the name-keyed events onto it.

-- Each registration's name over time: one row per name it has held,
-- [since, until) in (ledger_sequence, id) order. A rename applies to
-- the registration holding old_name when the rename happened, so the
-- chain follows the first rename of the current name after taking it.
CREATE VIEW contract_name_history AS
WITH RECURSIVE chain AS (
  SELECT
    r.id AS registration_id,
    r.emitter_contract_id,
    r.contract_name AS name,
    r.ledger_sequence,
    r.id AS since_id,
    0 AS step
  FROM registered_contracts r
  UNION ALL
  SELECT
    c.registration_id,
    c.emitter_contract_id,
    n.new_name,
    n.ledger_sequence,
    n.id,
    c.step + 1
  FROM chain c
  CROSS JOIN LATERAL (
    SELECT x.new_name, x.ledger_sequence, x.id
    FROM rename x
    WHERE x.emitter_contract_id = c.emitter_contract_id
      AND x.old_name = c.name
      AND (x.ledger_sequence, x.id) > (c.ledger_sequence, c.since_id)
    ORDER BY x.ledger_sequence, x.id
    LIMIT 1
  ) n
)
SELECT
  registration_id,
  emitter_contract_id,
  name,
  ledger_sequence AS since_ledger,
  since_id,
  LEAD(ledger_sequence) OVER w AS until_ledger,
  LEAD(since_id) OVER w AS until_id
FROM chain
WINDOW w AS (PARTITION BY registration_id ORDER BY step);

-- update_address / update_owner events, attributed to the registration
-- that held contract_name when the event happened.
CREATE VIEW contract_name_events AS
SELECT
  e.id,
  e.ledger_sequence,
  e.created_at,
  e.transaction_hash,
  e.emitter_contract_id,
  e.type,
  e.contract_name,
  e.value,
  h.registration_id
FROM (
  SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
         'update_address'::text AS type, contract_name, new_address AS value
  FROM update_address
  UNION ALL
  SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
         'update_owner'::text, contract_name, new_owner
  FROM update_owner
) e
LEFT JOIN contract_name_history h
  ON h.emitter_contract_id = e.emitter_contract_id
 AND h.name = e.contract_name
 AND (h.since_ledger, h.since_id) < (e.ledger_sequence, e.id)
 AND (h.until_id IS NULL OR (e.ledger_sequence, e.id) < (h.until_ledger, h.until_id));

-- Current state of every registration. owner is NULL until an
-- update_owner event: `register` doesn't emit the initial owner.
CREATE VIEW contract_registrations AS
SELECT
  r.id AS registration_id,
  ch.registry_channel AS channel,
  cur.name AS contract_name,
  COALESCE(addr.value, r.contract_id) AS contract_id,
  own.value AS owner,
  r.sac,
  dep.deployer,
  latest.wasm_hash,
  latest.wasm_name,
  latest.wasm_version,
  latest.wasm_channel
FROM registered_contracts r
JOIN registries ch ON ch.contract_id = r.emitter_contract_id
JOIN contract_name_history cur
  ON cur.registration_id = r.id AND cur.until_id IS NULL
LEFT JOIN LATERAL (
  SELECT value FROM contract_name_events
  WHERE registration_id = r.id AND type = 'update_address'
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) addr ON true
LEFT JOIN LATERAL (
  SELECT value FROM contract_name_events
  WHERE registration_id = r.id AND type = 'update_owner'
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) own ON true
LEFT JOIN LATERAL (
  SELECT deployer FROM deployed_contracts
  WHERE contract_id = r.contract_id
  ORDER BY ledger_sequence DESC
  LIMIT 1
) dep ON true
-- One pass over versions for just the addresses registrations point at;
-- a per-row LATERAL here rescans archive.deploys for every registration.
LEFT JOIN (
  SELECT DISTINCT ON (contract_id)
    contract_id, wasm_hash, wasm_name, wasm_version, wasm_channel
  FROM versions
  WHERE contract_id IN (
    SELECT contract_id FROM registered_contracts
    UNION
    SELECT new_address FROM update_address
  )
  ORDER BY contract_id, version_index DESC
) latest ON latest.contract_id = COALESCE(addr.value, r.contract_id);

-- One row per registry event, in feed order (ledger_sequence, id).
-- `event` is the event's own payload; the API joins the current state
-- of the affected Wasm (by wasm_hash) or contract (by registration_id).
CREATE VIEW changes AS
SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
       'publish'::text AS type, wasm_hash, NULL::text AS registration_id,
       jsonb_build_object(
         'wasm_name', wasm_name, 'wasm_version', wasm_version,
         'wasm_hash', wasm_hash, 'author', author
       ) AS event
FROM published_wasms
UNION ALL
SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
       'register', NULL, id,
       jsonb_build_object(
         'contract_name', contract_name, 'contract_id', contract_id,
         'sac', sac, 'wasm_hash', wasm_hash
       )
FROM registered_contracts
UNION ALL
-- A registry deploy also registers the name in the same transaction.
SELECT d.id, d.ledger_sequence, d.created_at, d.transaction_hash, d.emitter_contract_id,
       'deploy', NULL, reg.id,
       jsonb_build_object(
         'wasm_name', d.wasm_name, 'wasm_version', d.wasm_version,
         'deployer', d.deployer, 'contract_id', d.contract_id
       )
FROM deployed_contracts d
LEFT JOIN LATERAL (
  SELECT r.id FROM registered_contracts r
  WHERE r.contract_id = d.contract_id
  ORDER BY (r.transaction_hash = d.transaction_hash) DESC, r.ledger_sequence DESC
  LIMIT 1
) reg ON true
UNION ALL
SELECT x.id, x.ledger_sequence, x.created_at, x.transaction_hash, x.emitter_contract_id,
       'rename', NULL, h.registration_id,
       jsonb_build_object('old_name', x.old_name, 'new_name', x.new_name)
FROM rename x
LEFT JOIN contract_name_history h ON h.since_id = x.id
UNION ALL
SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
       type, NULL, registration_id,
       jsonb_build_object(
         'contract_name', contract_name,
         CASE type WHEN 'update_address' THEN 'new_address' ELSE 'new_owner' END, value
       )
FROM contract_name_events;

-- ── Accounts ────────────────────────────────────────────────────────
--
-- Named accounts (G-addresses and smart accounts) live in their own
-- namespace, separate from contracts, with the same name-keyed lifecycle:
-- `rename_account` moves an entry, and `update_acct_addr` /
-- `update_acct_owner` / `sec_flag_acct` refer to it by its name at that
-- point. As for contracts, the `register_account` event id is the stable
-- identity.

DROP VIEW IF EXISTS pending_account_preauths;
DROP VIEW IF EXISTS account_namings;
DROP VIEW IF EXISTS accounts;
DROP VIEW IF EXISTS account_name_events;
DROP VIEW IF EXISTS account_name_history;

CREATE INDEX IF NOT EXISTS account_name_trgm_idx_registered_accounts ON registered_accounts USING GIN (account_name gin_trgm_ops);
CREATE INDEX IF NOT EXISTS registered_accounts_account_id_idx ON registered_accounts (account_id);
CREATE INDEX IF NOT EXISTS update_acct_addr_new_address_idx ON update_acct_addr (new_address);

-- Each account registration's name over time, as contract_name_history.
CREATE VIEW account_name_history AS
WITH RECURSIVE chain AS (
  SELECT
    r.id AS registration_id,
    r.emitter_contract_id,
    r.account_name AS name,
    r.ledger_sequence,
    r.id AS since_id,
    r.created_at,
    0 AS step
  FROM registered_accounts r
  UNION ALL
  SELECT
    c.registration_id,
    c.emitter_contract_id,
    n.new_name,
    n.ledger_sequence,
    n.id,
    n.created_at,
    c.step + 1
  FROM chain c
  CROSS JOIN LATERAL (
    SELECT x.new_name, x.ledger_sequence, x.id, x.created_at
    FROM rename_account x
    WHERE x.emitter_contract_id = c.emitter_contract_id
      AND x.old_name = c.name
      AND (x.ledger_sequence, x.id) > (c.ledger_sequence, c.since_id)
    ORDER BY x.ledger_sequence, x.id
    LIMIT 1
  ) n
)
SELECT
  registration_id,
  emitter_contract_id,
  name,
  ledger_sequence AS since_ledger,
  since_id,
  created_at AS since_created_at,
  LEAD(ledger_sequence) OVER w AS until_ledger,
  LEAD(since_id) OVER w AS until_id
FROM chain
WINDOW w AS (PARTITION BY registration_id ORDER BY step);

-- update_acct_addr / update_acct_owner / sec_flag_acct events, attributed
-- to the registration that held account_name when the event happened.
CREATE VIEW account_name_events AS
SELECT
  e.id,
  e.ledger_sequence,
  e.created_at,
  e.transaction_hash,
  e.emitter_contract_id,
  e.type,
  e.account_name,
  e.value,
  e.operator,
  h.registration_id
FROM (
  SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
         'update_address'::text AS type, account_name, new_address AS value, operator
  FROM update_acct_addr
  UNION ALL
  SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
         'update_owner'::text, account_name, new_owner, operator
  FROM update_acct_owner
  UNION ALL
  SELECT id, ledger_sequence, created_at, transaction_hash, emitter_contract_id,
         'flag'::text, account_name, flagged::text, operator
  FROM sec_flag_acct
) e
LEFT JOIN account_name_history h
  ON h.emitter_contract_id = e.emitter_contract_id
 AND h.name = e.account_name
 AND (h.since_ledger, h.since_id) < (e.ledger_sequence, e.id)
 AND (h.until_id IS NULL OR (e.ledger_sequence, e.id) < (h.until_ledger, h.until_id));

-- Current state of every account registration. owner is NULL only for
-- registrations from registry versions before 0.8.0 that were never
-- given a new owner (their register_account event has no owner).
CREATE VIEW accounts AS
SELECT
  r.id AS registration_id,
  r.id,
  r.transaction_hash,
  r.ledger_sequence,
  r.emitter_contract_id,
  ch.registry_channel AS channel,
  cur.name AS account_name,
  COALESCE(addr.value, r.account_id) AS address,
  COALESCE(own.value, r.owner) AS owner,
  COALESCE(flag.value::boolean, false) AS flagged,
  r.created_at,
  GREATEST(r.created_at, renamed.at, changed.at) AS updated_at
FROM registered_accounts r
JOIN registries ch ON ch.contract_id = r.emitter_contract_id
JOIN account_name_history cur
  ON cur.registration_id = r.id AND cur.until_id IS NULL
LEFT JOIN LATERAL (
  SELECT value FROM account_name_events
  WHERE registration_id = r.id AND type = 'update_address'
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) addr ON true
LEFT JOIN LATERAL (
  SELECT value FROM account_name_events
  WHERE registration_id = r.id AND type = 'update_owner'
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) own ON true
LEFT JOIN LATERAL (
  SELECT value FROM account_name_events
  WHERE registration_id = r.id AND type = 'flag'
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) flag ON true
LEFT JOIN LATERAL (
  SELECT max(since_created_at) AS at FROM account_name_history
  WHERE registration_id = r.id AND since_id <> r.id
) renamed ON true
LEFT JOIN LATERAL (
  SELECT max(created_at) AS at FROM account_name_events
  WHERE registration_id = r.id
) changed ON true;

-- Every event that put an address under a name (register, address
-- update, rename), with the registration's address and owner at that
-- point. A matching preauthorization is consumed by any of these.
CREATE VIEW account_namings AS
SELECT
  n.registration_id,
  n.emitter_contract_id,
  n.name,
  n.ledger_sequence,
  n.event_id,
  COALESCE(addr.value, r.account_id) AS address,
  COALESCE(own.value, r.owner) AS owner
FROM (
  SELECT id AS registration_id, emitter_contract_id, account_name AS name,
         ledger_sequence, id AS event_id
  FROM registered_accounts
  UNION ALL
  SELECT registration_id, emitter_contract_id, account_name, ledger_sequence, id
  FROM account_name_events
  WHERE type = 'update_address'
  UNION ALL
  SELECT registration_id, emitter_contract_id, name, since_ledger, since_id
  FROM account_name_history
  WHERE since_id <> registration_id
) n
JOIN registered_accounts r ON r.id = n.registration_id
LEFT JOIN LATERAL (
  SELECT value FROM account_name_events
  WHERE registration_id = n.registration_id AND type = 'update_address'
    AND (ledger_sequence, id) <= (n.ledger_sequence, n.event_id)
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) addr ON true
LEFT JOIN LATERAL (
  SELECT value FROM account_name_events
  WHERE registration_id = n.registration_id AND type = 'update_owner'
    AND (ledger_sequence, id) < (n.ledger_sequence, n.event_id)
  ORDER BY ledger_sequence DESC, id DESC
  LIMIT 1
) own ON true;

-- Preauthorizations still waiting to be used: the latest preauth_acct per
-- (registry, name, address), unless a revoke or a matching naming (same
-- name, address and owner) came after it. Expiry isn't evented: an entry
-- lapses after the network's max temporary-entry TTL counted from
-- ledger_sequence (~3.1M ledgers on testnet and mainnet), so consumers
-- should treat old rows as possibly expired.
CREATE VIEW pending_account_preauths AS
SELECT
  p.id,
  p.transaction_hash,
  p.ledger_sequence,
  p.created_at,
  p.emitter_contract_id,
  ch.registry_channel AS channel,
  p.account_name,
  p.account_id AS address,
  p.owner
FROM (
  SELECT DISTINCT ON (emitter_contract_id, account_name, account_id) *
  FROM preauth_acct
  ORDER BY emitter_contract_id, account_name, account_id, ledger_sequence DESC, id DESC
) p
JOIN registries ch ON ch.contract_id = p.emitter_contract_id
WHERE NOT EXISTS (
  SELECT 1 FROM revoke_preauth_acct x
  WHERE x.emitter_contract_id = p.emitter_contract_id
    AND x.account_name = p.account_name
    AND x.account_id = p.account_id
    AND (x.ledger_sequence, x.id) > (p.ledger_sequence, p.id)
)
AND NOT EXISTS (
  SELECT 1 FROM account_namings n
  WHERE n.emitter_contract_id = p.emitter_contract_id
    AND n.name = p.account_name
    AND n.address = p.account_id
    AND n.owner = p.owner
    AND (n.ledger_sequence, n.event_id) > (p.ledger_sequence, p.id)
);

-- DDL for the v4 Goldsky Turbo sink tables.
--
-- Goldsky auto-creates these tables on first pipeline deploy based on the
-- projection of each transform_3_* block in registry-turbo-v4.yaml. We mirror
-- those shapes here so that:
--   1. the repo has a single source of truth for the v4 schema,
--   2. the integration test harness can stand up an identical Postgres from scratch,
--   3. indexes/constraints can be added here without waiting for a pipeline deploy.
--
-- All statements use IF NOT EXISTS so this file is safe to apply against an
-- already-populated Goldsky database without disturbing existing rows.

CREATE TABLE IF NOT EXISTS v4_deployed_contracts (
  id               TEXT    NOT NULL PRIMARY KEY,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  channel          TEXT,
  wasm_name        TEXT,
  wasm_version     TEXT,
  deployer         TEXT,
  contract_id      TEXT
);

CREATE TABLE IF NOT EXISTS v4_published_wasms (
  id               TEXT    NOT NULL PRIMARY KEY,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  channel          TEXT,
  author           TEXT,
  wasm_version     TEXT,
  wasm_hash        TEXT,
  wasm_name        TEXT
);

CREATE TABLE IF NOT EXISTS v4_registered_contracts (
  id               TEXT    NOT NULL PRIMARY KEY,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  channel          TEXT,
  contract_name    TEXT,
  contract_id      TEXT,
  sac              BOOLEAN,
  wasm_hash        TEXT
);

CREATE TABLE IF NOT EXISTS v4_rename (
  id               TEXT    NOT NULL PRIMARY KEY,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  channel          TEXT,
  old_name         TEXT,
  new_name         TEXT
);

CREATE TABLE IF NOT EXISTS v4_update_address (
  id               TEXT    NOT NULL PRIMARY KEY,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  channel          TEXT,
  contract_name    TEXT,
  new_address      TEXT
);

CREATE TABLE IF NOT EXISTS v4_update_owner (
  id               TEXT    NOT NULL PRIMARY KEY,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  channel          TEXT,
  contract_name    TEXT,
  new_owner        TEXT
);

-- raw_events_backup is a passthrough of transform_1_events (which is SELECT *
-- from the goldsky_source dataset). Columns mirror the stellar_testnet.events
-- dataset schema as consumed elsewhere in this repo (see lib/periodic-lambda.ts).
CREATE TABLE IF NOT EXISTS v4_raw_events_backup (
  id                           TEXT    NOT NULL PRIMARY KEY,
  type                         TEXT,
  topics                       TEXT,
  data                         TEXT,
  transaction_hash             TEXT,
  in_successful_contract_call  BOOLEAN,
  transaction_successful       BOOLEAN,
  ledger_sequence              BIGINT,
  ledger_closed_at             TIMESTAMP,
  contract_id                  TEXT
);

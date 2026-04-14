-- Registry lookup table for the v4 pipeline.
--
-- Populated by registry-turbo-v4's subregistry_events_pg sink, which extracts
-- `sub_reg` events emitted by the root registry
-- (CDVDJX2HXCDRWUA7ISE2X3W4S5A6DEWLSDHBK3I5WH3JLWKF2IT4HA2P on testnet).
-- Each row maps a sub-registry contract_id to its human-readable channel name.
-- Upserts on contract_id so re-announcing a sub-registry updates its channel.

CREATE TABLE IF NOT EXISTS registries (
  contract_id      TEXT    NOT NULL PRIMARY KEY,
  channel          TEXT    NOT NULL,
  id               TEXT    NOT NULL,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL
);

CREATE INDEX IF NOT EXISTS registries_channel_idx ON registries (channel);

-- Registry lookup table for the v4 pipeline.
--
-- Populated by registry-turbo-v4's subregistry_events_pg sink, which extracts
-- `sub_reg` events emitted by the root registry
-- (CBNBQND6EMYTTRTCUWUJ3VIKF7RUUISK5T4GAKTXRVIQRHGP4XQY4ID7 on testnet).
-- Each row maps a sub-registry contract_id to its human-readable channel name.
-- Upserts on contract_id so re-announcing a sub-registry updates its channel.

CREATE TABLE IF NOT EXISTS v4_registries (
  contract_id      TEXT    NOT NULL PRIMARY KEY,
  channel          TEXT    NOT NULL,
  id               TEXT    NOT NULL,
  transaction_hash TEXT    NOT NULL,
  ledger_sequence  BIGINT  NOT NULL,
  created_at       TIMESTAMP NOT NULL
);

CREATE INDEX IF NOT EXISTS v4_registries_channel_idx ON v4_registries (channel);

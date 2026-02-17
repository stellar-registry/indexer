-- <Current version - 2> is dropped
DROP table IF EXISTS deploys_3;
DROP TABLE IF EXISTS publishes_3;

-- <Current version - 1> is a backup
-- CREATE TABLE deploys_4(
--   id TEXT not null PRIMARY KEY,
--   transaction_hash TEXT not null,
--   ledger_sequence bigint not null,
--   created_at timestamp not null,
--   contract_id text,
--   contract_name text,
--   deployer text,
--   version text,
--   wasm_name text
-- )

-- Current version
CREATE TABLE IF NOT EXISTS deploys_5(
  id TEXT not null PRIMARY KEY,
  transaction_hash TEXT not null,
  ledger_sequence bigint not null,
  created_at timestamp not null,
  contract_id text,
  contract_name text,
  deployer text,
  version text,
  wasm_name text
);

CREATE TABLE IF NOT EXISTS publishes_5(
  id TEXT not null PRIMARY KEY,
  transaction_hash TEXT not null,
  ledger_sequence bigint not null,
  created_at timestamp not null,
  author text,
  version text,
  wasm_name text,
  wasm_hash text
);

-- Minimal stand-ins for the tables the Goldsky sinks create, so
-- goldsky/v1/post_init.sql can be applied to a scratch database in tests.
CREATE SCHEMA IF NOT EXISTS v1;
CREATE SCHEMA IF NOT EXISTS archive;

CREATE TABLE v1.registries (id text, transaction_hash text, ledger_sequence bigint, created_at timestamp, contract_id text PRIMARY KEY, registry_channel text);
CREATE TABLE v1.published_wasms (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, author text, wasm_version text, wasm_hash text, wasm_name text);
CREATE TABLE v1.registered_contracts (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, contract_name text, contract_id text, sac boolean, wasm_hash text);
CREATE TABLE v1.deployed_contracts (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, wasm_name text, wasm_version text, deployer text, contract_id text, registry_contract_id text);
CREATE TABLE v1.rename (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, old_name text, new_name text);
CREATE TABLE v1.update_address (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, contract_name text, new_address text);
CREATE TABLE v1.update_owner (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, contract_name text, new_owner text);
CREATE TABLE v1.index_height (id text PRIMARY KEY, ledger_sequence bigint, closed_at timestamp);

CREATE TABLE archive.uploads (id text PRIMARY KEY, ledger_sequence bigint, closed_at timestamp, transaction_hash text, wasm_hash text);
CREATE TABLE archive.deploys (id text PRIMARY KEY, ledger_sequence bigint, closed_at timestamp, transaction_hash text, contract_id text, wasm_hash text);
CREATE TABLE archive.upgrades (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, upgraded_contract_id text, old_executable_kind text, old_wasm_hash text, new_executable_kind text, new_wasm_hash text);

CREATE TABLE v1.registered_accounts (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, account_name text, account_id text, owner text);
CREATE TABLE v1.update_acct_owner (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, account_name text, new_owner text, operator text);
CREATE TABLE v1.update_acct_addr (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, account_name text, new_address text, operator text);
CREATE TABLE v1.rename_account (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, old_name text, new_name text, operator text);
CREATE TABLE v1.sec_flag_acct (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, account_name text, flagged boolean, operator text);
CREATE TABLE v1.preauth_acct (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, account_name text, account_id text, owner text);
CREATE TABLE v1.revoke_preauth_acct (id text PRIMARY KEY, transaction_hash text, ledger_sequence bigint, created_at timestamp, emitter_contract_id text, account_name text, account_id text);

-- archive post-init: indexes only. Support v1's LATERAL per-contract
-- lookups (see goldsky/v1/post_init.sql). CONCURRENTLY since these
-- tables span the whole network, not just this registry; needs no
-- explicit transaction, which plain `psql -f` doesn't open anyway.

SET search_path TO archive;

CREATE INDEX CONCURRENTLY IF NOT EXISTS deploys_contract_id_idx ON deploys (contract_id, ledger_sequence DESC);
CREATE INDEX CONCURRENTLY IF NOT EXISTS upgrades_upgraded_contract_id_idx ON upgrades (upgraded_contract_id, ledger_sequence DESC);

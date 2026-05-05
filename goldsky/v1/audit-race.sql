-- Audit for events dropped by the `dynamic_table_check` race in the
-- v1 Goldsky pipeline.
--
-- Why this exists
-- ---------------
-- `transform_3_events_with_name` in goldsky/v1/index.yaml filters the
-- event stream through `dynamic_table_check('registries_dynamic_table',
-- emitter_contract_id)`. The dynamic table is populated by a sibling
-- transform (`transform_2_subregistry_events`) whose writes land in a
-- Postgres-backed entity (`v1.registries_dynamic_table`). The check
-- runs on every event going through transform_3 and reads from the
-- same Postgres entity.
--
-- These two paths are not synchronized. When an event's emitter
-- contract_id was `sub_reg`'d only a few ledgers earlier, the Postgres
-- write from transform_2 may not have committed by the time the check
-- reads, so the event gets filtered out even though it legitimately
-- belongs. Observed race window: at least 10 s (2 ledgers) — safe at
-- ~70 s (14 ledgers).
--
-- This query finds events that should have passed the filter but
-- didn't, by comparing transform_1's sink (`v1.raw_events_backup`,
-- which is upstream of the race-prone filter) against the downstream
-- sinks. See also: goldsky/scripts/refresh.sh, which recovers lost
-- events by clearing pipeline state and replaying against the
-- already-populated Postgres dynamic table.
--
-- How it works
-- ------------
--   1. `raw_events_backup` is the ground truth — transform_1 applies
--      only the event-name filter (no `dynamic_table_check`), so every
--      candidate event lands here.
--   2. Join to `registries` by emitter contract_id to keep only events
--      whose emitter is a known registry (i.e. events that *should*
--      pass the transform_4 filter).
--   3. Left-join each event against its per-event sink by id. Rows
--      where every sink-side id is NULL are events that were dropped.
--
-- Caveats
-- -------
--   - Sinks with `primary_key: id` (deploy, publish, register, rename,
--     update_address, update_owner) have exactly one row per raw row.
--     The id-based left join is reliable for those.
--   - `registries` has `primary_key: contract_id`, so if the same
--     contract_id is `sub_reg`'d more than once, only the latest row
--     survives the upsert. The older raw sub_reg events will show up
--     here as false positives. If that situation arises, treat sub_reg
--     drops skeptically and count distinct contract_ids instead.
--   - Only flags drops for emitters eventually present in `registries`.
--     An event dropped for an emitter that was never successfully
--     `sub_reg`'d won't appear — that's correct behavior.
--
-- Usage
-- -----
--   psql "$DATABASE_URL" -f goldsky/v1/audit-race.sql
--
-- A result set of zero rows means no race-dropped events were found.

SET search_path TO v1;

WITH raw AS (
  SELECT
    r.id,
    r.transaction_hash,
    r.ledger_sequence,
    r.emitter_contract_id,
    r.event_name
  FROM raw_events_backup r
  JOIN registries g ON g.contract_id = r.emitter_contract_id
)
SELECT
  raw.event_name,
  raw.emitter_contract_id,
  raw.ledger_sequence,
  raw.transaction_hash,
  raw.id
FROM raw
LEFT JOIN deployed_contracts    d ON d.id = raw.id AND raw.event_name = 'deploy'
LEFT JOIN published_wasms       p ON p.id = raw.id AND raw.event_name = 'publish'
LEFT JOIN registered_contracts  c ON c.id = raw.id AND raw.event_name = 'register'
LEFT JOIN rename                n ON n.id = raw.id AND raw.event_name = 'rename'
LEFT JOIN update_address        a ON a.id = raw.id AND raw.event_name = 'update_address'
LEFT JOIN update_owner          o ON o.id = raw.id AND raw.event_name = 'update_owner'
LEFT JOIN registries            s ON s.id = raw.id AND raw.event_name = 'sub_reg'
WHERE raw.event_name IN (
    'deploy', 'publish', 'register', 'rename',
    'update_address', 'update_owner', 'sub_reg'
  )
  AND d.id IS NULL AND p.id IS NULL AND c.id IS NULL
  AND n.id IS NULL AND a.id IS NULL AND o.id IS NULL
  AND s.id IS NULL
ORDER BY raw.ledger_sequence, raw.event_name;

# Registry indexer
This project implements functionality to track activity in the registry contract.
A Goldsky pipeline filters on-chain events and pushes parsed records into
Postgres; `fly-app/` (an Actix-web service deployed on Fly.io) reads from
Postgres and serves the registry API to clients.

## Per-network configuration (testnet & mainnet)
Goldsky pipeline definitions are checked in as templates
(`goldsky/v1/index.template.yaml`, `goldsky/archive/index.template.yaml`)
with `${VAR}` placeholders. Per-network values (dataset prefix, start
ledger, root registry contract, hosted-Postgres secret name) live in
`goldsky/networks/<network>.env`. Render concrete pipeline definitions
with:

```sh
./goldsky/scripts/render.sh testnet   # or: mainnet
```

which writes self-contained pipeline dirs under `goldsky/rendered/<network>/`
(gitignored — regenerate, never edit). All other scripts take those
rendered dirs. Deploy the **archive pipeline first**: the v1 pipeline's
`post_init.sql` creates views over `archive.uploads` / `archive.deploys` /
`archive.upgrades`, so on a fresh database it can only succeed once the
archive pipeline has created those tables.

```sh
DATABASE_URL=... ./goldsky/scripts/redeploy.sh goldsky/rendered/testnet/archive
DATABASE_URL=... ./goldsky/scripts/redeploy.sh --number-of-initial-subregistries 7 goldsky/rendered/testnet/v1
```

Each network must use its own Postgres database (pipelines write to the
fixed `v1`/`archive` schemas, so sharing a database would collide) and
its own Fly app for the HTTP API: `fly-app/fly.toml` is the testnet app,
`fly-app/fly_mainnet.toml` the mainnet one
(`fly deploy -c fly_mainnet.toml`).

## Deployment

1. Apply relevant pipeline changes

```sh
./goldsky/scripts/render.sh testnet # or mainnet
./goldsky/scripts/turbo.sh apply goldsky/rendered/testnet/v1/index.yaml # or relevant rendered file
```

`turbo apply` swaps in the new pipeline definition without pausing it,
dropping tables, or resetting its checkpoint.

Use `redeploy.sh` (or the manual "Redeploy Goldsky Pipeline"
GitHub Action) _only_ when the change needs history reprocessed: a
new/changed sink column, a lowered `start_at`, or recovering from the
dynamic-table race described in that script's comments.

2. Update the database with psql

If you made any database changes (likely), apply the database schema.

```sh
psql "$DATABASE_URL" -f goldsky/v1/post_init.sql
```

3. Ship the API

Testnet auto-deploys on merge to `main`. To deploy mainnet, use
the [Fly CLI](https://fly.io/docs/flyctl/install/):

```sh
fly deploy -c fly_mainnet.toml
```

## Goldsky-first approach
Goldsky-first approach uses the rendered `goldsky/rendered/<network>/v1/index.yaml` configuration file as Goldsky pipeline configuration. 
It first filters all events that belong to registry contract, then stores raw events (as a backup data).
Finally, deploy/publish event JSONs are being parsed via SQL transformer and pushed into Postgres.
Note: if there are any migration or changes in the events schema, events would need to be re-processed 
from the first snapshot (tables need to be dropped for re-processing)
Alternatively, tables could be manually migrated after a necessary pipeline change, 
and data would need to be backfilled.

## Serving
`fly-app/` reads from the `v1`/`archive` schemas populated by the pipeline
and serves the registry API. It doesn't do any data processing itself —
just reads what the pipeline has already written. See `fly-app/fly.toml`
/ `fly-app/fly_mainnet.toml` for the per-network deploy config.

### API keys
On Fly, `/v1` is rate limited per client IP (1 request per 2s, burst of 5).
Trusted consumers can skip the limit by sending an `x-api-key` header with a
key listed in the `INDEXER_API_KEYS` secret, a comma-separated list of
`name:key` entries. The name identifies the consumer: each keyed request logs
it as `api_consumer`, and startup logs the configured names (never the keys).
A request with an unrecognized key gets `401`; requests without the header are
unaffected. A malformed entry stops the app at startup.

```sh
KEY=$(openssl rand -hex 32)   # share this with the consumer
fly secrets set INDEXER_API_KEYS="orbitlens:$KEY" -c fly-app/fly.toml          # testnet
fly secrets set INDEXER_API_KEYS="orbitlens:$KEY" -c fly-app/fly_mainnet.toml  # mainnet
```

To add a consumer, append an entry (`orbitlens:...,lobstr:...`); to revoke
one, set the list without it. To rotate a key, add a second entry for the same
name, hand out the new key, then remove the old entry. Setting a secret
restarts the app.

### Change feed (`/v1/changes`)
`GET /v1/changes?order=asc|desc&limit=1-200&cursor=<event id>` lists every
registry event (`publish`, `deploy`, `register`, `rename`,
`update_address`, `update_owner`) in `(ledger, id)` order, so external
indexers can bulk-load the Registry and stay in sync. Each entry has the
event's own payload (`event`) plus the *current* state of the Wasm
(`wasm`) or contract (`contract`) it affected. Contracts are identified by
`contract.registration_id` (the id of their `register` event), which stays
the same through renames and address changes; upsert on it.

Only ledgers up to `current_through` are served. That's the pipeline's
index height (`v1.index_height`, a heartbeat sink in the v1 pipeline) minus
a 12-ledger settle margin, so an event can't land behind a client's cursor
during normal operation. Ascending responses always return `next` (the last
id, or the cursor you sent), so poll with it. If `current_through` drops
below your cursor's ledger, the pipeline was replayed (e.g.
`refresh.sh`); resync from the start.

`GET /v1/index-height` returns that raw height (`ledger`, `closed_at`)
without the margin, e.g. for showing how far the index is behind the
network.

The views behind it live in `goldsky/v1/post_init.sql`. To test against a
local Postgres (the test drops and recreates the `v1`/`archive` schemas):

```sh
createdb registry_test
TEST_DATABASE_URL=postgres:///registry_test cargo test   # in fly-app/
```

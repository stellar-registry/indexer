# Registry Indexer

Tracks activity in Stellar registry contracts using a Goldsky pipeline for data ingestion and a Rust web service (fly-app) for serving the data.

## Architecture

- **Goldsky pipeline** (`registry-turbo-v4.yaml`) filters and transforms on-chain events, pushing parsed records into Postgres via SQL transforms.
- **fly-app/** is an Actix-web service deployed on Fly.io that reads from Postgres and serves the registry API.

## Pipeline management

Validate pipeline configs offline:

```sh
npm run validate:pipelines
```

Deploy/manage the pipeline via the GitHub Actions `deploy-pipeline` workflow or the Goldsky turbo CLI directly.

## Testing

```sh
# Integration tests (requires Postgres)
npm run test:integration

# Rust service tests
cd fly-app && cargo test
```

## Testnet reset

See `testnet_reset_scripts/README.md` for the procedure to follow when the Stellar testnet resets.

## Creating DB dump
Run `./bin/create_db_dump.sh <full PG url with password>`. The script strips
Neon-specific ACL directives so the output is replayable against a plain
Postgres.

## Running tests
1. `docker-compose up -d` to start local Postgres seeded from `sql/`
2. `cargo test`

## Generating scenarios
Requires Python 3.11+ (uses stdlib `tomllib`; no third-party deps).
1. Edit `scenarios_generator/scenarios.toml` to adjust endpoints and query params.
2. Start the dev server pointed at local Postgres.
3. Run `./bin/generate_scenarios.sh` and verify the regenerated
   `src/generated.rs` looks correct.

## Creating DB dump
Run `./bin/create_db_dump.sh <full PG url with password>`  
## Running tests
1. run `docker-compose up -d` to start local PG database
2. `cargo test`
## Generating scenarios
To generate scenarios you need to have `nix` installed on your machine.
After that, update `scenarios.yaml` to set endpoint and query parameters you want to test.  
Start development server (connected to local postgres DB)
Run `./generate_scenarios.sh` and verify scenario responses looks correct.
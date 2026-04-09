# Testnet reset scripts
Make sure you have stellar cli installed. As of 04-08, revision from [this PR](https://github.com/stellar/stellar-cli/pull/2460) must be used (install cli locally)
Before doing anything else, make sure to NOT drop old tables. Instead, when network is reset, 
simply change Goldsky pipeline renaming the tables, e.g. 
`v3_registered_contracts` -> `v4_registered_contracts` etc.
1. Pause Goldsky pipeline
2. Rename tables in the Goldsky yaml file. Do that for ALL `v3_` (where `3` is current version) tables (important!)
3. Run `save_wasms.py` (use `-h` for help): this will save all registry wasms in selected folder
4. Redeploy registry using registry shell deployment script
5. Update registry contracts in Goldsky pipeline: it can be restarted now. Do not forget to update starting block in the yaml file
6. Run `restore_wasms.py` (use `-h` for help): this script redeploys wasms into registry: should be run with `manager` set as source account, and registry's contract id passed as `--registry-id`
7. Make sure fly backend is still using previous version of the tables (`v3` in this example)
8. Run `redeploy_contracts.py` (use `-h` for help): this script redeploys all contracts from main registry, deployed before through the registry. It must have connection to backend, as it fetches all necessary infromation from previous generation tables (`v3` in this example)
9. Update tables in `fly-app/src/main.rs` replacing previous version (`v3`) with current version (`v4`) and redeploy the app via `fly deploy`
------
If there are any errors during running `save_wasms.py` or `redeploy_contracts.py`, it can be resumed from last succesful cursor `--cursor`, or you can just re-run the script entirely, ignoring all errors encountered on trying to redeploy already existing contracts.


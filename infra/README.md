# Instructions
- pg_dump the DB (i dumped the schemas separately, not a full dump)
- use flyctl to proxy the DB connection, e.g. `fly proxy 5432 -a registry-testnet-db`
    - don't use the ingress IP as it can (and will) time out / disconnect mid-restore
- use pg_restore to restore the tables/schemas etc
    - when restoring `v1`, you might need to manually create the schema and enable the `pg_trgm` extension
    - after that you might need to manually create the trigram indexes (see `v1/post_init.sql`)

Since pg_restore works in 3 steps: `pre-data`, `data`, `post-data`, where `post-data` sets up the indexes, the basic 256MB RAM on the VM will be insufficient
when restoring the `archive` schema. For this you might need to bump this ram temporarily:

```bash
# fetch the machine ID
fly machine list -a <app-name>
# apply config
fly machine update <machine-id> --vm-memory 2046 -a <app-name>
```

now run `SET maintenance_work_mem = '1GB';` in `psql`, trigger the restore.
potentially you can build a TOC with `pg_restore`:
```bash
pg_restore -l archive.pgdump > archive_toc.list
```

and invoke the separate steps:
```bash
pg_restore -d <db-uri> -j 4 --section=pre-data archive.pgdump
pg_restore -d <db-uri> -j 4 --section=data archive.pgdump
pg_restore -d <db-uri> -j 4 --section=post-data archive.pgdump
```

**DON'T FORGET TO RESET THE VM RAM AND THE maintenance_work_mem**

```
fly machine update <machine-id> --vm-memory 2046 -a <app-name>
SET maintenance_work_mem = '64MB';
```


## Goldsky

It is important to create a static IP for the DB as `shared` IPs only allow HTTP handlers.
```
fly ip allocate-v4 -a <app-name>
```
after that you need to run some SQL to setup the goldsky role and grant privileges:

```sql
CREATE ROLE goldsky_writer WITH LOGIN PASSWORD 'your-password';
GRANT CREATE, CONNECT ON DATABASE db_name TO goldsky_writer;

GRANT USAGE, CREATE ON SCHEMA v1 TO goldsky_writer;
GRANT USAGE, CREATE ON SCHEMA archive TO goldsky_writer;
-- GRANT USAGE, CREATE ON SCHEMA public TO goldsky_writer; -- potentially public too

GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA v1 TO goldsky_writer;
GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA archive TO goldsky_writer;
-- GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA public TO goldsky_writer; -- potentially public too
```

after that you can create the goldsky secret with the DB credentials and point it to the pipeline.

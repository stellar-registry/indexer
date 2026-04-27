#!/usr/bin/env bash
set -euo pipefail

if [ -z "${1:-}" ]; then
  echo "Usage: $0 <database_url>"
  exit 1
fi

pg_dump --inserts --column-inserts -f dump.sql \
  --exclude-table='deploys*' \
  --exclude-table='events' \
  --exclude-table='publishes*' \
  --exclude-table='raw_events_v2' \
  --exclude-table='registries' \
  --exclude-table='test' \
  --exclude-table='v2_*' \
  --exclude-table='v4_*' \
  "$1"

# Strip Neon-specific ACL statements and psql \restrict directives so the
# dump is replayable against a plain Postgres without neon_superuser /
# cloud_admin roles.
sed -i.bak -E \
  -e '/^\\(un)?restrict /d' \
  -e '/^ALTER DEFAULT PRIVILEGES .*(cloud_admin|neon_superuser)/d' \
  dump.sql
rm -f dump.sql.bak

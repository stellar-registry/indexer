#!/usr/bin/env bash

if [ -z "$1" ]; then
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

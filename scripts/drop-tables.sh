#!/usr/bin/env bash
# Drops all sink tables and dynamic tables defined in a Goldsky pipeline YAML.
#
# Usage:
#   DATABASE_URL="postgres://..." ./scripts/drop-tables.sh registry-testnet-v4.yaml

set -euo pipefail

SKIP_CONFIRM=false
if [[ "${1:-}" == "-y" || "${1:-}" == "--yes" ]]; then
  SKIP_CONFIRM=true
  shift
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "error: DATABASE_URL is not set" >&2
  exit 1
fi

YAML_FILE="${1:?usage: $0 <pipeline.yaml>}"

if [[ ! -f "$YAML_FILE" ]]; then
  echo "error: file not found: $YAML_FILE" >&2
  exit 1
fi

# Extract sink table names (lines like "        table: v4_foo")
# and dynamic table backend_entity_name values
tables=()
while IFS= read -r line; do
  # sink tables
  if [[ "$line" =~ ^[[:space:]]+table:[[:space:]]+(.+)$ ]]; then
    tables+=("${BASH_REMATCH[1]}")
  fi
  # dynamic tables
  if [[ "$line" =~ ^[[:space:]]+backend_entity_name:[[:space:]]+(.+)$ ]]; then
    tables+=("${BASH_REMATCH[1]}")
  fi
done < "$YAML_FILE"

if (( ${#tables[@]} == 0 )); then
  echo "no tables found in $YAML_FILE" >&2
  exit 1
fi

echo "tables to drop:"
for t in "${tables[@]}"; do
  echo "  - $t"
done

if [[ "$SKIP_CONFIRM" != true ]]; then
  read -rp "proceed? [y/N] " confirm
  if [[ "$confirm" != [yY] ]]; then
    echo "aborted"
    exit 0
  fi
fi

sql=""
for t in "${tables[@]}"; do
  sql+="DROP TABLE IF EXISTS public.\"$t\" CASCADE; "
done

psql "$DATABASE_URL" -c "$sql"

echo "done — dropped ${#tables[@]} table(s)"

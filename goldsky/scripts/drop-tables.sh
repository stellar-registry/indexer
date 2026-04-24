#!/usr/bin/env bash
# Drops every named schema referenced by a Goldsky pipeline YAML.
#
# We only use named schemas (never `public`), so wholesale
# `DROP SCHEMA ... CASCADE` is safe and takes out every table, view,
# dynamic-table backing row, etc. in one shot. If the YAML references
# `schema: public` we refuse to proceed — that would risk clobbering
# unrelated tables shared with other systems.
#
# Usage:
#   DATABASE_URL="postgres://..." ./goldsky/scripts/drop-tables.sh goldsky/v1/index.yaml

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

# Collect the unique set of schemas referenced in the YAML.
schemas=()
seen=""
while IFS= read -r line; do
  if [[ "$line" =~ ^[[:space:]]+schema:[[:space:]]+(.+)$ ]]; then
    schema="${BASH_REMATCH[1]}"
    if [[ "$schema" == "public" ]]; then
      echo "error: $YAML_FILE references schema: public — refusing to drop (would clobber unrelated tables)" >&2
      exit 1
    fi
    case " $seen " in
      *" $schema "*) ;;
      *)
        schemas+=("$schema")
        seen="$seen $schema"
        ;;
    esac
  fi
done < "$YAML_FILE"

if (( ${#schemas[@]} == 0 )); then
  echo "no schemas found in $YAML_FILE" >&2
  exit 1
fi

echo "schemas to drop:"
for s in "${schemas[@]}"; do
  echo "  - $s"
done

if [[ "$SKIP_CONFIRM" != true ]]; then
  read -rp "proceed? [y/N] " confirm
  if [[ "$confirm" != [yY] ]]; then
    echo "aborted"
    exit 0
  fi
fi

sql=""
for s in "${schemas[@]}"; do
  sql+="DROP SCHEMA IF EXISTS \"$s\" CASCADE; "
done

psql "$DATABASE_URL" -c "$sql"

echo "done — dropped ${#schemas[@]} schema(s)"

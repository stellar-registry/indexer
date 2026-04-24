#!/usr/bin/env bash
# Stops (pauses) a Goldsky turbo pipeline, drops its tables, validates
# and re-applies the pipeline definition, then (if present) applies
# post_init.sql once Goldsky has materialized the sink tables.
#
# Usage:
#   DATABASE_URL="postgres://..." ./goldsky/scripts/redeploy.sh goldsky/v1
#
# The argument is a directory containing:
#   - index.yaml      (required) Goldsky pipeline definition
#   - post_init.sql   (optional) SQL run after the pipeline is applied,
#                     e.g. CREATE VIEW statements that depend on the
#                     tables Goldsky provisions at the sinks.
#
# Tunables (env):
#   POST_INIT_MAX_ATTEMPTS   number of psql retries     (default: 12)
#   POST_INIT_RETRY_DELAY    seconds between retries    (default: 5)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"
DROP="$SCRIPT_DIR/drop-tables.sh"

PIPELINE_DIR="${1:?usage: $0 <pipeline-dir>}"
PIPELINE_DIR="${PIPELINE_DIR%/}"

if [[ ! -d "$PIPELINE_DIR" ]]; then
  echo "error: not a directory: $PIPELINE_DIR" >&2
  exit 1
fi

YAML_FILE="$PIPELINE_DIR/index.yaml"
POST_INIT_SQL="$PIPELINE_DIR/post_init.sql"

if [[ ! -f "$YAML_FILE" ]]; then
  echo "error: file not found: $YAML_FILE" >&2
  exit 1
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "error: DATABASE_URL is not set" >&2
  exit 1
fi

# Extract pipeline name from the YAML (first "name:" line)
PIPELINE_NAME=""
while IFS= read -r line; do
  if [[ "$line" =~ ^name:[[:space:]]+(.+)$ ]]; then
    PIPELINE_NAME="${BASH_REMATCH[1]}"
    break
  fi
done < "$YAML_FILE"

if [[ -z "$PIPELINE_NAME" ]]; then
  echo "error: could not find pipeline name in $YAML_FILE" >&2
  exit 1
fi

echo "==> pipeline: $PIPELINE_NAME"
echo "==> dir:      $PIPELINE_DIR"
echo "==> yaml:     $YAML_FILE"
if [[ -f "$POST_INIT_SQL" ]]; then
  echo "==> post_init: $POST_INIT_SQL"
else
  echo "==> post_init: (none)"
fi
echo ""

# 1. Stop (pause) the pipeline
echo "==> pausing pipeline..."
"$TURBO" pause "$PIPELINE_NAME"
echo ""

# 2. Drop tables
echo "==> dropping tables..."
"$DROP" -y "$YAML_FILE"
echo ""

# 3. Validate
echo "==> validating pipeline definition..."
"$TURBO" validate "$YAML_FILE"
echo ""

# 4. Apply
echo "==> applying pipeline..."
"$TURBO" apply "$YAML_FILE"
echo ""

# 5. Apply post_init.sql (if present), retrying until Goldsky has
#    created the sink tables the script depends on. Any psql error
#    (e.g. "relation ... does not exist") triggers a retry. psql
#    stderr is suppressed on non-final attempts so the expected
#    "relation does not exist" noise doesn't clutter the log; the
#    final attempt lets stderr through so real failures surface.
if [[ -f "$POST_INIT_SQL" ]]; then
  max_attempts="${POST_INIT_MAX_ATTEMPTS:-12}"
  delay="${POST_INIT_RETRY_DELAY:-5}"
  echo "==> applying $POST_INIT_SQL (up to $max_attempts attempts, ${delay}s apart)..."
  attempt=1
  while true; do
    if (( attempt < max_attempts )); then
      psql_stderr=/dev/null
    else
      psql_stderr=/dev/stderr
    fi
    if psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -q -f "$POST_INIT_SQL" 2>"$psql_stderr"; then
      echo "==> post_init.sql applied (attempt $attempt/$max_attempts)"
      break
    fi
    if (( attempt >= max_attempts )); then
      echo "error: post_init.sql failed after $attempt attempts — tables may not have been created in time" >&2
      exit 1
    fi
    echo "  tables not ready yet (attempt $attempt/$max_attempts); retrying in ${delay}s..."
    attempt=$((attempt + 1))
    sleep "$delay"
  done
  echo ""
fi

echo "==> redeploy complete"

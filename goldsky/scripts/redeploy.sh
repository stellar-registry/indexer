#!/usr/bin/env bash
# Stops (pauses) a Goldsky turbo pipeline, drops its tables, validates
# and re-applies the pipeline definition, then (if present) applies
# post_init.sql once Goldsky has materialized the sink tables.
#
# When `--number-of-initial-subregistries N` is passed, the script also
# runs a post-apply verification flow that detects and recovers from
# events dropped by the `dynamic_table_check` race in the v1 pipeline:
#
#   1. Poll v1.registries_dynamic_table until it has N rows, so the
#      Postgres-backed membership set is fully seeded. The expected
#      count is the bootstrap size — the number of contracts that
#      register themselves via `sub_reg` as part of the initial
#      deployment (root + its initial subregistries). Pass the value
#      that matches the deployment being replayed.
#   2. Sleep briefly to let any in-flight transform_5_* writes settle.
#   3. Run goldsky/v1/audit-race.sql. A non-empty result means
#      one or more events were dropped by the race between
#      transform_3's Postgres write and transform_4's dynamic_table
#      read (see that SQL file's header for the full explanation).
#   4. If drops are found, run goldsky/scripts/refresh.sh, which does a
#      `turbo restart --clear-state`. The Postgres dynamic table
#      survives the state clear, so the replay sees every contract_id
#      from the first ledger and recovers the lost events.
#   5. Re-audit. Exit non-zero if drops remain, since at that point
#      something is wrong beyond the known race (e.g. pipeline
#      definition bug, emitters missing from v1.registries).
#
# Usage:
#   DATABASE_URL="postgres://..." ./goldsky/scripts/redeploy.sh goldsky/v1
#   DATABASE_URL="postgres://..." ./goldsky/scripts/redeploy.sh \
#       --number-of-initial-subregistries 7 goldsky/v1
#
# The positional argument is a directory containing:
#   - index.yaml      (required) Goldsky pipeline definition
#   - post_init.sql   (optional) SQL run after the pipeline is applied,
#                     e.g. CREATE VIEW statements that depend on the
#                     tables Goldsky provisions at the sinks.
#
# Tunables (env):
#   POST_INIT_MAX_ATTEMPTS        post_init psql retries   (default: 12)
#   POST_INIT_RETRY_DELAY         seconds between retries  (default: 5)
#   DYNAMIC_TABLE_MAX_ATTEMPTS    dynamic-table poll tries (default: 60)
#   DYNAMIC_TABLE_RETRY_DELAY     seconds between polls    (default: 5)
#   AUDIT_SETTLE_DELAY            sleep before audit       (default: 10)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"
DROP="$SCRIPT_DIR/drop-tables.sh"
REFRESH="$SCRIPT_DIR/refresh.sh"

usage() {
  echo "usage: $0 [--number-of-initial-subregistries N] <pipeline-dir>" >&2
}

EXPECTED_SUBREGISTRIES=""
PIPELINE_DIR=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --number-of-initial-subregistries)
      EXPECTED_SUBREGISTRIES="${2:?--number-of-initial-subregistries requires a value}"
      shift 2
      ;;
    --number-of-initial-subregistries=*)
      EXPECTED_SUBREGISTRIES="${1#*=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      PIPELINE_DIR="${1:-}"
      shift || true
      break
      ;;
    -*)
      echo "error: unknown flag: $1" >&2
      usage
      exit 1
      ;;
    *)
      if [[ -n "$PIPELINE_DIR" ]]; then
        echo "error: unexpected positional arg: $1" >&2
        usage
        exit 1
      fi
      PIPELINE_DIR="$1"
      shift
      ;;
  esac
done

if [[ -z "$PIPELINE_DIR" ]]; then
  usage
  exit 1
fi

if [[ -n "$EXPECTED_SUBREGISTRIES" && ! "$EXPECTED_SUBREGISTRIES" =~ ^[0-9]+$ ]]; then
  echo "error: --number-of-initial-subregistries must be a non-negative integer" >&2
  exit 1
fi

PIPELINE_DIR="${PIPELINE_DIR%/}"
AUDIT_SQL="$PIPELINE_DIR/audit-race.sql"

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

# Extract pipeline name from the YAML (first "name:" line) and the
# Postgres sink schema (first indented "schema: X" line — sinks share a
# schema, so the first match is canonical). PIPELINE_SCHEMA replaces
# previously-hardcoded "v1." prefixes (e.g. in the dynamic-table audit),
# so the same script works against goldsky/v1, goldsky/v2, etc.
PIPELINE_NAME=""
PIPELINE_SCHEMA=""
while IFS= read -r line; do
  if [[ -z "$PIPELINE_NAME" && "$line" =~ ^name:[[:space:]]+(.+)$ ]]; then
    PIPELINE_NAME="${BASH_REMATCH[1]}"
  fi
  if [[ -z "$PIPELINE_SCHEMA" && "$line" =~ ^[[:space:]]+schema:[[:space:]]+(.+)$ ]]; then
    PIPELINE_SCHEMA="${BASH_REMATCH[1]}"
  fi
  if [[ -n "$PIPELINE_NAME" && -n "$PIPELINE_SCHEMA" ]]; then
    break
  fi
done < "$YAML_FILE"

if [[ -z "$PIPELINE_NAME" ]]; then
  echo "error: could not find pipeline name in $YAML_FILE" >&2
  exit 1
fi

if [[ -z "$PIPELINE_SCHEMA" ]]; then
  echo "error: could not find sink schema in $YAML_FILE" >&2
  exit 1
fi

echo "==> pipeline: $PIPELINE_NAME"
echo "==> schema:   $PIPELINE_SCHEMA"
echo "==> dir:      $PIPELINE_DIR"
echo "==> yaml:     $YAML_FILE"
if [[ -f "$POST_INIT_SQL" ]]; then
  echo "==> post_init: $POST_INIT_SQL"
else
  echo "==> post_init: (none)"
fi
if [[ -n "$EXPECTED_SUBREGISTRIES" ]]; then
  echo "==> expected initial subregistries: $EXPECTED_SUBREGISTRIES"
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

# 5. Restart with --clear-state so the source checkpoint rewinds to
#    start_at and replays the full history into the freshly-dropped
#    sink tables. Without --clear-state the pipeline would resume from
#    its last checkpoint and only repopulate events going forward.
echo "==> restarting pipeline (--clear-state)..."
"$TURBO" restart --clear-state "$PIPELINE_NAME"
echo ""

# 6. Apply post_init.sql (if present), retrying until Goldsky has
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

# 7. Race-condition verification and recovery. Only runs when the caller
#    supplied the expected count of initial subregistries, because the
#    audit needs a definition of "the dynamic table is fully seeded"
#    before it's meaningful to check for drops.
if [[ -n "$EXPECTED_SUBREGISTRIES" ]]; then
  dt_max_attempts="${DYNAMIC_TABLE_MAX_ATTEMPTS:-60}"
  dt_delay="${DYNAMIC_TABLE_RETRY_DELAY:-5}"
  settle_delay="${AUDIT_SETTLE_DELAY:-10}"

  echo "==> waiting for ${PIPELINE_SCHEMA}.registries_dynamic_table to reach $EXPECTED_SUBREGISTRIES rows"
  echo "    (up to $dt_max_attempts attempts, ${dt_delay}s apart)..."
  attempt=1
  while true; do
    count=$(psql "$DATABASE_URL" -tAc "SELECT count(*) FROM ${PIPELINE_SCHEMA}.registries_dynamic_table" 2>/dev/null || echo "")
    if [[ "$count" == "$EXPECTED_SUBREGISTRIES" ]]; then
      echo "==> dynamic table seeded: $count/$EXPECTED_SUBREGISTRIES rows (attempt $attempt/$dt_max_attempts)"
      break
    fi
    if (( attempt >= dt_max_attempts )); then
      echo "error: v1.registries_dynamic_table has '$count' rows, expected $EXPECTED_SUBREGISTRIES, after $attempt attempts" >&2
      exit 1
    fi
    echo "  dynamic table at '$count' rows (attempt $attempt/$dt_max_attempts); retrying in ${dt_delay}s..."
    attempt=$((attempt + 1))
    sleep "$dt_delay"
  done
  echo ""

  # Let any in-flight transform_5_* writes land before auditing, so we
  # don't falsely flag events that are merely buffered.
  echo "==> settling ${settle_delay}s before audit..."
  sleep "$settle_delay"
  echo ""

  # Run the race audit once and return the dropped row count via stdout.
  # Any non-empty dropped-row payload is echoed to stderr so the failure
  # detail lands in the log without polluting the captured count.
  # tuples-only, unaligned, quiet -> one row per output line.
  audit_run() {
    local rows
    rows=$(psql "$DATABASE_URL" -tAq -f "$AUDIT_SQL")
    if [[ -z "$rows" ]]; then
      echo 0
    else
      printf '%s\n' "$rows" >&2
      printf '%s\n' "$rows" | wc -l
    fi
  }

  echo "==> running race audit: $AUDIT_SQL"
  dropped=$(audit_run)

  if (( dropped == 0 )); then
    echo "==> no race drops detected — redeploy complete"
    exit 0
  fi

  echo ""
  echo "==> race drops detected ($dropped events). Running refresh to recover..."
  "$REFRESH" "$PIPELINE_DIR"
  echo ""

  echo "==> settling ${settle_delay}s before re-audit..."
  sleep "$settle_delay"
  echo ""

  echo "==> re-running race audit after refresh..."
  dropped_after=$(audit_run)

  if (( dropped_after > 0 )); then
    echo "error: $dropped_after events still missing after refresh — manual investigation needed" >&2
    exit 1
  fi

  echo "==> refresh recovered all dropped events"
fi

echo "==> redeploy complete"

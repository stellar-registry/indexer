#!/usr/bin/env bash
# Restarts a Goldsky turbo pipeline with `--clear-state`, which resets
# the source checkpoint back to `start_at` and replays the stream from
# the beginning.
#
# The point of doing this is to work around the race between
# `dynamic_table_check` reads in one transform and writes to the
# Postgres-backed dynamic table from another transform. Events whose
# emitter contract_id was added to the dynamic table only a few ledgers
# earlier can be dropped by the check because the Postgres write hasn't
# committed yet. See goldsky/v1/index.yaml transforms 3 and 4.
#
# `turbo restart --clear-state` clears pipeline state (source
# checkpoints) but does not truncate the Postgres entity backing the
# dynamic table. So on the replay the dynamic_table_check sees the
# fully-populated membership set from the prior run and the race no
# longer filters events out.
#
# Prerequisite: run the pipeline once with `redeploy.sh` first so the
# dynamic table has been populated with every contract_id the filter
# will need.
#
# Usage:
#   ./goldsky/scripts/refresh.sh goldsky/v1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"

PIPELINE_DIR="${1:?usage: $0 <pipeline-dir>}"
PIPELINE_DIR="${PIPELINE_DIR%/}"

if [[ ! -d "$PIPELINE_DIR" ]]; then
  echo "error: not a directory: $PIPELINE_DIR" >&2
  exit 1
fi

YAML_FILE="$PIPELINE_DIR/index.yaml"

if [[ ! -f "$YAML_FILE" ]]; then
  echo "error: file not found: $YAML_FILE" >&2
  exit 1
fi

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
echo "==> restarting with --clear-state (source checkpoint reset, Postgres dynamic tables preserved)..."
"$TURBO" restart "$PIPELINE_NAME" --clear-state
echo "==> refresh complete"

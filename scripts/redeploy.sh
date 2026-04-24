#!/usr/bin/env bash
# Stops (pauses) a Goldsky turbo pipeline, drops its tables, then validates
# and re-applies the pipeline definition.
#
# Usage:
#   DATABASE_URL="postgres://..." ./scripts/redeploy.sh registry-testnet-v4.yaml

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"
DROP="$SCRIPT_DIR/drop-tables.sh"

YAML_FILE="${1:?usage: $0 <pipeline.yaml>}"

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
echo "==> yaml:     $YAML_FILE"
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

echo "==> redeploy complete"

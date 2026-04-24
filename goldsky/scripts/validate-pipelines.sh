#!/usr/bin/env bash
# Runs `goldsky turbo validate` over every goldsky/v*/index.yaml pipeline.
# Requires the turbo CLI on PATH (install via https://goldsky.com/install).
# `turbo validate` is an offline YAML schema check — no auth, no network.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
shopt -s nullglob
YAMLS=("$REPO_ROOT"/goldsky/v*/index.yaml)

if (( ${#YAMLS[@]} == 0 )); then
  echo "no goldsky/v*/index.yaml pipelines found in $REPO_ROOT" >&2
  exit 1
fi

failures=0
for yaml in "${YAMLS[@]}"; do
  echo "validating $(basename "$yaml")"
  if ! "$TURBO" validate "$yaml"; then
    echo "FAIL: $yaml" >&2
    failures=$((failures + 1))
  fi
done

if (( failures > 0 )); then
  echo "$failures pipeline(s) failed validation" >&2
  exit 1
fi

echo "all pipelines valid"

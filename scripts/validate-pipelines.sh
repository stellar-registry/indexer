#!/usr/bin/env bash
# Runs `goldsky turbo validate` over every registry-*.yaml in the repo root.
# Requires the turbo CLI on PATH (install via https://goldsky.com/install).
# `turbo validate` is an offline YAML schema check — no auth, no network.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
shopt -s nullglob
# Only validate the turbo pipelines. registry-minimal.yaml is an older
# non-turbo definition and uses a different schema.
YAMLS=("$REPO_ROOT"/registry-testnet-*.yaml)

if (( ${#YAMLS[@]} == 0 )); then
  echo "no registry-turbo-*.yaml pipelines found in $REPO_ROOT" >&2
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

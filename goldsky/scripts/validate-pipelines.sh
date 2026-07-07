#!/usr/bin/env bash
# Renders every network × pipeline combination from the templates and
# runs `goldsky turbo validate` over each rendered definition.
# Requires the turbo CLI (install via https://goldsky.com/install) and
# envsubst. `turbo validate` is an offline YAML schema check — no auth,
# no network.
#
# Paths handed to turbo.sh are repo-relative so they resolve both when
# the turbo binary runs directly (turbo.sh cd's to the repo root) and
# under its Docker fallback (repo root mounted at /w).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TURBO="$SCRIPT_DIR/turbo.sh"
RENDER="$SCRIPT_DIR/render.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

shopt -s nullglob
ENV_FILES=("$REPO_ROOT"/goldsky/networks/*.env)

if (( ${#ENV_FILES[@]} == 0 )); then
  echo "no goldsky/networks/*.env files found in $REPO_ROOT" >&2
  exit 1
fi

failures=0
for env_file in "${ENV_FILES[@]}"; do
  network="$(basename "$env_file" .env)"
  "$RENDER" "$network"
  for yaml in "$REPO_ROOT/goldsky/rendered/$network"/*/index.yaml; do
    rel="${yaml#"$REPO_ROOT"/}"
    echo "validating $rel"
    if ! "$TURBO" validate "$rel"; then
      echo "FAIL: $rel" >&2
      failures=$((failures + 1))
    fi
  done
done

if (( failures > 0 )); then
  echo "$failures pipeline(s) failed validation" >&2
  exit 1
fi

echo "all pipelines valid"

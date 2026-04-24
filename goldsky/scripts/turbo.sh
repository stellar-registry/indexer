#!/usr/bin/env bash
# Wrapper that runs `goldsky turbo` inside a debian:trixie-slim container
# to work around the host GLIBC < 2.39 incompatibility.
#
# Usage:
#   ./goldsky/scripts/turbo.sh validate goldsky/v1/index.yaml
#   ./goldsky/scripts/turbo.sh apply goldsky/v1/index.yaml
#   ./goldsky/scripts/turbo.sh stop registry-testnet-v1
#   ./goldsky/scripts/turbo.sh list

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
GOLDSKY_DIR="${GOLDSKY_DIR:-$HOME/.goldsky}"
HOST_TURBO="$GOLDSKY_DIR/bin/turbo"

if [[ ! -f "$HOST_TURBO" ]]; then
  echo "error: turbo binary not found at $HOST_TURBO" >&2
  echo "install with: curl -fsSL https://install-turbo.goldsky.com | bash" >&2
  exit 1
fi

# Try running the binary directly; fall back to Docker if GLIBC is too old.
if "$HOST_TURBO" --version &>/dev/null; then
  cd "$REPO_ROOT"
  exec "$HOST_TURBO" "$@"
else
  exec docker run --rm \
    -v "$GOLDSKY_DIR:/root/.goldsky" \
    -v "$REPO_ROOT:/w" -w /w \
    debian:trixie-slim \
    /root/.goldsky/bin/turbo "$@"
fi

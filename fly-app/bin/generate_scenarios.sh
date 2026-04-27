#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FLY_APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

OUT="$FLY_APP_DIR/src/generated.rs"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

python3 "$FLY_APP_DIR/scenarios_generator/generate_urls.py" > "$TMP"
mv "$TMP" "$OUT"

#!/usr/bin/env bash
# Renders per-network Goldsky pipeline definitions from the checked-in
# templates.
#
# Pipeline definitions live as goldsky/<pipeline>/index.template.yaml
# with ${VAR} placeholders; per-network values live in
# goldsky/networks/<network>.env. This script substitutes the values
# (envsubst with an explicit variable list, so the $[0] / $.map JSON
# paths inside the SQL are left alone) and writes the result to
#
#   goldsky/rendered/<network>/<pipeline>/index.yaml
#
# alongside a copy of the pipeline's sidecar *.sql files (post_init.sql,
# audit-race.sql), so the rendered directory is a self-contained
# pipeline dir usable with the existing scripts:
#
#   ./goldsky/scripts/render.sh mainnet
#   DATABASE_URL=... ./goldsky/scripts/redeploy.sh goldsky/rendered/mainnet/v1
#
# goldsky/rendered/ is gitignored — regenerate it, never edit it.
#
# Usage:
#   ./goldsky/scripts/render.sh <network> [pipeline ...]
#
#   <network>   basename of a goldsky/networks/*.env file (testnet, mainnet)
#   [pipeline]  pipeline directory names under goldsky/ (default: every
#               directory containing an index.template.yaml)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
GOLDSKY_DIR="$REPO_ROOT/goldsky"

# The full set of placeholders templates may use — single source of
# truth for both the envsubst allowlist and the preflight checks below.
# envsubst substitutes ONLY these; any other $-token in the YAML/SQL
# survives verbatim.
TEMPLATE_VAR_NAMES=(NETWORK DATASET_PREFIX V1_START_AT ARCHIVE_START_AT ROOT_REGISTRY PG_SECRET_NAME INDEXER_WEBHOOK_SECRET_NAME INDEXER_WEBHOOK_BASE_URL)
TEMPLATE_VARS=""
for v in "${TEMPLATE_VAR_NAMES[@]}"; do
  TEMPLATE_VARS+="\${$v} "
done

usage() {
  echo "usage: $0 <network> [pipeline ...]" >&2
  echo "  networks:  $(ls "$GOLDSKY_DIR/networks" 2>/dev/null | sed 's/\.env$//' | paste -sd' ' -)" >&2
}

NETWORK_NAME="${1:-}"
if [[ -z "$NETWORK_NAME" ]]; then
  usage
  exit 1
fi
shift

ENV_FILE="$GOLDSKY_DIR/networks/$NETWORK_NAME.env"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "error: unknown network '$NETWORK_NAME' ($ENV_FILE not found)" >&2
  usage
  exit 1
fi

if ! command -v envsubst >/dev/null; then
  echo "error: envsubst not found (install gettext)" >&2
  exit 1
fi

# Default to every pipeline that has a template. A full render (no
# explicit pipeline args) also prunes the network's rendered tree, so a
# pipeline renamed or removed at the source can't linger as a stale
# rendered dir that validate-pipelines.sh or redeploy.sh would pick up.
PIPELINES=("$@")
if (( ${#PIPELINES[@]} == 0 )); then
  rm -rf "$GOLDSKY_DIR/rendered/$NETWORK_NAME"
  for tpl in "$GOLDSKY_DIR"/*/index.template.yaml; do
    PIPELINES+=("$(basename "$(dirname "$tpl")")")
  done
fi

if (( ${#PIPELINES[@]} == 0 )); then
  echo "error: no goldsky/*/index.template.yaml templates found" >&2
  exit 1
fi

# Clear any inherited values first: without this, a variable missing
# from the env file would be silently satisfied by whatever the calling
# shell happens to export (e.g. a testnet ROOT_REGISTRY left over from
# debugging) and rendered into the wrong network's pipeline.
unset "${TEMPLATE_VAR_NAMES[@]}"

set -a
# shellcheck source=/dev/null
source "$ENV_FILE"
set +a

# Every template variable must be set and non-empty: envsubst would
# silently substitute an empty string for an unset variable, producing
# a syntactically valid but wrong pipeline (e.g. emitter_contract_id = '').
for var in "${TEMPLATE_VAR_NAMES[@]}"; do
  if [[ -z "${!var:-}" ]]; then
    echo "error: $var is not set in $ENV_FILE" >&2
    exit 1
  fi
done

if [[ "$PG_SECRET_NAME" == *TODO* ]]; then
  echo "warning: PG_SECRET_NAME=$PG_SECRET_NAME is a placeholder — provision the" >&2
  echo "         hosted Postgres and set the real Goldsky secret name before 'turbo apply'" >&2
fi

for pipeline in "${PIPELINES[@]}"; do
  template="$GOLDSKY_DIR/$pipeline/index.template.yaml"
  if [[ ! -f "$template" ]]; then
    echo "error: template not found: $template" >&2
    exit 1
  fi

  # Clean before rendering so files deleted or renamed in the source
  # pipeline dir (e.g. a dropped post_init.sql) can't survive as stale
  # copies that redeploy.sh would happily apply.
  out_dir="$GOLDSKY_DIR/rendered/$NETWORK_NAME/$pipeline"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"

  envsubst "$TEMPLATE_VARS" < "$template" > "$out_dir/index.yaml"

  # A placeholder surviving substitution means the template uses a
  # variable that isn't in TEMPLATE_VARS — add it there and to the env
  # files (envsubst leaves unlisted variables untouched).
  if grep -nE '\$\{[A-Za-z_][A-Za-z0-9_]*\}' "$out_dir/index.yaml"; then
    echo "error: unresolved placeholders in $out_dir/index.yaml (add the variable to TEMPLATE_VARS and the env files)" >&2
    exit 1
  fi

  # Sidecar SQL (post_init.sql, audit-race.sql) rides along so the
  # rendered dir works as a redeploy.sh pipeline dir.
  for sql in "$GOLDSKY_DIR/$pipeline"/*.sql; do
    [[ -e "$sql" ]] && cp "$sql" "$out_dir/"
  done

  echo "rendered $NETWORK_NAME/$pipeline -> ${out_dir#"$REPO_ROOT"/}/index.yaml"
done

#!/usr/bin/env bash
# Backfills v1.contract_verifications for contracts that registered before
# the verify-build webhook existed (see fly-app/src/verification.rs).
#
# Goldsky's verify_build_webhook sink only fires POST /v1/webhooks/verified-build
# for `register` events it processes going forward from wherever its
# checkpoint currently is — a plain `turbo apply` (schema-only change, no
# reprocessing) doesn't replay history — so any contract registered before
# this feature shipped never got checked and has no row in
# v1.contract_verifications at all (indistinguishable via the API from
# "checked, not verified").
#
# This script does the same check verify_contract() does — query Stellar
# Expert, upsert into v1.contract_verifications — for every contract_id
# that's missing a row, so historical registrations get the same treatment
# newly-registered ones get automatically.
#
# Usage:
#   DATABASE_URL="postgres://..." ./fly-app/scripts/backfill-verifications.sh [network]
#
#   [network]  Stellar Expert network segment: testnet (default) or public
#              (Stellar Expert's own naming for mainnet)
#
# Requires: psql, curl, jq

set -euo pipefail

SEGMENT="${1:-testnet}"
if [[ "$SEGMENT" != "testnet" && "$SEGMENT" != "public" ]]; then
  echo "error: network must be 'testnet' or 'public' (got '$SEGMENT')" >&2
  exit 1
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "error: DATABASE_URL is not set" >&2
  exit 1
fi

for cmd in psql curl jq; do
  command -v "$cmd" >/dev/null || { echo "error: $cmd is required" >&2; exit 1; }
done

# One row per contract_id with no verification record yet, picking its
# earliest registration event so ledger_sequence reflects first
# registration — same as what the live webhook would have recorded.
query_result=$(psql "$DATABASE_URL" -tAF'|' -c "
  SELECT DISTINCT ON (contract_id) contract_id, ledger_sequence
  FROM v1.registered_contracts
  WHERE contract_id IS NOT NULL
    AND NOT EXISTS (
      SELECT 1 FROM v1.contract_verifications cv
      WHERE cv.contract_id = registered_contracts.contract_id
    )
  ORDER BY contract_id, ledger_sequence ASC
")

total=0
checked=0
verified_count=0
while IFS='|' read -r contract_id ledger_sequence; do
  [[ -z "$contract_id" ]] && continue
  total=$((total + 1))
done <<<"$query_result"

echo "found $total contract(s) with no verification record (segment: $SEGMENT)"

while IFS='|' read -r contract_id ledger_sequence; do
  [[ -z "$contract_id" ]] && continue

  response=$(curl -s "https://api.stellar.expert/explorer/${SEGMENT}/contract/${contract_id}")
  validation_status=$(jq -r '.validation.status // empty' <<<"$response")

  if [[ "$validation_status" == "verified" ]]; then
    repository=$(jq -r '.validation.repository // empty' <<<"$response")
    commit_hash=$(jq -r '.validation.commit // empty' <<<"$response")
    package=$(jq -r '.validation.package // empty' <<<"$response")
    path=$(jq -r '.validation.path // empty' <<<"$response")
    verified_count=$((verified_count + 1))
    echo "  verified:   $contract_id ($repository @ $commit_hash)"
  else
    # Not verified (or Stellar Expert has no record at all): still write
    # the row, with everything but contract_id/checked_at/ledger_sequence
    # NULL — same as verify_contract() does for the "checked, not
    # verified" case, so this contract won't be re-checked forever.
    repository=""
    commit_hash=""
    package=""
    path=""
    validation_status=""
    echo "  unverified: $contract_id"
  fi

  # Variable interpolation (:'name') only works when psql reads the SQL
  # from a file or stdin, not from -c — so this is fed via heredoc, with
  # every value bound through -v/:'name' rather than shell-interpolated
  # into the SQL text.
  psql "$DATABASE_URL" -q \
    -v contract_id="$contract_id" \
    -v status="$validation_status" \
    -v repository="$repository" \
    -v commit_hash="$commit_hash" \
    -v package="$package" \
    -v path="$path" \
    -v ledger_sequence="$ledger_sequence" <<'SQL'
      INSERT INTO v1.contract_verifications
        (contract_id, status, repository, commit_hash, package, path, checked_at, ledger_sequence)
      VALUES (
        :'contract_id',
        NULLIF(:'status', ''),
        NULLIF(:'repository', ''),
        NULLIF(:'commit_hash', ''),
        NULLIF(:'package', ''),
        NULLIF(:'path', ''),
        now(),
        NULLIF(:'ledger_sequence', '')::int8
      )
      ON CONFLICT (contract_id) DO NOTHING
SQL

  checked=$((checked + 1))
  sleep 0.2 # be polite to Stellar Expert's API
done <<<"$query_result"

echo "done: checked $checked contract(s), $verified_count verified"

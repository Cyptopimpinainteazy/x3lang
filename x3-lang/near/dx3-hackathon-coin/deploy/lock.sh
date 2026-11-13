#!/usr/bin/env bash
# Permanently locks the DX3HACK contract by deleting every full-access key on its account.
# Afterwards nobody, including the owner, can change its code, burn time or bound .x3 program.
# The owner functions (pause, end_now, add_holder, mint) keep working: they're called from
# x3lang.<network>, not signed by the token account.
#
#   deploy/lock.sh testnet
#   deploy/lock.sh mainnet
#
# There is no undo. Run it only after deploy/smoke.sh passed and you've checked the terms.
set -euo pipefail
cd "$(dirname "$0")/.."
. deploy/lib.sh

[ $# -eq 1 ] || die "usage: deploy/lock.sh testnet|mainnet"
set_network "$1"
need near jq curl

has_contract "$TOKEN" || die "$TOKEN has no contract on $NETWORK"
cfg=$(view "$TOKEN" hackathon_config '{}')
echo "Stored terms on $TOKEN:"
jq '{owner_id, faucet_operator_id, holder_parent_id, expires_at, burn_program_sha256, max_supply, active}' <<<"$cfg"

full=$(access_keys "$TOKEN" | jq -r '.keys[] | select(.access_key.permission == "FullAccess") | .public_key')
other=$(access_keys "$TOKEN" | jq -r '.keys[] | select(.access_key.permission != "FullAccess") | .public_key')
[ -z "$other" ] || die "$TOKEN has function-call keys too; remove or review them first: $other"
if [ -z "$full" ]; then
  echo "$TOKEN already has no keys. It's locked."
  exit 0
fi
echo
echo "Full-access keys to delete from $TOKEN:"
echo "$full"
echo
echo "This is permanent. To continue, type the token account name ($TOKEN):"
read -r answer </dev/tty
[ "$answer" = "$TOKEN" ] || die "stopped; nothing was deleted"

near account delete-keys "$TOKEN" public-keys "$(paste -sd, <<<"$full")" \
  network-config "$NETWORK" sign-with-keychain send

left=$(access_keys "$TOKEN" | jq '.keys | length')
[ "$left" = "0" ] || die "$TOKEN still has $left keys"
echo "$TOKEN is locked: 0 access keys. $EXPLORER/address/$TOKEN"

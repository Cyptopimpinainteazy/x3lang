#!/usr/bin/env bash
# Owner-run setup for Dotx3 HACKATHON ONLY COIN (DX3HACK).
#
#   deploy/setup.sh testnet <dx3_hackathon_coin.wasm> <expected wasm sha256>
#   deploy/setup.sh mainnet <dx3_hackathon_coin.wasm> <expected wasm sha256>
#
# Use the wasm and SHA256SUMS from the "DX3HACK Hackathon Coin (Unsigned Build)" CI artifact.
# Every signing step shows its cost and waits for you to type yes. Steps already done are
# skipped, so it is safe to re-run. It never deletes keys: locking is deploy/lock.sh.
# Signing uses your near CLI keychain for x3lang.<network>.
set -euo pipefail
[ $# -eq 3 ] || { echo "usage: deploy/setup.sh testnet|mainnet <wasm> <expected sha256>" >&2; exit 1; }
case $2 in /*) WASM=$2 ;; *) WASM=$PWD/$2 ;; esac
cd "$(dirname "$0")/.."
. deploy/lib.sh
set_network "$1"
EXPECTED_SHA=$3
need near jq curl openssl python3 sha256sum base64
LOG=$KEY_DIR/setup-$NETWORK.log
mkdir -p "$KEY_DIR"
chmod 700 "$KEY_DIR"
exec > >(tee -a "$LOG") 2>&1
echo "DX3HACK setup on $NETWORK, $(date -u +%FT%TZ)"

say "Preflight"
[ -f "$WASM" ] || die "wasm not found: $WASM"
actual=$(sha256sum "$WASM" | cut -d ' ' -f 1)
[ "$actual" = "$EXPECTED_SHA" ] || die "wasm sha256 is $actual, expected $EXPECTED_SHA"
echo "wasm sha256 ok: $actual ($(wc -c < "$WASM") bytes)"
if [ -f ../../../scripts/hackathon/check_guard_wasm_host_panics.py ]; then
  python3 -I ../../../scripts/hackathon/check_guard_wasm_host_panics.py --expect present "$WASM" >/dev/null \
    || die "wasm doesn't import env.panic_utf8: it was built without --cfg near"
  echo "refusal reasons reach the chain: ok"
fi
burn_sha=$(sha256sum x3/dx3hack_burn.x3 | cut -d ' ' -f 1)
[ "$(jq -r .burn_program_sha256 "$ARGS_FILE")" = "$burn_sha" ] || die "$ARGS_FILE doesn't bind x3/dx3hack_burn.x3"
for pair in "owner_id:$OWNER" "faucet_operator_id:$FAUCET" "holder_parent_id:$JUDGES"; do
  [ "$(jq -r ".${pair%%:*}" "$ARGS_FILE")" = "${pair#*:}" ] || die "$ARGS_FILE ${pair%%:*} isn't ${pair#*:}"
done
expires=$(jq -r .expires_at "$ARGS_FILE")
now_ns=$(( $(date -u +%s) * 1000000000 ))
[ "$expires" -gt "$now_ns" ] || die "expires_at is in the past"
echo "burn program bound: $burn_sha"
echo "burns at: $(date -u -d "@$(( expires / 1000000000 ))" +%FT%TZ)"
account_exists "$OWNER" || die "owner $OWNER doesn't exist on $NETWORK"

say "Accounts"
create() {
  local account=$1 amount=$2 why=$3
  if account_exists "$account"; then echo "$account exists"; return; fi
  confirm "create $account ($why), funded by $OWNER" "$amount plus network fees"
  near account create-account fund-myself "$account" "$amount" \
    autogenerate-new-keypair save-to-keychain \
    sign-as "$OWNER" network-config "$NETWORK" sign-with-keychain send
  account_exists "$account" || die "$account still doesn't exist"
}
create "$TOKEN" '7 NEAR' 'the token contract; ~4.1 NEAR locks the 412 KB code, the rest stores up to 500 holders'
create "$FAUCET" '0.5 NEAR' 'faucet operator; gas for add_holder and faucet_claim'
create "$JUDGES" '2 NEAR' 'parent of every judge holder account'

say "Deploy and initialize"
if has_contract "$TOKEN"; then
  echo "$TOKEN already has code; skipping deploy"
else
  confirm "deploy $WASM to $TOKEN and call new with $ARGS_FILE" "storage stake from $TOKEN's 7 NEAR, plus gas"
  near contract deploy "$TOKEN" use-file "$WASM" \
    with-init-call new file-args "$ARGS_FILE" \
    prepaid-gas '100 Tgas' attached-deposit '0 NEAR' \
    network-config "$NETWORK" sign-with-keychain send
fi

say "Verify stored terms against $ARGS_FILE"
cfg=$(view "$TOKEN" hackathon_config '{}') || die "hackathon_config view failed"
meta=$(view "$TOKEN" ft_metadata '{}') || die "ft_metadata view failed"
for k in owner_id faucet_operator_id holder_parent_id expires_at burn_program_sha256 max_supply claim_amount \
         claim_cooldown_sec faucet_daily_cap max_transfers_per_day max_holders; do
  want=$(jq -r ".$k" "$ARGS_FILE"); got=$(jq -r ".$k" <<<"$cfg")
  [ "$want" = "$got" ] || die "stored $k is $got, expected $want"
done
[ "$(jq -r .symbol <<<"$meta")" = "DX3HACK" ] || die "unexpected symbol"
[ "$(jq -r .name <<<"$meta")" = "Dotx3 HACKATHON ONLY COIN" ] || die "unexpected name"
echo "stored terms match; active=$(jq -r .active <<<"$cfg")"

say "Faucet operator key (function calls on $TOKEN only)"
if [ -f "$FAUCET_KEY_FILE" ]; then
  pub=$(jq -r .public_key "$FAUCET_KEY_FILE")
  echo "using existing key file $FAUCET_KEY_FILE"
else
  pub=$(make_key_file "$FAUCET_KEY_FILE" "$FAUCET")
  echo "wrote $FAUCET_KEY_FILE (mode 600)"
fi
keys=$(access_keys "$FAUCET")
if jq -e --arg p "$pub" '.keys[] | select(.public_key == $p)' >/dev/null <<<"$keys"; then
  echo "key $pub already on $FAUCET"
else
  confirm "add $pub to $FAUCET, limited to add_holder and faucet_claim on $TOKEN" "0.5 NEAR gas allowance, plus fees"
  near account add-key "$FAUCET" \
    grant-function-call-access --allowance '0.5 NEAR' --contract-account-id "$TOKEN" \
    --function-names 'add_holder, faucet_claim' \
    use-manually-provided-public-key "$pub" \
    network-config "$NETWORK" sign-with-keychain send
fi
perm=$(access_keys "$FAUCET" | jq -c --arg p "$pub" '.keys[] | select(.public_key == $p) | .access_key.permission')
[ "$(jq -r '.FunctionCall.receiver_id' <<<"$perm")" = "$TOKEN" ] || die "faucet key isn't scoped to $TOKEN: $perm"
[ "$(jq -c '.FunctionCall.method_names | sort' <<<"$perm")" = '["add_holder","faucet_claim"]' ] || die "faucet key methods: $perm"
echo "faucet key scoped: $perm"

if [ -n "${DX3HACK_EXTRA_HOLDERS:-}" ]; then
  say "Owner-approved holders"
  for h in $DX3HACK_EXTRA_HOLDERS; do
    if [ "$(view "$TOKEN" is_holder "$(jq -nc --arg a "$h" '{account_id:$a}')")" = "true" ]; then
      echo "$h is already a holder"; continue
    fi
    confirm "allowlist $h as a DX3HACK holder" "gas only; the contract pays the holder's storage"
    near contract call-function as-transaction "$TOKEN" add_holder \
      json-args "$(jq -nc --arg a "$h" '{account_id:$a}')" \
      prepaid-gas '30 Tgas' attached-deposit '0 NEAR' \
      sign-as "$OWNER" network-config "$NETWORK" sign-with-keychain send
  done
fi

say "Done"
echo "Token:        $EXPLORER/address/$TOKEN"
echo "Faucet key:   $FAUCET_KEY_FILE  (copy to the runner; never commit it)"
echo "Log:          $LOG"
echo "Next:         deploy/smoke.sh $NETWORK   (records on-chain refusals with the faucet key)"
echo "Then, only after you've checked every rule: deploy/lock.sh $NETWORK"

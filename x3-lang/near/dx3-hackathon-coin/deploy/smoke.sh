#!/usr/bin/env bash
# On-chain smoke test for a deployed DX3HACK, signed with the faucet operator's scoped key.
#
#   deploy/smoke.sh testnet
#   deploy/smoke.sh mainnet
#
# Expected successes: add a judge holder, one faucet claim.
# Expected refusals: a second claim inside the cooldown, a lookalike parent account, a
# nested subaccount, and a DEX account. Each refusal is a real transaction; the script
# checks every outcome against NEAR RPC and writes the hashes to
# $KEY_DIR/smoke-<network>.json. Costs only gas from the faucet key's allowance.
set -euo pipefail
cd "$(dirname "$0")/.."
. deploy/lib.sh

[ $# -eq 1 ] || die "usage: deploy/smoke.sh testnet|mainnet"
set_network "$1"
need near jq curl python3 base64
[ -f "$FAUCET_KEY_FILE" ] || die "no faucet key at $FAUCET_KEY_FILE; run deploy/setup.sh first"
OUT=$KEY_DIR/smoke-$NETWORK.json
RUN=$(date -u +%Y%m%d%H%M%S)
JUDGE=s$RUN.$JUDGES
case $NETWORK in testnet) DEX=v2.ref-finance.testnet ;; mainnet) DEX=v2.ref-finance.near ;; esac

has_contract "$TOKEN" || die "$TOKEN has no contract on $NETWORK"
[ "$(view "$TOKEN" hackathon_config '{}' | jq -r .active)" = "true" ] || die "DX3HACK isn't active"

confirm "run 6 faucet-key transactions on $TOKEN (2 should pass, 4 should be refused)" "gas only, from the faucet key's allowance"

results='[]'
# call METHOD JSON_ARGS EXPECT(ok|refused) EXPECTED_REASON LABEL
# The near CLI only signs (save-to-file). This script broadcasts with send_tx and reads the
# outcome itself, because the CLI doesn't print the transaction ID when a call is refused.
call() {
  local method=$1 args=$2 expect=$3 reason=$4 label=$5 signed b64 status hash outcome msg
  signed=$(mktemp)
  rm -f "$signed"
  near contract call-function as-transaction "$TOKEN" "$method" json-args "$args" \
    prepaid-gas '30 Tgas' attached-deposit '0 NEAR' \
    sign-as "$FAUCET" network-config "$NETWORK" \
    sign-with-access-key-file "$FAUCET_KEY_FILE" save-to-file "$signed" >/dev/null 2>&1 \
    || die "$label: the near CLI couldn't sign the transaction"
  b64=$(jq -r .signed_transaction_as_base64 "$signed")
  rm -f "$signed"
  [ -n "$b64" ] && [ "$b64" != null ] || die "$label: no signed transaction in the CLI output file"
  status=$(rpc "$(jq -nc --arg t "$b64" '{method:"send_tx",params:{signed_tx_base64:$t,wait_until:"FINAL"}}')") \
    || die "$label: RPC refused the transaction before execution"
  hash=$(jq -r .transaction.hash <<<"$status")
  if jq -e '.status | has("SuccessValue")' >/dev/null <<<"$status"; then outcome=ok; msg=""
  else outcome=refused; msg=$(jq -r '[.receipts_outcome[].outcome.status.Failure // empty | tostring] | join(" ")' <<<"$status"); fi
  printf '%-44s %-8s %s\n' "$label" "$outcome" "$EXPLORER/txns/$hash"
  [ "$outcome" = "$expect" ] || die "$label: expected $expect, got $outcome ($msg)"
  if [ "$expect" = refused ]; then
    grep -qF "$reason" <<<"$msg" || die "$label: refused for the wrong reason: $msg"
  fi
  results=$(jq -c --arg l "$label" --arg m "$method" --arg a "$args" --arg h "$hash" --arg o "$outcome" --arg r "$msg" \
    --arg u "$EXPLORER/txns/$hash" '. + [{label:$l, method:$m, args:($a|fromjson), tx_hash:$h, outcome:$o, reason:$r, url:$u}]' <<<"$results")
}

say "Smoke test on $NETWORK as $FAUCET"
call add_holder   "$(jq -nc --arg a "$JUDGE" '{account_id:$a}')" ok "" "add judge holder $JUDGE"
call faucet_claim "$(jq -nc --arg a "$JUDGE" '{receiver_id:$a}')" ok "" "faucet claim to $JUDGE"
call faucet_claim "$(jq -nc --arg a "$JUDGE" '{receiver_id:$a}')" refused "claimed recently" "second claim inside cooldown"
call add_holder   "$(jq -nc --arg a "evil-judges.x3lang.$SUFFIX" '{account_id:$a}')" refused "only add direct subaccounts" "lookalike parent account"
call add_holder   "$(jq -nc --arg a "pool.$JUDGE" '{account_id:$a}')" refused "only add direct subaccounts" "nested subaccount"
call add_holder   "$(jq -nc --arg a "$DEX" '{account_id:$a}')" refused "only add direct subaccounts" "DEX account $DEX"

bal=$(view "$TOKEN" ft_balance_of "$(jq -nc --arg a "$JUDGE" '{account_id:$a}')" | tr -d '"')
[ "$bal" = "$(jq -r .claim_amount "$ARGS_FILE")" ] || die "$JUDGE balance is $bal, expected one claim"
echo "balance of $JUDGE: $bal (one claim)"

jq -n --arg n "$NETWORK" --arg t "$TOKEN" --arg at "$(date -u +%FT%TZ)" --argjson r "$results" \
  '{network:$n, token:$t, ran_at:$at, results:$r}' > "$OUT"
say "All 6 outcomes matched. Evidence: $OUT"

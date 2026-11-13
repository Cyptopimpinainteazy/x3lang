#!/usr/bin/env bash
# agent-demo.sh: one guided AI Money Agent demonstration, step by step, with confirmation
# before every NEAR transaction. This script never moves USDC; the owner's own Solana
# wallet performs the deposit after the live quote is accepted.
#
#   x3-lang/near/agent-demo.sh setup                   write state and the owner's mandate
#   x3-lang/near/agent-demo.sh mandate                 owner creates the mandate on x3-guard
#   x3-lang/near/agent-demo.sh pay-dry                 real model writes and checks a payment
#   x3-lang/near/agent-demo.sh pay-live <sha256>       owner-approved live quote, exact dry program
#   x3-lang/near/agent-demo.sh register                agent registers + accepts on x3-guard
#   x3-lang/near/agent-demo.sh deposit <sig> [refund]  verify the owner's deposit and outcome
#   x3-lang/near/agent-demo.sh refuse                  malicious register call fails on chain
#   x3-lang/near/agent-demo.sh revoke                  owner revokes the mandate
#   x3-lang/near/agent-demo.sh post-revoke             valid call fails after revocation
#   x3-lang/near/agent-demo.sh show                    state, hashes and explorer links
#
# Set X3_AGENT_PROVIDER / ANTHROPIC_API_KEY (or an OpenAI-compatible provider) before
# pay-dry. Set X3_AGENT_PREPAID_STORAGE=1 explicitly for a reviewed Guard with
# owner-prepaid storage support and sufficient owner-funded credit (neither is verified here).
# This selects zero deposits for agent calls; owner funding and approval calls are unchanged.
# Keys and state stay in $X3_AGENT_RUN_DIR (default ~/.x3/agent-demo), modes 700/600.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
X3_LANG="$(cd "$HERE/.." && pwd)"
ADAPTER="$X3_LANG/near_intents.py"
AGENT="$X3_LANG/near_agent.py"
RUN_DIR="${X3_AGENT_RUN_DIR:-$HOME/.x3/agent-demo}"
STATE="$RUN_DIR/state.env"
MANDATE="$RUN_DIR/near_mandate_agent.x3"
ALLOWED="$RUN_DIR/agent_allowed_payment.x3"
DRY_REPORT="$RUN_DIR/agent-dry-run.json"
REFUSED="$RUN_DIR/agent_refused_payment.x3"
REFUSED_ARGS="$RUN_DIR/agent-refused-register.json"
REFUSED_LOG="$RUN_DIR/agent-refused-onchain.log"
POST_REVOKED="$RUN_DIR/agent_post_revoke_payment.x3"
POST_REVOKED_ARGS="$RUN_DIR/agent-post-revoke-register.json"
POST_REVOKED_LOG="$RUN_DIR/agent-post-revoke-onchain.log"
RECEIPT="$RUN_DIR/agent_payment.near-receipt.json"
NETWORK="mainnet"

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\n\033[31mStopped: %s\033[0m\n' "$*" >&2; exit 1; }

AGENT_STORAGE_ARGS=()
case "${X3_AGENT_PREPAID_STORAGE:-0}" in
  0) ;;
  1) AGENT_STORAGE_ARGS=(--prepaid-agent-storage) ;;
  *) die "X3_AGENT_PREPAID_STORAGE must be 0 or 1; support and owner credit are not auto-detected." ;;
esac

confirm() {
  local answer
  read -rp "  $1 Type yes to continue: " answer
  [[ "$answer" == "yes" ]] || die "not confirmed; nothing was sent."
}

load_state() {
  [[ -f "$STATE" ]] || die "run '$0 setup' first."
  # shellcheck source=/dev/null
  source "$STATE"
}

adapter() { python3 "$ADAPTER" "$@"; }
agent()   { python3 "$AGENT" "$@"; }

json_get() {
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1]));
for k in sys.argv[2].split("."): d=d[k]
print(d)' "$1" "$2"
}

guard_cmd() {
  adapter guard-args --receipt "$RECEIPT" --step "$1" --contract "$CONTRACT" --signer "$AGENT_ACCOUNT" \
    --key-file "$AGENT_KEY" --network "$NETWORK" "${AGENT_STORAGE_ARGS[@]}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])'
}

send_expected_refusal() {
  local command="$1" log="$2" label="$3" code
  confirm "$label This sends one NEAR transaction that is expected to fail."
  set +e
  bash -c "$command" >"$log" 2>&1
  code=$?
  set -e
  chmod 600 "$log"
  (( code != 0 )) || die "$label unexpectedly succeeded; inspect the contract and do not continue."
  grep -q 'X3_MANDATE_DENIED' "$log" || {
    tail -80 "$log" >&2
    die "$label failed without the expected X3_MANDATE_DENIED evidence."
  }
  info "Confirmed: x3-guard refused the transaction with X3_MANDATE_DENIED."
  info "Transaction output: $log"
}

cmd_setup() {
  mkdir -p "$RUN_DIR"
  chmod 700 "$RUN_DIR"
  say "Setting up the AI Money Agent demo"

  local owner agent_account owner_key agent_key contract recipient sol amount units min_receive
  read -rp "  Owner NEAR account: " owner
  [[ -n "$owner" ]] || die "an owner account is required."
  read -rp "  Owner key file: " owner_key
  [[ -f "$owner_key" ]] || die "owner key file not found: $owner_key"
  read -rp "  Agent NEAR account: " agent_account
  [[ -n "$agent_account" ]] || die "an agent account is required."
  [[ "$agent_account" != "$owner" ]] || die "the agent must use a different NEAR account."
  read -rp "  Agent key file: " agent_key
  [[ -f "$agent_key" ]] || die "agent key file not found: $agent_key"
  read -rp "  x3-guard contract account [${owner}]: " contract
  contract="${contract:-$owner}"
  read -rp "  Recipient NEAR account allowed by the mandate [x3-demo.near]: " recipient
  recipient="${recipient:-x3-demo.near}"
  read -rp "  Owner's Solana wallet (sends USDC, receives refunds): " sol
  [[ "$sol" =~ ^[1-9A-HJ-NP-Za-km-z]{32,44}$ ]] || die "that is not a Solana address."
  read -rp "  Payment amount in USDC [25]: " amount
  amount="${amount:-25}"
  [[ "$amount" =~ ^[0-9]+(\.[0-9]{1,6})?$ ]] || die "amount must be a number with at most 6 decimals."
  units="$(python3 -c 'import decimal,sys; print(int(decimal.Decimal(sys.argv[1])*1000000))' "$amount")"
  (( units >= 25000000 )) || die "use at least 25 USDC: 1Click's roughly 0.25 USDC refund fee otherwise exceeds 100 bps."
  min_receive=$(( units * 99 / 100 ))

  {
    printf '// Owner mandate for the AI Money Agent demo.\n'
    printf 'mandate x3_shopping_agent {\n'
    printf '    owner %s\n' "$owner"
    printf '    agent %s\n' "$agent_account"
    printf '    spend Solana.USDC\n'
    printf '    per_payment %s\n' "$units"
    printf '    per_24h %s\n' "$(( units * 2 ))"
    printf '    allow recipients %s\n' "$recipient"
    printf '    allow assets Near.USDC\n'
    printf '    refund_to %s\n' "$sol"
    printf '    require slippage <= 1%%\n'
    printf '    require fees <= 100 bps\n'
    printf '    expires 30d\n'
    printf '}\n'
  } >"$MANDATE"
  {
    printf 'OWNER=%q\n' "$owner"
    printf 'OWNER_KEY=%q\n' "$owner_key"
    printf 'AGENT_ACCOUNT=%q\n' "$agent_account"
    printf 'AGENT_KEY=%q\n' "$agent_key"
    printf 'CONTRACT=%q\n' "$contract"
    printf 'RECIPIENT=%q\n' "$recipient"
    printf 'SOLANA=%q\n' "$sol"
    printf 'AMOUNT=%q\n' "$amount"
    printf 'UNITS=%q\n' "$units"
    printf 'MIN_RECEIVE=%q\n' "$min_receive"
  } >"$STATE"
  chmod 600 "$STATE" "$MANDATE"
  info "Mandate written to $MANDATE"
  info "Key files are referenced by path only; they were not copied or printed."
  info "Next: $0 mandate"
}

cmd_mandate() {
  load_state
  say "Create the mandate on x3-guard"
  adapter mandate args "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK"
  confirm "This sends the owner's create_mandate transaction."
  bash -c "$(adapter mandate args "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])')"
  info "Next: $0 pay-dry"
}

cmd_pay_dry() {
  load_state
  say "Real model: English request -> X3 intent -> deterministic checks"
  agent pay "Pay ${RECIPIENT} ${AMOUNT} USDC for invoice 4411." --mandate "$MANDATE" \
    --out "$ALLOWED" --quote "${AGENT_STORAGE_ARGS[@]}" >"$DRY_REPORT"
  chmod 600 "$ALLOWED" "$DRY_REPORT"
  local sha provider model response_id
  sha="$(json_get "$DRY_REPORT" program.sha256)"
  provider="$(json_get "$DRY_REPORT" model.provider)"
  model="$(json_get "$DRY_REPORT" model.model)"
  response_id="$(json_get "$DRY_REPORT" model.id)"
  info "Provider:  $provider"
  info "Model:     $model"
  info "Response:  $response_id"
  info "Program:   $ALLOWED"
  info "SHA-256:   $sha"
  info "Dry run:   $DRY_REPORT"
  info "Next: $0 pay-live $sha"
}

cmd_pay_live() {
  load_state
  local sha="${1:-}" dry_sha issued_nonce
  [[ ! -e "$RECEIPT" ]] || die "this run already has a live receipt. Review the existing quote and deposit before starting another run."
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || die "give the program SHA-256 from pay-dry."
  [[ -f "$DRY_REPORT" && -f "$ALLOWED" ]] || die "run '$0 pay-dry' first."
  dry_sha="$(json_get "$DRY_REPORT" program.sha256)"
  issued_nonce="$(json_get "$DRY_REPORT" nonce)"
  [[ "$sha" == "$dry_sha" ]] || die "that SHA-256 is not the program produced by the recorded dry run."
  say "Owner-approved live quote"
  info "Reusing the exact dry-run program and issued nonce; the model is not called again."
  agent pay --program "$ALLOWED" --issued-nonce "$issued_nonce" --mandate "$MANDATE" \
    --guard-contract "$CONTRACT" --live --approve "$sha" \
    --nonce-store "$RUN_DIR/nonces.json" --receipt "$RECEIPT" "${AGENT_STORAGE_ARGS[@]}" >"$RUN_DIR/live-quote.json"
  chmod 600 "$RECEIPT" "$RUN_DIR/live-quote.json"
  info "Live quote written. Next: $0 register"
}

cmd_register() {
  load_state
  [[ -f "$RECEIPT" ]] || die "run '$0 pay-live <sha>' first."
  say "Agent registers and accepts the live quote on x3-guard"
  confirm "Two agent transactions to $CONTRACT."
  bash -c "$(guard_cmd register)"
  bash -c "$(guard_cmd quote)"
  info "On-chain acceptance complete. The owner now deposits from their own Solana wallet."
  info "Then run: $0 deposit <solana-tx-signature>"
}

cmd_deposit() {
  load_state
  local sig="${1:-}" refund_sig="${2:-}" address recorded status_code=0 verify_code=0
  [[ -f "$RECEIPT" ]] || die "run '$0 pay-live <sha>' and '$0 register' first."
  [[ "$sig" =~ ^[1-9A-HJ-NP-Za-km-z]{64,90}$ ]] || die "give the owner's Solana transaction signature."
  address="$(json_get "$RECEIPT" deposit.address)"
  recorded="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["deposit"].get("tx_hash") or "")' "$RECEIPT")"
  if [[ -z "$recorded" ]]; then
    adapter submit-deposit --deposit-address "$address" --tx-hash "$sig" --receipt "$RECEIPT"
  elif [[ "$recorded" != "$sig" ]]; then
    die "this receipt is already tracking a different signature; check the original transfer."
  fi
  say "Waiting for 1Click's terminal status"
  adapter status --deposit-address "$address" --wait --interval 10 --max-wait 1800 --receipt "$RECEIPT" \
    || status_code=$?
  case "$status_code" in
    0) info "1Click reports SUCCESS; independent chain verification is required." ;;
    2) info "1Click reports: refunded" ;;
    3) die "1Click reports: failed. Do not record an outcome." ;;
    *) die "the swap has no confirmed terminal outcome. Resume with the same signature. Do not deposit again." ;;
  esac

  adapter verify-deposit --receipt "$RECEIPT" || die "the deposit is not proven yet; wait and rerun."
  if [[ "$status_code" == "0" ]]; then
    die "Run scripts/hackathon/verify_mainnet_payment.py with the reviewed quote hash and public wallet and payout signer. This script cannot record a settled Guard outcome."
  fi
  if [[ -z "$refund_sig" ]]; then
    read -rp "  Solana signature of the refund transaction: " refund_sig
  fi
  adapter verify-settlement --receipt "$RECEIPT" --refund-tx "$refund_sig" || verify_code=$?
  [[ "$verify_code" == "2" ]] || die "the chain proof does not match the refund status."
  info "Source-chain refund-sized transfer verified; solver causality is not proven."

  say "Record the verified outcome on x3-guard"
  confirm "One agent transaction to $CONTRACT."
  bash -c "$(guard_cmd outcome)"
  info "Next: $0 refuse"
}

cmd_refuse() {
  load_state
  {
    printf 'intent refused_recipient {\n'
    printf '    from Solana.USDC amount %s receiver %s\n' "$UNITS" "$SOLANA"
    printf '    to Near.USDC receiver attacker.near\n'
    printf '    route {\n'
    printf '        bridge NearIntents Solana.USDC -> Near.USDC min_receive %s;\n' "$MIN_RECEIVE"
    printf '    }\n'
    printf '    under mandate %s\n' "$OWNER"
    printf '    require slippage <= 0.5%%\n'
    printf '    require fees <= 100 bps\n'
    printf '    require nonce unused agent_refused_001\n'
    printf '    timeout 10m refund Solana.USDC\n'
    printf '    on_fail rollback\n'
    printf '}\n'
  } >"$REFUSED"
  chmod 600 "$REFUSED"
  say "Submit an out-of-mandate program; x3-guard must refuse it on chain"
  agent register-args "$REFUSED" --mandate "$MANDATE" --contract "$CONTRACT" \
    --signer "$AGENT_ACCOUNT" --key-file "$AGENT_KEY" --network "$NETWORK" "${AGENT_STORAGE_ARGS[@]}" >"$REFUSED_ARGS"
  chmod 600 "$REFUSED_ARGS"
  info "Expected policy failure: mandate_recipient (attacker.near is not allowed)."
  send_expected_refusal "$(json_get "$REFUSED_ARGS" command)" "$REFUSED_LOG" "Refused-payment call."
  info "Next: $0 revoke"
}

cmd_revoke() {
  load_state
  say "Owner revokes the mandate"
  adapter mandate revoke "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK"
  confirm "This sends the owner's revoke_mandate transaction."
  bash -c "$(adapter mandate revoke "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])')"
  info "Next: $0 post-revoke"
}

cmd_post_revoke() {
  load_state
  local nonce="agent_post_revoke_$(date -u +%Y%m%dT%H%M%S)"
  {
    printf 'intent post_revoke_payment {\n'
    printf '    from Solana.USDC amount %s receiver %s\n' "$UNITS" "$SOLANA"
    printf '    to Near.USDC receiver %s\n' "$RECIPIENT"
    printf '    route {\n'
    printf '        bridge NearIntents Solana.USDC -> Near.USDC min_receive %s;\n' "$MIN_RECEIVE"
    printf '    }\n'
    printf '    under mandate %s\n' "$OWNER"
    printf '    require slippage <= 0.5%%\n'
    printf '    require fees <= 100 bps\n'
    printf '    require nonce unused %s\n' "$nonce"
    printf '    timeout 10m refund Solana.USDC\n'
    printf '    on_fail rollback\n'
    printf '}\n'
  } >"$POST_REVOKED"
  chmod 600 "$POST_REVOKED"
  say "Show that a previously valid agent is refused after owner revocation"
  agent register-args "$POST_REVOKED" --mandate "$MANDATE" --contract "$CONTRACT" \
    --signer "$AGENT_ACCOUNT" --key-file "$AGENT_KEY" --network "$NETWORK" "${AGENT_STORAGE_ARGS[@]}" >"$POST_REVOKED_ARGS"
  chmod 600 "$POST_REVOKED_ARGS"
  info "Expected policy failure: mandate_active."
  send_expected_refusal "$(json_get "$POST_REVOKED_ARGS" command)" "$POST_REVOKED_LOG" "Post-revocation call."
  info "Next: $0 show"
}

cmd_show() {
  load_state
  say "Agent demo state"
  info "Mandate:  $MANDATE"
  info "Allowed:  $ALLOWED"
  info "Refused:  $REFUSED"
  info "Receipt:  $RECEIPT"
  info "Contract: https://nearblocks.io/address/$CONTRACT"
  if [[ -f "$DRY_REPORT" ]]; then
    python3 - "$DRY_REPORT" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
m = r.get("model") or {}
print("  Provider:", m.get("provider"))
print("  Model:   ", m.get("model"))
print("  Response:", m.get("id"))
print("  Program: ", (r.get("program") or {}).get("sha256"))
PY
  fi
  [[ -f "$RECEIPT" ]] && info "Quote hash: $(json_get "$RECEIPT" quote_hash)"
  [[ -f "$REFUSED_LOG" ]] && info "Refused on chain: $REFUSED_LOG"
  [[ -f "$POST_REVOKED_LOG" ]] && info "Refused after revocation: $POST_REVOKED_LOG"
}

case "${1:-}" in
  setup) cmd_setup ;;
  mandate) cmd_mandate ;;
  pay-dry) cmd_pay_dry ;;
  pay-live) shift; cmd_pay_live "${1:-}" ;;
  register) cmd_register ;;
  deposit) shift; cmd_deposit "${1:-}" "${2:-}" ;;
  refuse) cmd_refuse ;;
  revoke) cmd_revoke ;;
  post-revoke) cmd_post_revoke ;;
  show) cmd_show ;;
  *) sed -n '2,19p' "$0"; exit 1 ;;
esac

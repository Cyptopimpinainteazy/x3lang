#!/usr/bin/env bash
# agent-demo.sh: one guided AI Money Agent demonstration, step by step, with confirmation
# before every NEAR transaction. This script never moves USDC; the owner's own Solana
# wallet performs the deposit after the live quote is accepted.
#
#   x3-lang/near/agent-demo.sh setup            write state and example programs (asks for values)
#   x3-lang/near/agent-demo.sh mandate          create the agent mandate on x3-guard
#   x3-lang/near/agent-demo.sh pay-dry          a real model writes and checks one allowed payment
#   x3-lang/near/agent-demo.sh pay-live <sha>   owner-approved live quote for the allowed program
#   x3-lang/near/agent-demo.sh register         agent registers + accepts the live quote on NEAR
#   x3-lang/near/agent-demo.sh deposit <tx-sig> [refund-tx-sig] track and verify settlement or refund
#   x3-lang/near/agent-demo.sh refuse           send a refused program; x3-guard fails it on chain
#   x3-lang/near/agent-demo.sh revoke           owner revokes the mandate
#   x3-lang/near/agent-demo.sh show             where everything is
#
# Set X3_AGENT_PROVIDER / ANTHROPIC_API_KEY (or an OpenAI-compatible provider) before
# pay-dry and pay-live. Keys and state are stored in $X3_AGENT_RUN_DIR with modes 700/600.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
X3_LANG="$(cd "$HERE/.." && pwd)"
ADAPTER="$X3_LANG/near_intents.py"
AGENT="$X3_LANG/near_agent.py"
RUN_DIR="${X3_AGENT_RUN_DIR:-$HOME/.x3/agent-demo}"
STATE="$RUN_DIR/state.env"
MANDATE="$RUN_DIR/near_mandate_agent.x3"
ALLOWED="$RUN_DIR/agent_allowed_payment.x3"
REFUSED="$RUN_DIR/agent_refused_payment.x3"
RECEIPT="$RUN_DIR/agent_payment.near-receipt.json"
NETWORK="mainnet"

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\n\033[31mStopped: %s\033[0m\n' "$*" >&2; exit 1; }

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

guard_cmd() {
  adapter guard-args --receipt "$RECEIPT" --step "$1" --contract "$CONTRACT" --signer "$AGENT_ACCOUNT" \
    --key-file "$AGENT_KEY" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])'
}

cmd_setup() {
  mkdir -p "$RUN_DIR"
  chmod 700 "$RUN_DIR"
  say "Setting up the AI Money Agent demo"

  local owner agent_account owner_key agent_key contract sol amount
  read -rp "  Owner NEAR account: " owner
  [[ -n "$owner" ]] || die "an owner account is required."
  read -rp "  Owner key file: " owner_key
  [[ -f "$owner_key" ]] || die "owner key file not found: $owner_key"
  read -rp "  Agent NEAR account: " agent_account
  [[ -n "$agent_account" ]] || die "an agent account is required."
  read -rp "  Agent key file: " agent_key
  [[ -f "$agent_key" ]] || die "agent key file not found: $agent_key"
  read -rp "  x3-guard contract account [${owner}]: " contract
  contract="${contract:-$owner}"
  read -rp "  Owner's Solana wallet (sends USDC, receives refunds): " sol
  [[ "$sol" =~ ^[1-9A-HJ-NP-Za-km-z]{32,44}$ ]] || die "that is not a Solana address."
  read -rp "  Payment amount in USDC [25]: " amount
  amount="${amount:-25}"
  [[ "$amount" =~ ^[1-9][0-9]*$ ]] || die "amount must be a positive whole-number USDC amount (no decimals)"

  cat > "$MANDATE" <<EOF
// Owner mandate for the AI Money Agent demo. Replace accounts before running.
mandate x3_shopping_agent {
    owner ${owner}
    agent ${agent_account}
    spend Solana.USDC
    per_payment ${amount}000000
    per_24h ${amount}000000
    allow recipients x3-demo.near
    allow assets Near.USDC
    refund_to ${sol}
    require slippage <= 1%
    require fees <= 100 bps
    expires 30d
}
EOF
  {
    printf 'OWNER=%q\n' "$owner"
    printf 'OWNER_KEY=%q\n' "$owner_key"
    printf 'AGENT_ACCOUNT=%q\n' "$agent_account"
    printf 'AGENT_KEY=%q\n' "$agent_key"
    printf 'CONTRACT=%q\n' "$contract"
    printf 'SOLANA=%q\n' "$sol"
    printf 'AMOUNT=%q\n' "$amount"
  } > "$STATE"
  chmod 600 "$STATE"
  info "Mandate written to $MANDATE"
  info "Next: $0 mandate"
}

cmd_mandate() {
  load_state
  say "Create the mandate on x3-guard"
  adapter mandate args "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK"
  confirm "This sends the owner's create_mandate transaction."
  bash -c "$(adapter mandate args "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])')"
}

cmd_pay_dry() {
  load_state
  say "Real model: English request -> X3 intent -> deterministic checks"
  agent pay "Pay x3-demo.near ${AMOUNT} USDC for invoice 4411." --mandate "$MANDATE" \
    --out "$ALLOWED" --quote
  info "Approval hash is printed above as --approve <sha256>."
  info "Next: $0 pay-live <sha256>"
}

cmd_pay_live() {
  load_state
  local sha="${1:-}"
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || die "give the program SHA-256 from pay-dry."
  say "Owner-approved live quote"
  agent pay "Pay x3-demo.near ${AMOUNT} USDC for invoice 4411." --mandate "$MANDATE" \
    --out "$ALLOWED" --approved-program "$ALLOWED" --guard-contract "$CONTRACT" --live --approve "$sha" \
    --nonce-store "$RUN_DIR/nonces.json" --receipt "$RECEIPT" > "$RUN_DIR/live-quote.json"
  chmod 600 "$RECEIPT"
  info "Live quote written. Next: $0 register"
}

cmd_register() {
  load_state
  [[ -f "$RECEIPT" ]] || die "run '$0 pay-live <sha>' first."
  say "Agent registers and accepts the live quote on x3-guard"
  confirm "Two agent transactions to $CONTRACT."
  bash -c "$(guard_cmd register)"
  bash -c "$(guard_cmd quote)"
  info "On-chain acceptance complete. Owner deposits next."
  info "Run: $0 deposit <solana-tx-signature>"
}

cmd_deposit() {
  load_state
  local sig="${1:-}" refund_sig="${2:-}"
  [[ "$sig" =~ ^[1-9A-HJ-NP-Za-km-z]{64,90}$ ]] || die "give the Solana transaction signature."
  [[ -f "$RECEIPT" ]] || die "run '$0 pay-live <sha>' first."
  local deposit_address
  deposit_address="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["deposit"]["address"])' "$RECEIPT")"
  adapter submit-deposit --deposit-address "$deposit_address" --tx-hash "$sig" --receipt "$RECEIPT"

  local status_code=0
  adapter status --deposit-address "$deposit_address" --wait --interval 10 --max-wait 1800 \
    --receipt "$RECEIPT" || status_code=$?
  case "$status_code" in
    0) info "1Click reports settled; checking independent chain evidence." ;;
    2) info "1Click reports refunded; independent refund proof is required." ;;
    3) die "1Click reports failed. Do not record a successful on-chain outcome." ;;
    *) die "1Click outcome is not terminal (exit $status_code); retry after finality." ;;
  esac

  # Helius is optional by default but mandatory when explicitly requested.
  local helius_args=()
  if [[ "${X3_REQUIRE_HELIUS:-0}" == "1" ]]; then
    helius_args=(--helius)
  fi
  # Never record the on-chain outcome until the source deposit is proven.
  adapter verify-deposit --receipt "$RECEIPT" "${helius_args[@]}" || die "Solana deposit is not independently verified."

  local proof_code=0
  if [[ "$status_code" == 2 ]]; then
    if [[ -z "$refund_sig" ]]; then
      read -rp "  Finalized Solana refund transaction signature: " refund_sig
    fi
    [[ "$refund_sig" =~ ^[1-9A-HJ-NP-Za-km-z]{64,90}$ ]] || die "a valid Solana refund signature is required."
    adapter verify-settlement --receipt "$RECEIPT" --refund-tx "$refund_sig" || proof_code=$?
    [[ "$proof_code" == 2 ]] || die "Refund is not independently proven; no outcome was recorded."
  else
    adapter verify-settlement --receipt "$RECEIPT" || proof_code=$?
    [[ "$proof_code" == 0 ]] || die "NEAR delivery is not independently proven; no outcome was recorded."
  fi

  confirm "Record the verified $([[ "$status_code" == 2 ]] && echo refund || echo settlement) on NEAR."
  bash -c "$(guard_cmd outcome)"
  info "Outcome recorded only after source and terminal evidence checks."
}

cmd_refuse() {
  load_state
  cat > "$REFUSED" <<EOF
intent refused_recipient {
    from Solana.USDC amount ${AMOUNT}000000 receiver ${SOLANA}
    to Near.USDC receiver attacker.near
    route {
        bridge NearIntents Solana.USDC -> Near.USDC min_receive $((AMOUNT * 990000));
    }
    under mandate ${OWNER}
    require slippage <= 0.5%
    require fees <= 100 bps
    require nonce unused agent_refused_001
    timeout 10m refund Solana.USDC
    on_fail rollback
}
EOF
  say "Submit a refused program; x3-guard must reject it on chain"
  agent register-args "$REFUSED" --mandate "$MANDATE" --contract "$CONTRACT" \
    --signer "$AGENT_ACCOUNT" --key-file "$AGENT_KEY" --network "$NETWORK"
  confirm "Send the refused agent_register_intent transaction; it is expected to fail with X3_MANDATE_DENIED."
  bash -c "$(agent register-args "$REFUSED" --mandate "$MANDATE" --contract "$CONTRACT" \
    --signer "$AGENT_ACCOUNT" --key-file "$AGENT_KEY" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])')"
}

cmd_revoke() {
  load_state
  say "Owner revokes the mandate"
  adapter mandate revoke "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK"
  confirm "This sends the owner's revoke_mandate transaction."
  bash -c "$(adapter mandate revoke "$MANDATE" --contract "$CONTRACT" --key-file "$OWNER_KEY" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])')"
}

cmd_show() {
  load_state
  say "Agent demo state"
  info "Mandate:  $MANDATE"
  info "Allowed:  $ALLOWED"
  info "Refused:  $REFUSED"
  info "Receipt:  $RECEIPT"
  info "Contract: https://nearblocks.io/address/$CONTRACT"
  [[ -f "$RECEIPT" ]] && python3 -c 'import json; r=json.load(open(sys.argv[1])); print("  Quote hash:", r.get("quote_hash"))' "$RECEIPT"
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
  show) cmd_show ;;
  *) sed -n '2,15p' "$0"; exit 1 ;;
esac

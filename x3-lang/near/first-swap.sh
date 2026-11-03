#!/usr/bin/env bash
# first-swap.sh: one real, small NEAR Intents swap through X3, step by step, with proof at every step.
#
#   x3-lang/near/first-swap.sh setup               write the X3 program for this run (asks for each value)
#   x3-lang/near/first-swap.sh check               tools, NEAR balance, and a dry quote of the program
#   x3-lang/near/first-swap.sh build               build the x3-guard contract (cargo near)
#   x3-lang/near/first-swap.sh deploy              deploy x3-guard to your NEAR account     (costs NEAR)
#   x3-lang/near/first-swap.sh quote               live quote, then register + accept it on NEAR (costs NEAR)
#   x3-lang/near/first-swap.sh deposited <tx-sig>  after YOU send the USDC: track, verify from both chains, record
#   x3-lang/near/first-swap.sh show                where everything is, with explorer links
#
# This script never moves your USDC. The deposit is made by you, from your own Solana wallet,
# after `quote` prints the address and exact amount. Every step that spends NEAR asks first.
# State lives in $X3_RUN_DIR (default ~/.x3/first-swap).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
X3_LANG="$(cd "$HERE/.." && pwd)"
ADAPTER="$X3_LANG/near_intents.py"
GUARD_DIR="$HERE/x3-guard"
WASM="$GUARD_DIR/target/near/x3_guard.wasm"
RUN_DIR="${X3_RUN_DIR:-$HOME/.x3/first-swap}"
STATE="$RUN_DIR/state.env"
PROGRAM="$RUN_DIR/x3_first_swap.x3"
RECEIPT="$RUN_DIR/x3_first_swap.near-receipt.json"
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

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is not installed. $2"; }

adapter() { python3 "$ADAPTER" "$@"; }

json_get() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1]));
for k in sys.argv[2].split("."): d=d[k]
print(d)' "$1" "$2"; }

guard_cmd() {
  adapter guard-args --receipt "$RECEIPT" --step "$1" --contract "$CONTRACT" --signer "$ACCOUNT" \
    --key-file "$KEY_FILE" --network "$NETWORK" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["command"])'
}

near_view() {
  near contract call-function as-read-only "$CONTRACT" "$1" json-args "$2" network-config "$NETWORK" now
}

# ------------------------------------------------------------------------------------------

cmd_setup() {
  mkdir -p "$RUN_DIR"
  chmod 700 "$RUN_DIR"
  say "Setting up the first swap"

  local default_key="" default_account=""
  local keys=("$HOME"/.near-credentials/implicit/*.json)
  if [[ ${#keys[@]} -eq 1 && -f "${keys[0]}" ]]; then
    default_key="${keys[0]}"
    default_account="$(basename "$default_key" .json)"
  fi

  local account key_file contract sol amount
  read -rp "  Your NEAR account [${default_account:-none found}]: " account
  account="${account:-$default_account}"
  [[ -n "$account" ]] || die "a NEAR account is required."
  read -rp "  Its key file [${default_key:-~/.near-credentials/...}]: " key_file
  key_file="${key_file:-$default_key}"
  [[ -f "$key_file" ]] || die "key file not found: $key_file"
  read -rp "  Account that will hold the x3-guard contract [$account]: " contract
  contract="${contract:-$account}"
  read -rp "  Your Solana wallet address (sends the USDC, receives any refund): " sol
  [[ "$sol" =~ ^[1-9A-HJ-NP-Za-km-z]{32,44}$ ]] || die "that is not a Solana address."
  read -rp "  How much USDC to swap [10]: " amount
  amount="${amount:-10}"
  [[ "$amount" =~ ^[0-9]+(\.[0-9]{1,6})?$ ]] || die "amount must be a number with at most 6 decimals."

  # Base units (USDC has 6 decimals), a floor 1.5% under the input (USDC -> USDC), and a fee
  # cap that covers 1Click's ~0.25 USDC refund fee on small amounts: `require fees` bounds the
  # swap's cost and the refund path alike.
  local units min_receive fee_cap nonce
  units="$(python3 -c 'import sys,decimal; print(int(decimal.Decimal(sys.argv[1])*1000000))' "$amount")"
  (( units >= 1000000 )) || die "swap at least 1 USDC."
  min_receive=$(( units * 985 / 1000 ))
  fee_cap="$(python3 -c 'import sys,math; u=int(sys.argv[1]); print(max(100, math.ceil(300000*10000/u)))' "$units")"
  nonce="first_swap_$(date -u +%Y%m%d%H%M%S)"

  cat > "$PROGRAM" <<EOF
// The first real X3 swap through NEAR Intents: ${amount} USDC on Solana -> USDC on NEAR.
// Floor: 1.5% under the input. Fees: at most ${fee_cap} bps on the swap and on a refund.
intent x3_first_swap {
    from Solana.USDC amount ${units} receiver ${sol}
    to Near.USDC receiver ${account}
    route {
        bridge NearIntents Solana.USDC -> Near.USDC min_receive ${min_receive};
    }
    require slippage <= 0.5%
    require fees <= ${fee_cap} bps
    require nonce unused ${nonce}
    timeout 10m refund Solana.USDC
    on_fail rollback
}
EOF
  {
    printf 'ACCOUNT=%q\n' "$account"
    printf 'KEY_FILE=%q\n' "$key_file"
    printf 'CONTRACT=%q\n' "$contract"
    printf 'SOLANA=%q\n' "$sol"
    printf 'UNITS=%q\n' "$units"
  } > "$STATE"
  chmod 600 "$STATE"
  info "Program written to $PROGRAM"
  info "Next: $0 check"
}

cmd_check() {
  load_state
  say "Checking tools"
  need python3 ""
  need near "Install it: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/near/near-cli-rs/releases/latest/download/near-cli-rs-installer.sh | sh"
  need cargo "Install Rust: https://rustup.rs"
  cargo near --version >/dev/null 2>&1 || die "cargo-near is not installed. Install it: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/near/cargo-near/releases/latest/download/cargo-near-installer.sh | sh"
  info "python3, near, cargo, cargo-near: present"

  say "NEAR account $ACCOUNT"
  near account view-account-summary "$ACCOUNT" network-config "$NETWORK" now || die "could not read $ACCOUNT on $NETWORK; is it funded?"

  say "A dry quote of the program (nothing is signed or sent)"
  local out
  out="$(adapter quote "$PROGRAM")" || die "the dry quote was refused; the error is printed above."
  printf '%s' "$out" | python3 -c '
import json, sys
r = json.load(sys.stdin)
for g in r["guards"]:
    print("  " + ("ok  " if g["ok"] else "FAIL") + "  " + g["guard"])
q = r["quote_response"]["quote"]
print("  1Click guarantees at least", q["minAmountOut"], "of", r["leg"]["min_receive"], "required (base units)")'
  info "Next: $0 build"
}

cmd_build() {
  load_state
  say "Building the x3-guard contract"
  (cd "$GUARD_DIR" && cargo near build non-reproducible-wasm)
  [[ -f "$WASM" ]] || die "the build did not produce $WASM"
  local bytes
  bytes=$(wc -c < "$WASM")
  info "Built $WASM: $bytes bytes"
  info "Deploying it locks about $(python3 -c "print(round($bytes/100000*1.0 + 0.1, 2))") NEAR for storage."
  info "Next: $0 deploy"
}

cmd_deploy() {
  load_state
  [[ -f "$WASM" ]] || die "build the contract first: $0 build"
  say "Deploying x3-guard to $CONTRACT on $NETWORK"
  confirm "This deploys the contract and locks NEAR for its storage."
  near contract deploy "$CONTRACT" use-file "$WASM" with-init-call new json-args '{}' \
    prepaid-gas '30 Tgas' attached-deposit '0 NEAR' network-config "$NETWORK" \
    sign-with-access-key-file "$KEY_FILE" send
  say "The deployed contract trusts this 1Click key:"
  near_view get_manager_public_key '{}'
  info "Next: $0 quote"
}

cmd_quote() {
  load_state
  [[ ! -f "$RECEIPT" ]] || die "a receipt already exists at $RECEIPT; this run has quoted. Use '$0 setup' for a new run."
  say "Requesting a LIVE quote (this opens a deposit address; nothing is sent yet)"
  adapter quote "$PROGRAM" --live --receipt "$RECEIPT" --nonce-store "$RUN_DIR/nonces.json" > "$RUN_DIR/live-quote.json" \
    || die "the live quote was refused; see the error above. Nothing was registered."
  chmod 600 "$RECEIPT"
  info "Every guard passed. Receipt: $RECEIPT"

  say "Registering the policy and accepting the quote on NEAR"
  info "Two transactions to $CONTRACT. Storage is paid from the attached NEAR and the rest refunded."
  confirm "The quote is fresh for 5 minutes, so both are sent now."
  bash -c "$(guard_cmd register)"
  bash -c "$(guard_cmd quote)"

  local nonce
  nonce="$(json_get "$RECEIPT" leg.nonce)"
  say "On chain now:"
  near_view get_intent "{\"owner\":\"$ACCOUNT\",\"nonce\":\"$nonce\"}"

  local address amount deadline
  address="$(json_get "$RECEIPT" deposit.address)"
  amount="$(python3 -c 'import sys; print(int(sys.argv[1])/1e6)' "$(json_get "$RECEIPT" deposit.amount)")"
  deadline="$(json_get "$RECEIPT" deposit.deadline)"
  say "Your turn: send the deposit from your Solana wallet"
  info "Send exactly:   $amount USDC (SPL USDC on Solana)"
  info "To:             $address"
  info "Before:         $deadline (UTC)"
  info "From:           $SOLANA"
  info "Then run:       $0 deposited <the transaction signature>"
}

cmd_deposited() {
  load_state
  local sig="${1:-}"
  [[ "$sig" =~ ^[1-9A-HJ-NP-Za-km-z]{64,90}$ ]] || die "give the Solana transaction signature: $0 deposited <signature>"
  [[ -f "$RECEIPT" ]] || die "no receipt; run '$0 quote' first."
  local address
  address="$(json_get "$RECEIPT" deposit.address)"

  say "Telling 1Click about the deposit"
  adapter submit-deposit --deposit-address "$address" --tx-hash "$sig" --receipt "$RECEIPT" >/dev/null

  say "Waiting for the swap to finish (up to 30 minutes)"
  local code=0
  adapter status --deposit-address "$address" --wait --interval 10 --max-wait 1800 --receipt "$RECEIPT" || code=$?
  case "$code" in
    0) info "1Click reports: settled" ;;
    2) info "1Click reports: refunded" ;;
    3) info "1Click reports: failed" ;;
    *) die "the swap has not finished yet. Run '$0 deposited $sig' again later; nothing is lost by waiting." ;;
  esac

  say "Proving the deposit from Solana"
  adapter verify-deposit --receipt "$RECEIPT" || info "The deposit is not proven yet (finalization takes ~15 s). Rerun this step."

  say "Proving the outcome"
  local vcode=0
  if [[ "$code" == 2 ]]; then
    local refund
    read -rp "  Signature of the refund transaction you received on Solana: " refund
    adapter verify-settlement --receipt "$RECEIPT" --refund-tx "$refund" || vcode=$?
  else
    adapter verify-settlement --receipt "$RECEIPT" || vcode=$?
  fi
  case "$vcode" in
    0) info "VERIFIED: settled, proven from Solana and NEAR." ;;
    2) info "VERIFIED: refunded, proven from Solana." ;;
    *) die "not proven from the chains yet; the receipt says what is missing. Rerun this step in a minute." ;;
  esac

  say "Recording the outcome on NEAR"
  confirm "One transaction to $CONTRACT."
  bash -c "$(guard_cmd outcome)"
  cmd_show
}

cmd_show() {
  load_state
  say "This run"
  info "Program:   $PROGRAM"
  info "Receipt:   $RECEIPT"
  info "Contract:  https://nearblocks.io/address/$CONTRACT"
  if [[ -f "$RECEIPT" ]]; then
    python3 - "$RECEIPT" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
print("  Quote hash:", r.get("quote_hash"))
tx = (r.get("deposit") or {}).get("tx_hash")
if tx:
    print("  Deposit:   https://solscan.io/tx/" + tx)
v = r.get("verification")
if v:
    print("  Verified:  ", v["status"], v.get("outcome") or "", "(1Click agrees)" if v.get("1click_agrees") else "")
PY
  fi
}

case "${1:-}" in
  setup) cmd_setup ;;
  check) cmd_check ;;
  build) cmd_build ;;
  deploy) cmd_deploy ;;
  quote) cmd_quote ;;
  deposited) shift; cmd_deposited "${1:-}" ;;
  show) cmd_show ;;
  *) sed -n '2,13p' "$0"; exit 1 ;;
esac

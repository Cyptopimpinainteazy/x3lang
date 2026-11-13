#!/usr/bin/env bash
# vault-testnet.sh: deploy the X3 agent vault to NEAR testnet and attack it with the agent's own key.
#
#   x3-lang/near/vault-testnet.sh setup      choose the owner account and its key file
#   x3-lang/near/vault-testnet.sh check      tools, the owner's key, and the owner's balance
#   x3-lang/near/vault-testnet.sh build      build the vault WASM (cargo near)
#   x3-lang/near/vault-testnet.sh accounts   create vault, agent, vendor and stranger sub-accounts  (4 txs)
#   x3-lang/near/vault-testnet.sh deploy     deploy + initialize the vault in one transaction       (1 tx)
#   x3-lang/near/vault-testnet.sh lock       delete the vault account's only key (irreversible)     (1 tx)
#   x3-lang/near/vault-testnet.sh fund       owner deposits 1 NEAR of escrow                        (1 tx)
#   x3-lang/near/vault-testnet.sh scope      give the agent a request_payment-only key, delete its full key (2 txs)
#   x3-lang/near/vault-testnet.sh attack     the agent pays, then tries to overspend and to steal   (6 txs + 2 refusals)
#   x3-lang/near/vault-testnet.sh revoke     owner revokes, agent is refused, owner recovers escrow (4 txs)
#   x3-lang/near/vault-testnet.sh evidence   read everything back from RPC, judge it, write evidence (read-only)
#   x3-lang/near/vault-testnet.sh show       accounts and explorer links
#
# TESTNET ONLY. Every step that signs asks first. The vault contract itself refuses to
# initialize on any account that does not end in .testnet. Keys stay in ~/.near-credentials;
# this script passes key-file paths to near-cli and never reads, prints or copies a private key.
# State lives in $X3_RUN_DIR (default ~/.x3/vault-testnet).
# shellcheck disable=SC2153  # OWNER, VAULT, AGENT, ... come from the sourced state file
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
VAULT_DIR="$HERE/x3-agent-vault"
WASM="$VAULT_DIR/target/near/x3_agent_vault_testnet.wasm"
HELPER="$REPO/scripts/hackathon/vault_testnet_evidence.py"
RUN_DIR="${X3_RUN_DIR:-$HOME/.x3/vault-testnet}"
STATE="$RUN_DIR/state.env"
RUN="$RUN_DIR/run.json"
LOGS="$RUN_DIR/logs"
NEAR="${NEAR_CLI:-near}"
NET="testnet"
EXPLORER="https://testnet.nearblocks.io/txns"
CREDS="$HOME/.near-credentials"

# Mandate terms, in yoctoNEAR. 1 NEAR = 10^24.
CAP=200000000000000000000000        # 0.2 NEAR per payment
BUDGET=400000000000000000000000     # 0.4 NEAR in total
FUND=1000000000000000000000000      # 1 NEAR of escrow
FUND_NEAR='1 NEAR'
PAY_1=100000000000000000000000      # 0.1: allowed
PAY_2=200000000000000000000000      # 0.2: allowed, brings spend to 0.3
PAY_3=200000000000000000000000      # 0.2: would bring spend to 0.5 > budget
OVER_CAP=300000000000000000000000   # 0.3 > cap
AFTER_REVOKE=50000000000000000000000
AGENT_ALLOWANCE='0.25 NEAR'
METHOD=request_payment

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\n\033[31mStopped: %s\033[0m\n' "$*" >&2; exit 1; }

confirm() {
  local answer
  read -rp "  $1 Type yes to continue: " answer
  [[ "$answer" == "yes" ]] || die "not confirmed; nothing was sent."
}

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is not installed. $2"; }
helper() { python3 "$HELPER" "$@"; }

load_state() {
  [[ -f "$STATE" ]] || die "run '$0 setup' first."
  # shellcheck source=/dev/null
  source "$STATE"
}

has_step() { [[ -f "$RUN" ]] && helper has-step --run "$RUN" --label "$1"; }

key_file() { helper key-file --account "$1" --kind full; }
agent_scoped_key() { helper key-file --account "$AGENT" --kind function-call --receiver "$VAULT" --method "$METHOD"; }

# tx LABEL EXPECT SIGNER [NONCE RECIPIENT AMOUNT] -- near-cli arguments...
# EXPECT is success, returns_true, denied:<reason> or rpc_refused:<refusal>. A step already in the
# run file is skipped, so a command can be re-run after an interruption.
tx() {
  local label="$1" expect="$2" signer="$3"; shift 3
  local extra=()
  if [[ "$1" != "--" ]]; then extra=(--nonce "$1" --recipient "$2" --amount "$3"); shift 3; fi
  [[ "$1" == "--" ]] || die "internal: tx() needs -- before the near-cli arguments"; shift
  if has_step "$label"; then info "$label: already recorded, skipping"; return 0; fi

  mkdir -p "$LOGS"
  local log="$LOGS/$label.log" code=0 hash
  say "$label (expect: $expect)"
  set +e
  "$NEAR" "$@" 2>&1 | tee "$log"
  code=${PIPESTATUS[0]}
  set -e
  hash="$(helper tx-hash "$log")" || die "$label: could not read the transaction id from $log"

  case "$expect" in
    rpc_refused:*)
      [[ -z "$hash" ]] || die "$label reached the chain as $hash. A FunctionCall key must not be able to send this. Stop and investigate."
      (( code != 0 )) || die "$label: near-cli reported success. Stop and investigate."
      helper step --run "$RUN" --label "$label" --expect "$expect" --signer "$signer" --cli-exit "$code" \
        --log "$log" "${extra[@]}" >/dev/null || die "$label: refused, but not with NEAR's access-key refusal; see $log"
      info "refused before it reached the chain (near-cli exit $code), as NEAR requires for this key"
      return 0 ;;
    denied:*)
      (( code != 0 )) || die "$label: the vault ACCEPTED a request it must refuse ($hash). This is a security finding; stop here." ;;
    *)
      (( code == 0 )) || die "$label failed unexpectedly${hash:+ ($EXPLORER/$hash)}; see $log" ;;
  esac
  if [[ -z "$hash" ]]; then
    read -rp "  near-cli printed no transaction id. Paste the hash from the output above (blank stops): " hash
    [[ -n "$hash" ]] || die "$label: no transaction hash; nothing recorded."
  fi
  helper step --run "$RUN" --label "$label" --expect "$expect" --signer "$signer" --tx "$hash" "${extra[@]}" >/dev/null
  info "recorded $EXPLORER/$hash"
}

pay() {  # pay LABEL EXPECT NONCE RECIPIENT AMOUNT
  local args
  args="$(printf '{"nonce":"%s","recipient":"%s","amount":"%s"}' "$3" "$4" "$5")"
  tx "$1" "$2" "$AGENT" "$3" "$4" "$5" -- contract call-function as-transaction "$VAULT" "$METHOD" \
    json-args "$args" prepaid-gas '100 Tgas' attached-deposit '0 NEAR' sign-as "$AGENT" \
    network-config "$NET" sign-with-access-key-file "$(agent_scoped_key)" send
}

# ------------------------------------------------------------------------------------------

cmd_setup() {
  mkdir -p "$RUN_DIR"; chmod 700 "$RUN_DIR"
  [[ ! -f "$STATE" ]] || die "a run already exists in $RUN_DIR. Use X3_RUN_DIR=<new dir> for a fresh one."
  say "Setting up a vault run on NEAR testnet"
  local owner key tag lock
  read -rp "  Owner account (creates and funds everything) [x3lang.testnet]: " owner
  owner="${owner:-x3lang.testnet}"
  [[ "$owner" == *.testnet ]] || die "testnet accounts only."
  read -rp "  Owner key file [$CREDS/$NET/$owner.json]: " key
  key="${key:-$CREDS/$NET/$owner.json}"
  [[ -f "$key" ]] || die "key file not found: $key"
  read -rp "  Lock the vault (delete its only key, so no key can ever move its funds)? [yes]: " lock
  lock="${lock:-yes}"
  [[ "$lock" == yes || "$lock" == no ]] || die "answer yes or no."
  tag="$(date -u +%m%d%H%M)"
  {
    printf 'OWNER=%q\n' "$owner"
    printf 'OWNER_KEY=%q\n' "$key"
    printf 'TAG=%q\n' "$tag"
    printf 'VAULT=%q\n' "vault-$tag.$owner"
    printf 'AGENT=%q\n' "agent-$tag.$owner"
    printf 'VENDOR=%q\n' "vendor-$tag.$owner"
    printf 'STRANGER=%q\n' "stranger-$tag.$owner"
    printf 'LOCK=%q\n' "$lock"
  } > "$STATE"
  chmod 600 "$STATE"
  load_state
  info "vault     $VAULT   holds the escrow; runs the contract"
  info "agent     $AGENT   the AI agent; will hold only a request_payment key"
  info "vendor    $VENDOR  the one recipient the owner allows"
  info "stranger  $STRANGER  an attacker's account"
  info "Next: $0 check"
}

cmd_check() {
  load_state
  say "Tools"
  need python3 ""
  need "$NEAR" "Install near-cli-rs: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/near/near-cli-rs/releases/latest/download/near-cli-rs-installer.sh | sh"
  need cargo "Install Rust: https://rustup.rs"
  cargo near --version >/dev/null 2>&1 || die "cargo-near is not installed: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/near/cargo-near/releases/latest/download/cargo-near-installer.sh | sh"
  info "$("$NEAR" --version)"
  info "$(cargo near --version)"
  say "Owner $OWNER"
  info "key file public key on chain as full access: $(helper key-check --account "$OWNER" --key-file "$OWNER_KEY")"
  "$NEAR" account view-account-summary "$OWNER" network-config "$NET" now
  info "The run costs about 6 testnet NEAR (about 0.7 comes back at the end). Get more from the faucet if needed."
  info "Next: $0 build"
}

cmd_build() {
  load_state
  say "Building the vault"
  (cd "$VAULT_DIR" && cargo near build non-reproducible-wasm)
  [[ -f "$WASM" ]] || die "the build did not produce $WASM"
  info "$WASM: $(wc -c < "$WASM") bytes, sha256 $(sha256sum "$WASM" | cut -d' ' -f1)"
  info "Next: $0 accounts"
}

cmd_accounts() {
  load_state
  [[ -f "$WASM" ]] || die "build first: $0 build"
  local bytes vault_near
  bytes=$(wc -c < "$WASM")
  # Storage is 1 NEAR per 100 kB of code and state; add 1.5 NEAR for state, the contract's
  # 0.5 NEAR recovery reserve, and margin. None of this is spendable by the agent.
  vault_near="$(python3 -c 'import math,sys; print(math.ceil(int(sys.argv[1]) / 100000 + 1.5))' "$bytes")"
  say "Creating four sub-accounts of $OWNER"
  info "$VAULT with $vault_near NEAR (storage for $bytes bytes of code, not spendable by the agent)"
  info "$AGENT with 0.5 NEAR (pays its own gas), $VENDOR and $STRANGER with 0.1 NEAR each"
  confirm "Four transactions from $OWNER."
  if [[ ! -f "$RUN" ]]; then
    local deadline
    deadline=$(( ($(date +%s) + 8 * 86400) * 1000 ))
    helper init --run "$RUN" --owner "$OWNER" --vault "$VAULT" --agent "$AGENT" --vendor "$VENDOR" \
      --stranger "$STRANGER" --cap "$CAP" --budget "$BUDGET" --fund "$FUND" --deadline-ms "$deadline" --locked "$LOCK"
  fi
  local who amount
  for who in vault agent vendor stranger; do
    case "$who" in
      vault) amount="$vault_near NEAR" ;; agent) amount='0.5 NEAR' ;; *) amount='0.1 NEAR' ;;
    esac
    local id_var="${who^^}"
    tx "create_$who" success "$OWNER" -- account create-account fund-myself "${!id_var}" "$amount" \
      autogenerate-new-keypair save-to-legacy-keychain sign-as "$OWNER" network-config "$NET" \
      sign-with-access-key-file "$OWNER_KEY" send
  done
  info "Next: $0 deploy"
}

cmd_deploy() {
  load_state
  has_step create_stranger || die "create the accounts first: $0 accounts"
  local deadline args
  deadline="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["terms"]["deadline_ms"])' "$RUN")"
  args="$(printf '{"owner":"%s","agent":"%s","recipient":"%s","cap":"%s","budget":"%s","deadline_ms":%s}' \
    "$OWNER" "$AGENT" "$VENDOR" "$CAP" "$BUDGET" "$deadline")"
  say "Deploying the vault to $VAULT and initializing it in the same transaction"
  info "owner $OWNER, agent $AGENT, recipient $VENDOR"
  info "per payment 0.2 NEAR, total 0.4 NEAR, expires $(date -u -d "@$(( deadline / 1000 ))" '+%Y-%m-%d %H:%M UTC')"
  confirm "One transaction from $VAULT."
  tx vault_deployed success "$VAULT" -- contract deploy "$VAULT" use-file "$WASM" with-init-call new \
    json-args "$args" prepaid-gas '100 Tgas' attached-deposit '0 NEAR' network-config "$NET" \
    sign-with-access-key-file "$(key_file "$VAULT")" send
  say "Deployed code"
  helper manifest-check --account "$VAULT" --wasm "$WASM"
  helper view --vault "$VAULT"
  if [[ "$LOCK" == yes ]]; then info "Next: $0 lock"; else info "Next: $0 fund"; fi
}

cmd_lock() {
  load_state
  [[ "$LOCK" == yes ]] || die "this run was set up without locking."
  has_step vault_deployed || die "deploy first: $0 deploy"
  if has_step vault_locked; then info "already locked"; return 0; fi
  say "Checking the code one more time before locking"
  helper manifest-check --account "$VAULT" --wasm "$WASM"
  local key pk
  key="$(key_file "$VAULT")"
  pk="$(helper public-key --account "$VAULT" --kind full)"
  say "Locking $VAULT"
  info "This deletes $pk, the vault account's only key. Afterwards no key can sign for the vault:"
  info "the contract code is the only thing that can move its NEAR, and the code can never be changed."
  info "The owner can still recover escrow through recover_available after revoking or expiry."
  info "The storage NEAR in the account stays locked there for good (testnet NEAR)."
  confirm "This cannot be undone."
  tx vault_locked success "$VAULT" -- account delete-keys "$VAULT" public-keys "$pk" \
    network-config "$NET" sign-with-access-key-file "$key" send
  info "Next: $0 fund"
}

cmd_fund() {
  load_state
  has_step vault_deployed || die "deploy first: $0 deploy"
  [[ "$LOCK" != yes ]] || has_step vault_locked || die "lock first: $0 lock"
  say "Owner deposits $FUND_NEAR of escrow into the vault"
  confirm "One transaction from $OWNER, attaching $FUND_NEAR."
  tx owner_funds success "$OWNER" -- contract call-function as-transaction "$VAULT" fund json-args '{}' \
    prepaid-gas '30 Tgas' attached-deposit "$FUND_NEAR" sign-as "$OWNER" network-config "$NET" \
    sign-with-access-key-file "$OWNER_KEY" send
  helper view --vault "$VAULT"
  info "Next: $0 scope"
}

cmd_scope() {
  load_state
  has_step owner_funds || die "fund first: $0 fund"
  local full_key="" full_pk=""
  if ! has_step agent_full_key_deleted; then
    full_key="$(key_file "$AGENT")"
    full_pk="$(helper public-key --account "$AGENT" --kind full)"
  fi
  say "Scoping the agent"
  info "1. Add a key to $AGENT that can only call $VAULT::$METHOD, with $AGENT_ALLOWANCE for gas."
  info "2. Delete $AGENT's full-access key. After this the agent cannot transfer, deploy, add keys"
  info "   or call anything else, from its own account or any other."
  confirm "Two transactions from $AGENT."
  tx agent_key_scoped success "$AGENT" -- account add-key "$AGENT" grant-function-call-access \
    --allowance "$AGENT_ALLOWANCE" --contract-account-id "$VAULT" --function-names "$METHOD" \
    autogenerate-new-keypair save-to-legacy-keychain network-config "$NET" \
    sign-with-access-key-file "$full_key" send
  info "scoped key file: $(agent_scoped_key)"
  tx agent_full_key_deleted success "$AGENT" -- account delete-keys "$AGENT" public-keys "$full_pk" \
    network-config "$NET" sign-with-access-key-file "$full_key" send
  "$NEAR" account list-keys "$AGENT" network-config "$NET" now
  info "Next: $0 attack"
}

cmd_attack() {
  load_state
  has_step agent_full_key_deleted || die "scope the agent first: $0 scope"
  say "The agent, using only its scoped key"
  info "pays the vendor 0.1, tries 0.3 (over the cap), pays a stranger, replays a nonce,"
  info "pays 0.2 more, tries 0.2 past the 0.4 budget, then tries a raw transfer and an owner method."
  info "The last two are refused by NEAR before they reach the chain, so they cost nothing."
  confirm "Six transactions from $AGENT, paid from its key allowance."
  if ! python3 -c 'import json,sys; sys.exit(0 if "before_attacks" in json.load(open(sys.argv[1]))["snapshots"] else 1)' "$RUN"; then
    helper snapshot --run "$RUN" --name before_attacks --account "$VENDOR" --account "$STRANGER" >/dev/null
  fi
  pay agent_pays_vendor       returns_true                           pay-1      "$VENDOR"   "$PAY_1"
  pay agent_over_cap          'denied:per-payment cap exceeded'      over-cap   "$VENDOR"   "$OVER_CAP"
  pay agent_pays_stranger     'denied:recipient not allowlisted'     to-stranger "$STRANGER" "$PAY_1"
  pay agent_replays_nonce     'denied:nonce already used'            pay-1      "$VENDOR"   "$PAY_1"
  pay agent_pays_vendor_again returns_true                           pay-2      "$VENDOR"   "$PAY_2"
  pay agent_over_budget       'denied:cumulative budget exceeded'    pay-3      "$VENDOR"   "$PAY_3"
  local scoped
  scoped="$(agent_scoped_key)"
  tx agent_raw_transfer rpc_refused:requires_full_access "$AGENT" -- tokens "$AGENT" send-near "$STRANGER" '0.01 NEAR' \
    network-config "$NET" sign-with-access-key-file "$scoped" send
  tx agent_calls_revoke rpc_refused:method_not_allowed "$AGENT" -- contract call-function as-transaction "$VAULT" revoke \
    json-args '{}' prepaid-gas '30 Tgas' attached-deposit '0 NEAR' sign-as "$AGENT" \
    network-config "$NET" sign-with-access-key-file "$scoped" send
  helper view --vault "$VAULT"
  info "Next: $0 revoke"
}

cmd_revoke() {
  load_state
  has_step agent_over_budget || die "run the attacks first: $0 attack"
  say "Revocation and recovery"
  info "The owner tries to pull escrow while the mandate is live (refused), revokes it,"
  info "the agent tries to pay again (refused), and the owner recovers the remaining escrow."
  confirm "Three transactions from $OWNER and one from $AGENT."
  tx owner_recovers_while_active 'denied:revoke or expire mandate before recovery' "$OWNER" -- \
    contract call-function as-transaction "$VAULT" recover_available json-args "{\"amount\":\"$PAY_1\"}" \
    prepaid-gas '100 Tgas' attached-deposit '0 NEAR' sign-as "$OWNER" network-config "$NET" \
    sign-with-access-key-file "$OWNER_KEY" send
  tx owner_revokes success "$OWNER" -- contract call-function as-transaction "$VAULT" revoke json-args '{}' \
    prepaid-gas '30 Tgas' attached-deposit '0 NEAR' sign-as "$OWNER" network-config "$NET" \
    sign-with-access-key-file "$OWNER_KEY" send
  pay agent_after_revoke 'denied:owner revoked mandate' after-revoke "$VENDOR" "$AFTER_REVOKE"
  local available
  available="$(helper view --vault "$VAULT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["available"])')"
  [[ "$available" =~ ^[1-9][0-9]*$ ]] || die "nothing left to recover (available=$available)"
  tx owner_recovers returns_true "$OWNER" -- contract call-function as-transaction "$VAULT" recover_available \
    json-args "{\"amount\":\"$available\"}" prepaid-gas '100 Tgas' attached-deposit '0 NEAR' sign-as "$OWNER" \
    network-config "$NET" sign-with-access-key-file "$OWNER_KEY" send
  helper view --vault "$VAULT"
  info "Next: $0 evidence"
}

cmd_evidence() {
  load_state
  has_step owner_recovers || die "finish the run first: $0 revoke"
  local out="$REPO/evidence/agent-vault-testnet-$TAG/evidence.json"
  say "Reading the whole run back from NEAR testnet RPC"
  helper collect --run "$RUN" --wasm "$WASM" --out "$out"
  say "Re-verifying the written evidence"
  helper verify --evidence "$out" --wasm "$WASM"
  info "Evidence: $out"
  info "Commit it, then send Claude the path. Nothing in it is secret: it holds account ids,"
  info "public keys, transaction hashes and RPC results."
}

cmd_show() {
  load_state
  say "Accounts"
  for id in "$OWNER" "$VAULT" "$AGENT" "$VENDOR" "$STRANGER"; do info "https://testnet.nearblocks.io/address/$id"; done
  [[ -f "$RUN" ]] || return 0
  say "Transactions"
  python3 - "$RUN" "$EXPLORER" <<'PY'
import json, sys
run = json.load(open(sys.argv[1]))
for s in run["steps"]:
    link = f"{sys.argv[2]}/{s['tx_hash']}" if s.get("tx_hash") else f"(never reached the chain, near-cli exit {s['cli_exit']})"
    want = s["expect"] + (": " + s["reason"] if s.get("reason") else "")
    print(f"  {s['label']:<28} {want:<50} {link}")
PY
}

case "${1:-}" in
  setup) cmd_setup ;; check) cmd_check ;; build) cmd_build ;; accounts) cmd_accounts ;;
  deploy) cmd_deploy ;; lock) cmd_lock ;; fund) cmd_fund ;; scope) cmd_scope ;;
  attack) cmd_attack ;; revoke) cmd_revoke ;; evidence) cmd_evidence ;; show) cmd_show ;;
  *) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac

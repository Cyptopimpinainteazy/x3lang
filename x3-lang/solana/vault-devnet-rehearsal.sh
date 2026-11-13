#!/usr/bin/env bash
# vault-devnet-rehearsal.sh: rehearse the X3 Solana mandate vault on DEVNET at zero value.
#
# Every instruction and every refusal the mainnet run needs is exercised here first, with a
# throwaway test token. This script refuses to run against any cluster but devnet.
#
#   vault-devnet-rehearsal.sh setup                 record key paths and the RPC URL
#   vault-devnet-rehearsal.sh check                 tools, cluster identity, balances
#   vault-devnet-rehearsal.sh build                 cargo build-sbf; print size, SHA-256 and rent
#   vault-devnet-rehearsal.sh deploy                deploy the program            (costs devnet SOL)
#   vault-devnet-rehearsal.sh accounts              test mint, vault and recipient token accounts
#   vault-devnet-rehearsal.sh initialize            owner creates the policy
#   vault-devnet-rehearsal.sh approve               owner approves one exact payment
#   vault-devnet-rehearsal.sh attack-not-agent      owner key tries Execute        -> Unauthorized
#   vault-devnet-rehearsal.sh attack-destination    agent pays another account     -> IncorrectTokenAccount
#   vault-devnet-rehearsal.sh execute               agent executes the approval
#   vault-devnet-rehearsal.sh attack-replay         agent repeats the payment      -> MissingApproval
#   vault-devnet-rehearsal.sh revoke                owner revokes the policy
#   vault-devnet-rehearsal.sh attack-post-revoke    agent executes after revoke    -> Revoked
#   vault-devnet-rehearsal.sh withdraw              owner recovers the remainder
#   vault-devnet-rehearsal.sh show                  state, addresses and recorded signatures
#
# The vault steps go through vault_client.py, which prints the full transaction, simulates it,
# and signs only after you type "yes" on the terminal. The Solana CLI steps ask here first.
# Key files are referenced by path and never copied or printed. State lives in
# $X3_VAULT_RUN_DIR (default ~/.x3/vault-devnet), mode 700.
#
# Devnet proves the program and the client work together. It does not involve 1Click, so the
# quote digest used here is a rehearsal label, not a signed quote.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT="$HERE/vault_client.py"
CRATE="$HERE/x3-mandate-vault"
RUN_DIR="${X3_VAULT_RUN_DIR:-$HOME/.x3/vault-devnet}"
STATE="$RUN_DIR/state.env"
LOG="$RUN_DIR/signatures.log"
SBF_DIR="$RUN_DIR/sbf"
DEVNET_GENESIS="EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
DECIMALS=6
PER_PAYMENT=25000000     # 25 test tokens
PER_DAY=50000000         # 50 test tokens
PAYMENT=10000000         # 10 test tokens
VAULT_FUNDING=40         # whole test tokens minted into the vault

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
die()  { printf '\n\033[31mStopped: %s\033[0m\n' "$*" >&2; exit 1; }

confirm() {
  local answer
  read -rp "  $1 Type yes to continue: " answer
  [[ "$answer" == "yes" ]] || die "not confirmed; nothing was sent."
}

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is not installed. $2"; }

load_state() {
  [[ -f "$STATE" ]] || die "run '$0 setup' first."
  # shellcheck source=/dev/null
  source "$STATE"
}

save() { printf '%s=%q\n' "$1" "$2" >>"$STATE"; }

require() {
  local name
  for name in "$@"; do
    [[ -n "${!name:-}" ]] || die "$name is not recorded yet; run the earlier steps first ('$0 show')."
  done
}

json_get() { python3 -c 'import json,sys; print(json.loads(sys.argv[1])[sys.argv[2]])' "$1" "$2"; }

client() { python3 "$CLIENT" "$@"; }

# One vault instruction: full plan, simulation, typed yes, finalized result. The client's
# JSON goes to the terminal and to a per-step file; signatures are appended to $LOG.
vault() {
  local label="$1" command="$2" key="$3"; shift 3
  say "$label"
  local out="$RUN_DIR/$label.jsonl" code=0
  client "$command" --rpc-url "$RPC" --cluster devnet --program "$PROGRAM" --keypair "$key" --send "$@" \
    | tee "$out" || code=$?
  python3 - "$out" "$label" >>"$LOG" <<'PY'
import json, sys
decoder, text, position = json.JSONDecoder(), open(sys.argv[1]).read(), 0
while position < len(text):
    if text[position].isspace():
        position += 1
        continue
    value, position = decoder.raw_decode(text, position)
    if value.get("sent") is True:
        print(sys.argv[2], value.get("signature"), "finalized=" + str(value.get("finalized")),
              "error=" + str(value.get("error")), "as_expected=" + str(value.get("as_expected")))
PY
  [[ "$code" == "0" ]] || die "$label did not finish as expected (exit $code). Read $out before retrying."
}

assert_devnet() {
  local genesis
  genesis="$(solana genesis-hash --url "$RPC")"
  [[ "$genesis" == "$DEVNET_GENESIS" ]] || die "the RPC at $RPC is not devnet (genesis $genesis). This script is devnet only."
}

cmd_setup() {
  mkdir -p "$RUN_DIR"
  chmod 700 "$RUN_DIR"
  say "Setting up the devnet vault rehearsal"
  local owner_key agent_key rpc
  read -rp "  Owner keypair file (a devnet-only key): " owner_key
  [[ -f "$owner_key" ]] || die "owner keypair file not found: $owner_key"
  read -rp "  Agent keypair file (a different devnet-only key): " agent_key
  [[ -f "$agent_key" ]] || die "agent keypair file not found: $agent_key"
  [[ "$owner_key" != "$agent_key" ]] || die "the agent must use a different key."
  read -rp "  Devnet RPC URL [https://api.devnet.solana.com]: " rpc
  rpc="${rpc:-https://api.devnet.solana.com}"
  local owner agent
  owner="$(client address --keypair "$owner_key")"
  agent="$(client address --keypair "$agent_key")"
  [[ "$owner" != "$agent" ]] || die "the agent must use a different key."
  : >"$STATE"
  chmod 600 "$STATE"
  save OWNER_KEY "$owner_key"
  save AGENT_KEY "$agent_key"
  save OWNER "$owner"
  save AGENT "$agent"
  save RPC "$rpc"
  info "Owner $owner"
  info "Agent $agent"
  info "Key files are referenced by path only; they were not copied or printed."
  info "Next: $0 check"
}

cmd_check() {
  load_state
  need solana "Install the Agave/Solana CLI."
  need spl-token "Install it with: cargo install spl-token-cli"
  need solana-keygen "It ships with the Solana CLI."
  need cargo-build-sbf "It ships with the Solana CLI (cargo build-sbf)."
  need python3 "Python 3.11+ with x3-lang/requirements.txt."
  assert_devnet
  say "Cluster and balances"
  info "RPC $RPC is devnet."
  info "Owner balance: $(solana balance "$OWNER" --url "$RPC")"
  info "Agent balance: $(solana balance "$AGENT" --url "$RPC")"
  info "The owner needs devnet SOL for the deploy and rent; the agent needs a little for fees."
  info "Faucet: solana airdrop 2 $OWNER --url $RPC"
  info "Next: $0 build"
}

cmd_build() {
  load_state
  need cargo-build-sbf "It ships with the Solana CLI (cargo build-sbf)."
  say "Building the SBF program"
  mkdir -p "$SBF_DIR"
  cargo build-sbf --manifest-path "$CRATE/Cargo.toml" --sbf-out-dir "$SBF_DIR"
  local so="$SBF_DIR/x3_mandate_vault.so"
  [[ -f "$so" ]] || die "the build produced no $so"
  chmod 600 "$SBF_DIR"/*-keypair.json 2>/dev/null || true
  local bytes
  bytes="$(wc -c <"$so" | tr -d ' ')"
  info "Program: $so"
  info "Size: $bytes bytes"
  info "SHA-256: $(sha256sum "$so" | cut -d' ' -f1)"
  info "Rent for the program data at this size:"
  solana rent "$bytes" --url "$RPC" | sed 's/^/    /'
  info "An upgradeable deploy reserves about twice the program size; closing the program returns it."
  info "Record the size and SHA-256: the mainnet run must deploy these exact bytes."
  info "Next: $0 deploy"
}

cmd_deploy() {
  load_state
  assert_devnet
  local so="$SBF_DIR/x3_mandate_vault.so" program_key="$SBF_DIR/x3_mandate_vault-keypair.json"
  [[ -f "$so" && -f "$program_key" ]] || die "run '$0 build' first."
  say "Deploying to devnet"
  local program
  program="$(solana-keygen pubkey "$program_key")"
  info "Program ID: $program"
  info "Upgrade authority: $OWNER (the owner key). Devnet only."
  confirm "This deploys the program and spends devnet SOL."
  solana program deploy "$so" --program-id "$program_key" --keypair "$OWNER_KEY" --url "$RPC"
  save PROGRAM "$program"
  save PROGRAM_SHA256 "$(sha256sum "$so" | cut -d' ' -f1)"
  info "Next: $0 accounts"
}

new_key() {
  local path="$RUN_DIR/$1.json"
  [[ -f "$path" ]] || solana-keygen new --no-bip39-passphrase --silent --outfile "$path" >/dev/null
  chmod 600 "$path"
  printf '%s' "$path"
}

cmd_accounts() {
  load_state
  require PROGRAM
  assert_devnet
  say "Creating the rehearsal token and accounts"
  local mint_key vault_key recipient_key attacker_key
  mint_key="$(new_key mint)"
  vault_key="$(new_key vault-token)"
  recipient_key="$(new_key recipient)"
  attacker_key="$(new_key attacker)"
  local mint vault_token recipient attacker policy recipient_token attacker_token owner_token
  mint="$(solana-keygen pubkey "$mint_key")"
  vault_token="$(solana-keygen pubkey "$vault_key")"
  recipient="$(solana-keygen pubkey "$recipient_key")"
  attacker="$(solana-keygen pubkey "$attacker_key")"
  policy="$(json_get "$(client derive --program "$PROGRAM" --owner "$OWNER" --vault-token "$vault_token")" policy)"
  recipient_token="$(json_get "$(client ata --wallet "$recipient" --mint "$mint")" associated_token_account)"
  attacker_token="$(json_get "$(client ata --wallet "$attacker" --mint "$mint")" associated_token_account)"
  owner_token="$(json_get "$(client ata --wallet "$OWNER" --mint "$mint")" associated_token_account)"
  info "Test mint          $mint ($DECIMALS decimals, worthless)"
  info "Policy PDA         $policy"
  info "Vault token        $vault_token (authority: the policy PDA)"
  info "Recipient          $recipient -> $recipient_token (stands in for the 1Click deposit address)"
  info "Attacker           $attacker -> $attacker_token"
  info "Owner token        $owner_token"
  confirm "This sends several devnet transactions from the owner: one mint, four token accounts, one mint-to, one SOL transfer."
  spl-token create-token "$mint_key" --decimals "$DECIMALS" --mint-authority "$OWNER" --fee-payer "$OWNER_KEY" --url "$RPC"
  spl-token create-account "$mint" "$vault_key" --owner "$policy" --fee-payer "$OWNER_KEY" --url "$RPC"
  spl-token create-account "$mint" --owner "$recipient" --fee-payer "$OWNER_KEY" --url "$RPC"
  spl-token create-account "$mint" --owner "$attacker" --fee-payer "$OWNER_KEY" --url "$RPC"
  spl-token create-account "$mint" --owner "$OWNER" --fee-payer "$OWNER_KEY" --url "$RPC"
  spl-token mint "$mint" "$VAULT_FUNDING" "$vault_token" --mint-authority "$OWNER_KEY" --fee-payer "$OWNER_KEY" --url "$RPC"
  solana transfer "$AGENT" 0.05 --allow-unfunded-recipient --keypair "$OWNER_KEY" --url "$RPC"
  save MINT "$mint"
  save VAULT_TOKEN "$vault_token"
  save POLICY "$policy"
  save RECIPIENT "$recipient"
  save RECIPIENT_TOKEN "$recipient_token"
  save ATTACKER_TOKEN "$attacker_token"
  save OWNER_TOKEN "$owner_token"
  info "Next: $0 initialize"
}

cmd_initialize() {
  load_state
  require PROGRAM MINT VAULT_TOKEN
  local expires=$(( $(date +%s) + 172800 ))
  vault initialize initialize "$OWNER_KEY" --vault-token "$VAULT_TOKEN" --mint "$MINT" --agent "$AGENT" \
    --max-per-payment "$PER_PAYMENT" --max-per-day "$PER_DAY" --expires-at "$expires"
  info "Next: $0 approve"
}

cmd_approve() {
  load_state
  require PROGRAM POLICY RECIPIENT RECIPIENT_TOKEN
  local nonce digest
  nonce=$(( ${NONCE:-0} + 1 ))
  digest="$(printf 'x3-vault-devnet-rehearsal:%s:%s' "$POLICY" "$nonce" | sha256sum | cut -d' ' -f1)"
  vault "approve-$nonce" approve "$OWNER_KEY" --policy "$POLICY" --nonce "$nonce" --quote-digest "$digest" \
    --destination-token "$RECIPIENT_TOKEN" --destination-owner "$RECIPIENT" --amount "$PAYMENT" \
    --valid-until $(( $(date +%s) + 1800 ))
  save NONCE "$nonce"
  save DIGEST "$digest"
  info "Next: $0 attack-not-agent"
}

cmd_attack_not_agent() {
  load_state
  require PROGRAM POLICY NONCE
  vault attack-not-agent execute "$OWNER_KEY" --policy "$POLICY" --expect-error 1
  info "Recorded: a key that is not the agent cannot execute. Next: $0 attack-destination"
}

cmd_attack_destination() {
  load_state
  require PROGRAM POLICY ATTACKER_TOKEN NONCE
  vault attack-destination execute "$AGENT_KEY" --policy "$POLICY" --destination-token "$ATTACKER_TOKEN" --expect-error 10
  info "Recorded: the agent cannot redirect an approved payment. Next: $0 execute"
}

cmd_execute() {
  load_state
  require PROGRAM POLICY NONCE
  vault "execute-$NONCE" execute "$AGENT_KEY" --policy "$POLICY"
  info "Next: $0 attack-replay"
}

cmd_attack_replay() {
  load_state
  require PROGRAM POLICY RECIPIENT_TOKEN NONCE DIGEST
  vault attack-replay execute "$AGENT_KEY" --policy "$POLICY" --nonce "$NONCE" --quote-digest "$DIGEST" \
    --destination-token "$RECIPIENT_TOKEN" --expect-error 7
  info "Recorded: an executed approval cannot be replayed. Next: $0 revoke"
}

cmd_revoke() {
  load_state
  require PROGRAM POLICY
  vault revoke revoke "$OWNER_KEY" --policy "$POLICY"
  info "Next: $0 attack-post-revoke"
}

cmd_attack_post_revoke() {
  load_state
  require PROGRAM POLICY RECIPIENT_TOKEN NONCE DIGEST
  vault attack-post-revoke execute "$AGENT_KEY" --policy "$POLICY" --nonce "$NONCE" --quote-digest "$DIGEST" \
    --destination-token "$RECIPIENT_TOKEN" --expect-error 5
  info "Recorded: nothing executes after revocation. Next: $0 withdraw"
}

cmd_withdraw() {
  load_state
  require PROGRAM POLICY OWNER_TOKEN
  local remaining
  remaining="$(json_get "$(client show --rpc-url "$RPC" --cluster devnet --program "$PROGRAM" --policy "$POLICY")" vault_balance)"
  [[ "$remaining" != "0" ]] || die "the vault is already empty."
  vault withdraw withdraw "$OWNER_KEY" --policy "$POLICY" --destination-token "$OWNER_TOKEN" --amount "$remaining"
  info "Rehearsal complete. Run '$0 show' and keep $LOG with the build size and SHA-256."
}

cmd_show() {
  load_state
  say "Rehearsal state ($RUN_DIR)"
  grep -v '_KEY=' "$STATE" | sed 's/^/  /'
  if [[ -n "${PROGRAM:-}" && -n "${POLICY:-}" ]]; then
    say "Policy on devnet"
    client show --rpc-url "$RPC" --cluster devnet --program "$PROGRAM" --policy "$POLICY" || true
  fi
  if [[ -f "$LOG" ]]; then
    say "Recorded transactions"
    sed 's/^/  /' "$LOG"
    info "Explorer: https://explorer.solana.com/tx/<signature>?cluster=devnet"
  fi
}

case "${1:-}" in
  setup) cmd_setup ;;
  check) cmd_check ;;
  build) cmd_build ;;
  deploy) cmd_deploy ;;
  accounts) cmd_accounts ;;
  initialize) cmd_initialize ;;
  approve) cmd_approve ;;
  attack-not-agent) cmd_attack_not_agent ;;
  attack-destination) cmd_attack_destination ;;
  execute) cmd_execute ;;
  attack-replay) cmd_attack_replay ;;
  revoke) cmd_revoke ;;
  attack-post-revoke) cmd_attack_post_revoke ;;
  withdraw) cmd_withdraw ;;
  show) cmd_show ;;
  *) sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac

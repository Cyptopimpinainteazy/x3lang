# Shared helpers for the DX3HACK owner scripts. Sourced, not executed.
# Reads go straight to NEAR RPC with curl; only signing uses the near CLI.
# The variables set_network defines are used by the scripts that source this file.
# shellcheck disable=SC2034

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
say() { printf '\n== %s\n' "$*"; }

need() { for c in "$@"; do command -v "$c" >/dev/null 2>&1 || die "missing command: $c"; done; }

# confirm "what this does" "what it costs" -> returns only if the owner types yes.
confirm() {
  printf '\n-- About to: %s\n   Cost: %s\n' "$1" "$2"
  if [ "${DX3HACK_YES:-}" = "i-have-reviewed-every-step" ]; then
    echo "   (DX3HACK_YES set; proceeding)"
    return 0
  fi
  local answer
  read -r -p "   Type yes to sign it: " answer </dev/tty
  [ "$answer" = "yes" ] || die "stopped before signing"
}

set_network() {
  case "$1" in
    testnet) SUFFIX=testnet; ARGS_FILE=deploy/init-args.testnet.json; EXPLORER=https://testnet.nearblocks.io
             RPCS=("https://rpc.testnet.fastnear.com" "https://rpc.testnet.near.org") ;;
    mainnet) SUFFIX=near; ARGS_FILE=deploy/init-args.json; EXPLORER=https://nearblocks.io
             RPCS=("https://rpc.mainnet.fastnear.com" "https://rpc.mainnet.near.org") ;;
    *) die "network must be testnet or mainnet, got: $1" ;;
  esac
  NETWORK=$1
  OWNER=x3lang.$SUFFIX
  TOKEN=dx3hack.x3lang.$SUFFIX
  FAUCET=faucet.x3lang.$SUFFIX
  JUDGES=judges.x3lang.$SUFFIX
  KEY_DIR=${DX3HACK_KEY_DIR:-$HOME/.x3/dx3hack}
  FAUCET_KEY_FILE=$KEY_DIR/faucet-key.$NETWORK.json
}

# rpc '{"method":...,"params":...}' -> prints the JSON result, or fails with the RPC error.
rpc() {
  local body out url
  body=$(jq -c '. + {jsonrpc:"2.0", id:"dx3hack"}' <<<"$1")
  for url in "${RPCS[@]}"; do
    if out=$(curl -sS --fail --max-time 20 -H 'content-type: application/json' -d "$body" "$url" 2>/dev/null); then
      if jq -e '.error' >/dev/null <<<"$out"; then
        jq -r '.error.cause.name // .error.name // "RPC_ERROR"' <<<"$out" >&2
        return 2
      fi
      jq -c '.result' <<<"$out"
      return 0
    fi
  done
  echo "UNREACHABLE" >&2
  return 3
}

account_exists() {
  local err
  if rpc "$(jq -nc --arg a "$1" '{method:"query",params:{request_type:"view_account",finality:"final",account_id:$a}}')" >/dev/null 2>/tmp/dx3hack-rpc.err; then
    return 0
  fi
  err=$(cat /tmp/dx3hack-rpc.err)
  [ "$err" = "UNKNOWN_ACCOUNT" ] && return 1
  die "couldn't read $1 from RPC ($err)"
}

has_contract() {
  local r
  r=$(rpc "$(jq -nc --arg a "$1" '{method:"query",params:{request_type:"view_account",finality:"final",account_id:$a}}')") || return 1
  [ "$(jq -r '.code_hash' <<<"$r")" != "11111111111111111111111111111111" ]
}

# view CONTRACT METHOD JSON_ARGS -> prints the decoded JSON return value.
view() {
  local args_b64 r
  args_b64=$(printf '%s' "$3" | base64 | tr -d '\n')
  r=$(rpc "$(jq -nc --arg a "$1" --arg m "$2" --arg b "$args_b64" \
    '{method:"query",params:{request_type:"call_function",finality:"final",account_id:$a,method_name:$m,args_base64:$b}}')") || return 1
  jq -r '.result | implode' <<<"$r"
}

access_keys() {
  rpc "$(jq -nc --arg a "$1" '{method:"query",params:{request_type:"view_access_key_list",finality:"final",account_id:$a}}')"
}

# Generates an ed25519 keypair with openssl and writes a NEAR key file (mode 600).
# Prints the public key.
make_key_file() {
  local file=$1 account=$2 tmp
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  openssl genpkey -algorithm ed25519 -out "$tmp/k.pem" 2>/dev/null
  openssl pkey -in "$tmp/k.pem" -outform DER 2>/dev/null | tail -c 32 > "$tmp/seed"
  openssl pkey -in "$tmp/k.pem" -pubout -outform DER 2>/dev/null | tail -c 32 > "$tmp/pub"
  python3 -I - "$tmp/seed" "$tmp/pub" "$file" "$account" <<'PY'
import json, os, sys
ALPHA = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = ALPHA[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s
seed = open(sys.argv[1], "rb").read(); pub = open(sys.argv[2], "rb").read()
assert len(seed) == 32 and len(pub) == 32, "openssl did not produce an ed25519 key"
key = {"account_id": sys.argv[4], "public_key": "ed25519:" + b58(pub), "private_key": "ed25519:" + b58(seed + pub)}
fd = os.open(sys.argv[3], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w") as f: json.dump(key, f, indent=2); f.write("\n")
print(key["public_key"])
PY
}

#!/usr/bin/env bash
# Deploy the X3 Agent Forge to forge.dotx3.net (or X3_FORGE_HOST) on this machine.
#
#   deploy.sh up [--ref REF]      first deploy: check zone -> Access gate -> release -> service -> tunnel -> DNS
#   deploy.sh update [--ref REF]  ship a new commit to the running service (no Cloudflare calls)
#   deploy.sh status              service, release, mode, and what the hostname returns
#   deploy.sh lock                (re)create the owner-only Access gate and require it at the origin
#   deploy.sh token               print the one-time High Command setup token
#   deploy.sh down                stop the service and the tunnel (DNS record stays)
#
# Needs: git, node >= 20.19, npm, python3 (>= 3.10, with venv), curl, cloudflared logged in
# (~/.cloudflared/cert.pem), systemd --user.
# For up / lock: CF_API_TOKEN, CF_ACCOUNT_ID, X3_OWNER_EMAIL in the environment. The token needs
# Access: Apps and Policies Edit, Access: Organizations Read, and Zone: Read + DNS: Edit on the
# zone that holds the hostname. It is never written to disk.
#
# Separate from deploy/x3star: its own tunnel (x3-forge), its own Access app
# and owner-only policy ("X3 Agent Forge"), its own port. It reuses the Access helper code only,
# so it can never change the x3star.net demos' gate.
# Fail-closed: the Forge refuses every request without the owner's Access JWT, and DNS is only
# pointed at the tunnel after the Access app is verified.
set -Eeuo pipefail
trap 'echo "deploy.sh: failed at line $LINENO: $BASH_COMMAND" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST="${X3_FORGE_HOST:-forge.dotx3.net}"
STATE="${X3_FORGE_HOME:-$HOME/x3-forge}"
PORT="${X3_FORGE_PORT:-7420}"
TUNNEL_NAME="${X3_FORGE_TUNNEL:-x3-forge}"
APP_NAME="${X3_FORGE_APP_NAME:-X3 Agent Forge (dotx3.net)}"
POLICY_NAME="${X3_FORGE_POLICY_NAME:-X3 Agent Forge: owner only}"
CF_DIR="${X3_CLOUDFLARED_DIR:-$HOME/.cloudflared}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
ENV_FILE="$STATE/forge.env"
TUNNEL_CFG="$STATE/cloudflared.yml"
DATA="$STATE/data"
VENV="$STATE/venv"
WEB_UNIT="x3-forge-web.service"
TUNNEL_UNIT="x3-forge-tunnel.service"
KEEP_RELEASES=5

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"; }

[[ "$HOST" =~ ^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$ ]] || die "X3_FORGE_HOST is not a hostname: $HOST"

node_bin() {
  need node
  node -e 'const [a,b]=process.versions.node.split(".").map(Number);process.exit(a>22||a===22&&b>=12||a===20&&b>=19?0:1)' \
    || die "node 20.19+ or 22.12+ is required to build the web app (found $(node --version))"
  command -v node
}

repo_root() { git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || die "deploy.sh must run from inside an x3lang git checkout"; }
current() { echo "$STATE/current"; }
manifest() { echo "$STATE/access-sites.json"; }

env_get() { [[ -f "$ENV_FILE" ]] && sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1 || true; }

parse_ref() {
  REF="HEAD"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --ref) [[ $# -ge 2 ]] || die "--ref needs a value"; REF="$2"; shift 2 ;;
      *) die "unknown option: $1" ;;
    esac
  done
}

require_cf_env() {
  [[ -n "${CF_API_TOKEN:-}" ]] || die "CF_API_TOKEN is not set"
  [[ -n "${CF_ACCOUNT_ID:-}" ]] || die "CF_ACCOUNT_ID is not set"
  [[ "${X3_OWNER_EMAIL:-}" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+$ ]] || die "X3_OWNER_EMAIL is not set or not an email (the only address allowed through the login)"
}

# ── Release: the Forge and the x3-lang modules it imports, from one exact commit ─────
build_release() {
  local ref="$1" repo sha dest tmp
  repo="$(repo_root)"
  need tar; need npm; need python3
  sha="$(git -C "$repo" rev-parse --verify --quiet "${ref}^{commit}")" || die "unknown git ref: $ref"
  mkdir -p "$STATE/releases" "$DATA"
  chmod 700 "$STATE" "$DATA"
  dest="$STATE/releases/$sha"
  if [[ ! -f "$dest/RELEASE" ]]; then
    tmp="$(mktemp -d "$STATE/releases/.tmp.XXXXXX")"
    mapfile -t files < <(git -C "$repo" ls-tree -r --name-only "$sha" -- x3-lang | awk -F/ 'NF==2 && /\.py$/')
    files+=(x3-lang/forge x3-lang/requirements.txt x3-lang/tests/fixtures/near_intents_tokens.json
            deploy/forge deploy/x3star/cf-access.mjs)
    git -C "$repo" archive --format=tar "$sha" -- "${files[@]}" | tar -x -C "$tmp" \
      || { rm -rf "$tmp"; die "could not export the Forge from $sha (is it on this commit?)"; }
    [[ -f "$tmp/x3-lang/forge/server.py" && -f "$tmp/x3-lang/forge/web/package-lock.json" ]] \
      || { rm -rf "$tmp"; die "$sha does not contain the Agent Forge"; }
    say "building the web app for ${sha:0:12}"
    node_bin >/dev/null
    (cd "$tmp/x3-lang/forge/web" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --no-audit --no-fund --loglevel=error && npx vite build --logLevel warn) \
      || { rm -rf "$tmp"; die "web build failed"; }
    rm -rf "$tmp/x3-lang/forge/web/node_modules"
    echo "$sha" > "$tmp/RELEASE"
    chmod -R go-w "$tmp"
    rm -rf "$dest"
    mv "$tmp" "$dest"
  fi
  ensure_venv "$dest"
  ln -sfn "releases/$sha" "$STATE/current.new"
  mv -Tf "$STATE/current.new" "$STATE/current"
  local n=0 r
  while IFS= read -r r; do
    n=$((n + 1))
    [[ $n -le $KEEP_RELEASES || "$r" == "$sha" ]] && continue
    rm -rf "${STATE:?}/releases/$r"
  done < <(find "$STATE/releases" -mindepth 1 -maxdepth 1 -type d -regextype posix-extended -regex '.*/[0-9a-f]{40}' -printf '%T@ %f\n' | sort -rn | cut -d' ' -f2)
  say "release ${sha:0:12} -> $STATE/current"
}

ensure_venv() {
  local rel="$1"
  python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' || die "python 3.10+ is required"
  [[ -x "$VENV/bin/python" ]] || python3 -m venv "$VENV" || die "python3 -m venv failed (install python3-venv)"
  "$VENV/bin/python" -m pip install --quiet --disable-pip-version-check -r "$rel/x3-lang/requirements.txt" || die "pip install failed"
  (cd "$rel/x3-lang" && "$VENV/bin/python" -I -c 'import sys; sys.path.insert(0, "."); from forge import server, core; core.load_registry(); core.load_assets()') \
    || die "the release does not import cleanly under $VENV"
}

# ── Cloudflare ──────────────────────────────────────────────────────────────────────
write_manifest() {
  printf '{"sites": {"%s": {}}}\n' "$HOST" > "$(manifest)"
}

cf_access() {
  X3_DEMOS_MANIFEST="$(manifest)" X3_ACCESS_APP_NAME="$APP_NAME" X3_ACCESS_POLICY_NAME="$POLICY_NAME" "$(node_bin)" "$(current)/deploy/x3star/cf-access.mjs" "$@"
}

cf_dns() { "$(node_bin)" "$(current)/deploy/forge/cf-dns.mjs" "$@"; }

json_field() { node -e 'const o=JSON.parse(process.argv[1]);process.stdout.write(String(o[process.argv[2]]??""))' "$1" "$2"; }

check_zone() {
  local out
  out="$(cf_dns zone "$HOST")" || die "$HOST cannot be served: its domain is not an active Cloudflare zone in account $CF_ACCOUNT_ID (or the token lacks Zone: Read)"
  say "$HOST is in Cloudflare zone $(json_field "$out" name)"
}

write_env() {
  local team="$1" aud="$2" token tmp
  token="$(env_get FORGE_SETUP_TOKEN)"
  [[ -n "$token" ]] || token="$(python3 -c 'import secrets; print(secrets.token_urlsafe(18))')"
  tmp="$(umask 077 && mktemp "$STATE/.forge.env.XXXXXX")"
  {
    echo "# Written by deploy/forge/deploy.sh $(date -u +%Y-%m-%dT%H:%M:%SZ). Keep it 0600."
    echo "FORGE_ACCESS_MODE=access"
    echo "FORGE_ACCESS_TEAM_DOMAIN=$team"
    echo "FORGE_ACCESS_AUD=$aud"
    echo "FORGE_OWNER_EMAIL=$X3_OWNER_EMAIL"
    echo "FORGE_SETUP_TOKEN=$token"
    echo "FORGE_EXTRA_DOMAINS=${FORGE_EXTRA_DOMAINS:-}"
    echo "PYTHONPATH=$STATE/current/x3-lang"
  } > "$tmp"
  mv -f "$tmp" "$ENV_FILE"
}

lock() {
  require_cf_env
  write_manifest
  check_zone
  local out team aud
  say "ensuring the owner-only Access gate for $HOST"
  out="$(cf_access ensure)" || die "could not create or verify the Access app"
  team="$(json_field "$out" teamDomain)"; aud="$(json_field "$out" aud)"
  [[ -n "$team" && -n "$aud" ]] || die "cf-access did not return a team domain and AUD"
  write_env "$team" "$aud"
  say "Access gate ready (team $team, only $X3_OWNER_EMAIL)"
}

verify_gate_before_dns() {
  require_cf_env
  local out
  out="$(cf_access verify)" || die "Access gate verification failed; refusing to point DNS at the tunnel"
  [[ "$(json_field "$out" aud)" == "$(env_get FORGE_ACCESS_AUD)" ]] || die "Access app AUD does not match the Forge's; run deploy.sh lock"
  say "Access gate verified for $HOST"
}

# ── systemd --user ───────────────────────────────────────────────────────────────────
systemd_ok() {
  systemctl --user show-environment >/dev/null 2>&1 \
    || die "systemctl --user is unavailable in this shell. Over SSH try: export XDG_RUNTIME_DIR=/run/user/$(id -u)"
}

install_units() {
  local cf
  need cloudflared; cf="$(command -v cloudflared)"
  systemd_ok
  mkdir -p "$UNIT_DIR"
  cat > "$UNIT_DIR/$WEB_UNIT" <<UNIT
[Unit]
Description=X3 Agent Forge ($HOST)
After=network-online.target

[Service]
Type=simple
EnvironmentFile=$ENV_FILE
WorkingDirectory=$STATE/current/x3-lang
ExecStart="$VENV/bin/python" -m forge.server --host 127.0.0.1 --port $PORT --data "$DATA" --secure-cookie
Restart=on-failure
RestartSec=2
NoNewPrivileges=yes
UMask=0077

[Install]
WantedBy=default.target
UNIT
  cat > "$UNIT_DIR/$TUNNEL_UNIT" <<UNIT
[Unit]
Description=Cloudflare Tunnel for the X3 Agent Forge ($HOST)
After=network-online.target $WEB_UNIT
Wants=$WEB_UNIT

[Service]
Type=simple
ExecStart="$cf" tunnel --no-autoupdate --config "$TUNNEL_CFG" run
Restart=always
RestartSec=5
NoNewPrivileges=yes

[Install]
WantedBy=default.target
UNIT
  systemctl --user daemon-reload
  if [[ "$(loginctl show-user "$USER" -p Linger --value 2>/dev/null || echo no)" != "yes" ]]; then
    warn "user lingering is off, so the Forge stops when you log out. Fix once with: sudo loginctl enable-linger $USER"
  fi
}

health_check() {
  local tries=0 code
  until curl -fsS -o /dev/null "http://127.0.0.1:$PORT/health" 2>/dev/null; do
    tries=$((tries + 1))
    [[ $tries -le 30 ]] || { journalctl --user -u "$WEB_UNIT" -n 30 --no-pager >&2 || true; die "the Forge did not come up on 127.0.0.1:$PORT"; }
    sleep 0.5
  done
  code="$(curl -s -o /dev/null -w '%{http_code}' -H "Host: $HOST" "http://127.0.0.1:$PORT/api/status")"
  [[ "$code" == "403" ]] || die "the Forge answered $code without a login; expected 403"
  say "Forge healthy on 127.0.0.1:$PORT and refusing requests without the owner's Access login"
}

restart_web() {
  systemd_ok
  systemctl --user enable "$WEB_UNIT" >/dev/null 2>&1
  systemctl --user restart "$WEB_UNIT"
  health_check
}

# ── Tunnel + DNS ────────────────────────────────────────────────────────────────────
tunnel_id() {
  cloudflared tunnel list --output json 2>/dev/null | node -e '
    let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{
      const list=JSON.parse(s||"[]").filter(t=>t.name===process.argv[1]&&(!t.deleted_at||t.deleted_at.startsWith("0001")));
      if(list.length>1){console.error("more than one tunnel named "+process.argv[1]);process.exit(4)}
      process.stdout.write(list[0]?.id||"")})' "$TUNNEL_NAME"
}

ensure_tunnel() {
  need cloudflared
  [[ -f "$CF_DIR/cert.pem" || -n "${TUNNEL_ORIGIN_CERT:-}" ]] || die "cloudflared is not logged in. Run once: cloudflared tunnel login"
  local id pinned=""
  id="$(tunnel_id)"
  if [[ -f "$TUNNEL_CFG" ]]; then
    pinned="$(sed -n 's/^tunnel: //p' "$TUNNEL_CFG")"
    [[ -n "$pinned" && "$pinned" == "$id" ]] || die "tunnel $TUNNEL_NAME does not match this deployment's saved ID"
  elif [[ -n "$id" ]]; then
    die "a tunnel named $TUNNEL_NAME already exists but was not made by this deployment; set X3_FORGE_TUNNEL to a new name"
  fi
  if [[ -z "$id" ]]; then
    say "creating tunnel $TUNNEL_NAME"
    cloudflared tunnel create "$TUNNEL_NAME" >/dev/null
    id="$(tunnel_id)"
  fi
  [[ "$id" =~ ^[0-9a-f-]{36}$ ]] || die "could not resolve tunnel id for $TUNNEL_NAME"
  [[ -f "$CF_DIR/$id.json" ]] || die "tunnel credentials $CF_DIR/$id.json not found on this machine"
  {
    echo "# Written by deploy/forge/deploy.sh. Only the Forge hostname; everything else is 404."
    echo "tunnel: $id"
    echo "credentials-file: $CF_DIR/$id.json"
    echo "ingress:"
    echo "  - hostname: $HOST"
    echo "    service: http://127.0.0.1:$PORT"
    echo "  - service: http_status:404"
  } > "$TUNNEL_CFG.new"
  cloudflared tunnel --config "$TUNNEL_CFG.new" ingress validate >/dev/null || die "cloudflared rejected the generated ingress config"
  mv -f "$TUNNEL_CFG.new" "$TUNNEL_CFG"
  TUNNEL_ID="$id"
  say "tunnel $TUNNEL_NAME ($id) configured"
}

start_tunnel() {
  systemd_ok
  systemctl --user enable "$TUNNEL_UNIT" >/dev/null 2>&1
  systemctl --user restart "$TUNNEL_UNIT"
  sleep 2
  systemctl --user is-active --quiet "$TUNNEL_UNIT" || { journalctl --user -u "$TUNNEL_UNIT" -n 20 --no-pager >&2 || true; die "tunnel service failed to start"; }
  say "tunnel running"
}

route_dns() {
  verify_gate_before_dns
  local out
  out="$(cf_dns route "$HOST" "$TUNNEL_ID")" || die "could not point $HOST at the tunnel"
  say "DNS $HOST -> tunnel $TUNNEL_NAME ($(json_field "$out" changed) change)"
}

# ── Commands ────────────────────────────────────────────────────────────────────────
cmd_up() {
  parse_ref "$@"
  need git; need curl; need cloudflared; node_bin >/dev/null; systemd_ok; require_cf_env
  build_release "$REF"
  lock
  install_units
  restart_web
  ensure_tunnel
  start_tunnel
  route_dns
  cmd_status
  echo
  say "open https://$HOST, sign in as $X3_OWNER_EMAIL, go to Birth and claim High Command with: $(env_get FORGE_SETUP_TOKEN)"
}

cmd_update() {
  parse_ref "$@"
  [[ -f "$ENV_FILE" ]] || die "nothing deployed yet; run deploy.sh up"
  build_release "$REF"
  restart_web
}

cmd_lock() {
  [[ -d "$STATE/current" ]] || die "nothing deployed yet; run deploy.sh up"
  lock
  restart_web
}

cmd_token() {
  [[ -f "$ENV_FILE" ]] || die "nothing deployed yet"
  if [[ -f "$DATA/operator.json" ]]; then
    echo "High Command is already claimed (key $(sed -n 's/.*"public_key": "\([0-9a-f]*\)".*/\1/p' "$DATA/operator.json"))."
  else
    env_get FORGE_SETUP_TOKEN
  fi
}

cmd_down() {
  systemd_ok
  systemctl --user disable --now "$TUNNEL_UNIT" "$WEB_UNIT" 2>/dev/null || true
  say "stopped. The DNS record still points at the tunnel; visitors get a Cloudflare tunnel error until 'deploy.sh up'."
}

cmd_status() {
  local line
  echo "host    : https://$HOST"
  echo "release : $(cat "$STATE/current/RELEASE" 2>/dev/null || echo none)"
  echo "mode    : $(env_get FORGE_ACCESS_MODE)"
  echo "claimed : $([[ -f "$DATA/operator.json" ]] && echo yes || echo no)"
  if systemctl --user show-environment >/dev/null 2>&1; then
    echo "web     : $(systemctl --user is-active "$WEB_UNIT" 2>/dev/null || true)"
    echo "tunnel  : $(systemctl --user is-active "$TUNNEL_UNIT" 2>/dev/null || true)"
  fi
  line="$(curl -s -o /dev/null --max-time 10 -w '%{http_code} %{redirect_url}' "https://$HOST/" || echo "unreachable")"
  case "$line" in
    *cloudflareaccess.com*) echo "https://$HOST -> login required (Access)";;
    *) echo "https://$HOST -> $line";;
  esac
}

main() {
  local cmd="${1:-}"; shift || true
  case "$cmd" in
    up) cmd_up "$@" ;;
    update) cmd_update "$@" ;;
    lock) cmd_lock ;;
    token) cmd_token ;;
    down) cmd_down ;;
    status) cmd_status ;;
    release) parse_ref "$@"; build_release "$REF" ;;
    *) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; [[ -z "$cmd" ]] || exit 1 ;;
  esac
}

main "$@"

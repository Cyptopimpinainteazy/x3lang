#!/usr/bin/env bash
# Deploy the X3Lang judge demos to *.x3star.net on this machine.
#
#   deploy.sh up [--ref REF]          first deploy: Access gate -> web server -> tunnel -> DNS
#   deploy.sh update [--ref REF]      ship a new commit to the running server (no Cloudflare calls)
#   deploy.sh status                  services, release, mode, and what each hostname returns
#   deploy.sh lock                    (re)create the owner-only Access gate and require it at the origin
#   deploy.sh go-public --yes-publish release day: serve without login and delete the Access gate
#   deploy.sh down                    stop the web server and the tunnel (DNS records stay)
#   deploy.sh release [--ref REF]     only build and switch the release directory
#
# Needs: git, node >= 20, curl, cloudflared (logged in: ~/.cloudflared/cert.pem), systemd --user.
# For up / lock / go-public: CF_API_TOKEN (Access: Apps and Policies Edit + Organizations Read),
# CF_ACCOUNT_ID and X3_OWNER_EMAIL in the environment. The token is never written to disk.
#
# Fail-closed order: the origin refuses every request without a valid Access JWT unless
# go-public was run, and DNS is only pointed at the tunnel after the Access app is verified.
set -Eeuo pipefail
trap 'echo "deploy.sh: failed at line $LINENO: $BASH_COMMAND" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE="${X3_DEMOS_HOME:-$HOME/x3lang-demos}"
PORT="${X3_DEMOS_PORT:-7410}"
TUNNEL_NAME="${X3_TUNNEL_NAME:-x3lang-demos}"
CF_DIR="${X3_CLOUDFLARED_DIR:-$HOME/.cloudflared}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
ENV_FILE="$STATE/server.env"
TUNNEL_CFG="$STATE/cloudflared.yml"
PUBLIC_MARK="$STATE/PUBLIC_APPROVED"
WEB_UNIT="x3lang-demos-web.service"
TUNNEL_UNIT="x3lang-demos-tunnel.service"
KEEP_RELEASES=5

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"; }

node_bin() {
  need node
  local major
  major="$(node -p 'process.versions.node.split(".")[0]')"
  [[ "$major" -ge 20 ]] || die "node >= 20 is required (found $(node --version))"
  command -v node
}

repo_root() {
  git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || die "deploy.sh must run from inside an x3lang git checkout"
}

current_dir() { echo "$STATE/current/deploy/x3star"; }

hosts() {
  local manifest="${1:-$(current_dir)/sites.json}"
  node -e 'const m=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));console.log(Object.keys(m.sites).sort().join("\n"))' "$manifest"
}

env_get() {
  [[ -f "$ENV_FILE" ]] || { echo ""; return; }
  sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1
}

write_env() {
  local mode="$1" team="${2:-}" aud="${3:-}" tmp
  mkdir -p "$STATE"
  tmp="$(umask 077 && mktemp "$STATE/.server.env.XXXXXX")"
  {
    echo "# Written by deploy.sh $(date -u +%Y-%m-%dT%H:%M:%SZ). Not secret, but keep it 0600."
    echo "X3_ACCESS_MODE=$mode"
    echo "X3_ACCESS_TEAM_DOMAIN=$team"
    echo "X3_ACCESS_AUD=$aud"
    echo "X3_DEMOS_PORT=$PORT"
    echo "X3_DEMOS_BIND=127.0.0.1"
  } > "$tmp"
  mv -f "$tmp" "$ENV_FILE"
}

require_cf_env() {
  [[ -n "${CF_API_TOKEN:-}" ]] || die "CF_API_TOKEN is not set (Cloudflare API token with Access: Apps and Policies Edit and Access: Organizations Read)"
  [[ -n "${CF_ACCOUNT_ID:-}" ]] || die "CF_ACCOUNT_ID is not set"
  [[ -n "${X3_OWNER_EMAIL:-}" ]] || die "X3_OWNER_EMAIL is not set (the only address allowed through the login)"
}

cf_access() {
  X3_DEMOS_MANIFEST="$(current_dir)/sites.json" "$(node_bin)" "$(current_dir)/cf-access.mjs" "$@"
}

json_field() { node -e 'const o=JSON.parse(process.argv[1]);process.stdout.write(String(o[process.argv[2]]??""))' "$1" "$2"; }

parse_ref() {
  REF="HEAD"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --ref) [[ $# -ge 2 ]] || die "--ref needs a value"; REF="$2"; shift 2 ;;
      *) die "unknown option: $1" ;;
    esac
  done
}

# ── Release: only the files listed in sites.json, from one exact commit ───────────────
build_release() {
  local ref="$1" repo sha dest tmp manifest
  repo="$(repo_root)"
  need tar
  sha="$(git -C "$repo" rev-parse --verify --quiet "${ref}^{commit}")" || die "unknown git ref: $ref"
  mkdir -p "$STATE/releases"
  dest="$STATE/releases/$sha"
  if [[ ! -f "$dest/RELEASE" ]]; then
    manifest="$(mktemp)"
    git -C "$repo" show "$sha:deploy/x3star/sites.json" > "$manifest" || die "$sha has no deploy/x3star/sites.json"
    mapfile -t files < <(node -e 'const m=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));for(const f of m.files)console.log(f)' "$manifest")
    rm -f "$manifest"
    files+=(deploy/x3star/server.mjs deploy/x3star/cf-access.mjs deploy/x3star/sites.json)
    tmp="$(mktemp -d "$STATE/releases/.tmp.XXXXXX")"
    git -C "$repo" archive --format=tar "$sha" -- "${files[@]}" | tar -x -C "$tmp" \
      || { rm -rf "$tmp"; die "could not export the listed files from $sha (is every file in sites.json committed?)"; }
    for f in "${files[@]}"; do [[ -f "$tmp/$f" ]] || { rm -rf "$tmp"; die "release is missing $f"; }; done
    echo "$sha" > "$tmp/RELEASE"
    chmod -R go-w "$tmp"
    rm -rf "$dest"
    mv "$tmp" "$dest"
  fi
  ln -sfn "releases/$sha" "$STATE/current.new"
  mv -Tf "$STATE/current.new" "$STATE/current"
  # keep the newest releases, never the one in use
  local n=0 r
  while IFS= read -r r; do
    n=$((n + 1))
    [[ $n -le $KEEP_RELEASES || "$r" == "$sha" ]] && continue
    rm -rf "${STATE:?}/releases/$r"
  done < <(find "$STATE/releases" -mindepth 1 -maxdepth 1 -type d -regextype posix-extended -regex '.*/[0-9a-f]{40}' -printf '%T@ %f\n' | sort -rn | cut -d' ' -f2)
  say "release ${sha:0:12} -> $STATE/current (${#files[@]} files)"
}

# ── systemd --user units ─────────────────────────────────────────────────────────────
systemd_ok() {
  systemctl --user show-environment >/dev/null 2>&1 \
    || die "systemctl --user is unavailable in this shell. Over SSH try: export XDG_RUNTIME_DIR=/run/user/$(id -u)"
}

install_units() {
  local node cf
  node="$(node_bin)"; need cloudflared; cf="$(command -v cloudflared)"
  systemd_ok
  mkdir -p "$UNIT_DIR"
  cat > "$UNIT_DIR/$WEB_UNIT" <<UNIT
[Unit]
Description=X3Lang demos web server (x3star.net)
After=network-online.target

[Service]
Type=simple
EnvironmentFile=$ENV_FILE
ExecStart="$node" "$STATE/current/deploy/x3star/server.mjs"
Restart=on-failure
RestartSec=2
NoNewPrivileges=yes
UMask=0077

[Install]
WantedBy=default.target
UNIT
  cat > "$UNIT_DIR/$TUNNEL_UNIT" <<UNIT
[Unit]
Description=Cloudflare Tunnel for the X3Lang demos (x3star.net)
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
    warn "user lingering is off, so these services stop when you log out. Fix once with: sudo loginctl enable-linger $USER"
  fi
}

restart_web() {
  systemd_ok
  systemctl --user enable "$WEB_UNIT" >/dev/null 2>&1
  systemctl --user restart "$WEB_UNIT"
  health_check
}

# The origin must answer /healthz, and every site must refuse a request without an Access JWT
# (access mode) or redirect to its entry page (public mode).
health_check() {
  local mode code h tries=0
  mode="$(env_get X3_ACCESS_MODE)"
  until curl -fsS -o /dev/null "http://127.0.0.1:$PORT/healthz" 2>/dev/null; do
    tries=$((tries + 1))
    [[ $tries -le 20 ]] || { journalctl --user -u "$WEB_UNIT" -n 20 --no-pager >&2 || true; die "web server did not come up on 127.0.0.1:$PORT"; }
    sleep 0.5
  done
  while IFS= read -r h; do
    code="$(curl -s -o /dev/null -w '%{http_code}' -H "Host: $h" "http://127.0.0.1:$PORT/")"
    if [[ "$mode" == "access" ]]; then
      [[ "$code" == "403" ]] || die "$h answered $code without a login; expected 403 in access mode"
    else
      [[ "$code" == "200" || "$code" == "302" ]] || die "$h answered $code; expected its home page (200 or 302)"
    fi
  done < <(hosts)
  say "web server healthy on 127.0.0.1:$PORT (mode=$mode)"
}

# ── Cloudflare Access ───────────────────────────────────────────────────────────────
lock() {
  require_cf_env
  local out team aud
  say "ensuring the owner-only Access gate for: $(hosts | paste -sd' ' -)"
  out="$(cf_access ensure)"
  team="$(json_field "$out" teamDomain)"; aud="$(json_field "$out" aud)"
  [[ -n "$team" && -n "$aud" ]] || die "cf-access did not return a team domain and AUD"
  write_env access "$team" "$aud"
  rm -f "$PUBLIC_MARK"
  say "Access gate ready (team $team, only ${X3_OWNER_EMAIL})"
}

verify_gate_before_dns() {
  local mode out aud
  mode="$(env_get X3_ACCESS_MODE)"
  if [[ "$mode" == "public" ]]; then
    [[ -f "$PUBLIC_MARK" ]] || die "server env says public but go-public was never approved; run deploy.sh lock"
    return
  fi
  [[ "$mode" == "access" ]] || die "server env has no access mode; run deploy.sh lock first"
  require_cf_env
  out="$(cf_access verify)" || die "Access gate verification failed; refusing to point DNS at the tunnel"
  aud="$(json_field "$out" aud)"
  [[ "$aud" == "$(env_get X3_ACCESS_AUD)" ]] || die "Access app AUD does not match the server's X3_ACCESS_AUD; run deploy.sh lock"
  say "Access gate verified for every hostname"
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
  local id h
  id="$(tunnel_id)"
  if [[ -z "$id" ]]; then
    say "creating tunnel $TUNNEL_NAME"
    cloudflared tunnel create "$TUNNEL_NAME" >/dev/null
    id="$(tunnel_id)"
  fi
  [[ "$id" =~ ^[0-9a-f-]{36}$ ]] || die "could not resolve tunnel id for $TUNNEL_NAME"
  [[ -f "$CF_DIR/$id.json" ]] || die "tunnel credentials $CF_DIR/$id.json not found on this machine (was $TUNNEL_NAME created elsewhere?)"
  {
    echo "# Written by deploy.sh. Only the X3Lang demo hostnames; everything else is 404."
    echo "tunnel: $id"
    echo "credentials-file: $CF_DIR/$id.json"
    echo "ingress:"
    while IFS= read -r h; do
      echo "  - hostname: $h"
      echo "    service: http://127.0.0.1:$PORT"
    done < <(hosts)
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
  local h
  while IFS= read -r h; do
    cloudflared tunnel route dns --overwrite-dns "$TUNNEL_ID" "$h" >/dev/null
    say "DNS $h -> tunnel $TUNNEL_NAME"
  done < <(hosts)
}

# ── Commands ────────────────────────────────────────────────────────────────────────
cmd_up() {
  parse_ref "$@"
  need git; need curl; node_bin >/dev/null; need cloudflared; systemd_ok
  [[ "$(env_get X3_ACCESS_MODE)" == "public" && -f "$PUBLIC_MARK" ]] || require_cf_env
  build_release "$REF"
  if [[ "$(env_get X3_ACCESS_MODE)" == "public" && -f "$PUBLIC_MARK" ]]; then
    say "keeping public mode (approved $(cat "$PUBLIC_MARK"))"
  else
    lock
  fi
  install_units
  restart_web
  ensure_tunnel
  start_tunnel
  route_dns
  cmd_status
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

cmd_go_public() {
  [[ "${1:-}" == "--yes-publish" ]] || die "this makes every demo page public. Re-run as: deploy.sh go-public --yes-publish"
  [[ -d "$STATE/current" ]] || die "nothing deployed yet; run deploy.sh up"
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) by $USER at $(cat "$STATE/current/RELEASE")" > "$PUBLIC_MARK"
  write_env public
  restart_web
  if [[ -n "${CF_API_TOKEN:-}" && -n "${CF_ACCOUNT_ID:-}" ]]; then
    cf_access remove >/dev/null
    say "Access gate removed; the demos are public"
  else
    warn "origin is public, but the Access app still asks for login. Remove it with CF_API_TOKEN and CF_ACCOUNT_ID set: deploy.sh go-public --yes-publish"
  fi
}

cmd_down() {
  systemd_ok
  systemctl --user disable --now "$TUNNEL_UNIT" "$WEB_UNIT" 2>/dev/null || true
  say "stopped. DNS records still point at the tunnel; visitors get a Cloudflare tunnel error until 'deploy.sh up'."
}

cmd_status() {
  local h line
  echo "release : $(cat "$STATE/current/RELEASE" 2>/dev/null || echo none)"
  echo "mode    : $(env_get X3_ACCESS_MODE || true)"
  if systemctl --user show-environment >/dev/null 2>&1; then
    echo "web     : $(systemctl --user is-active "$WEB_UNIT" 2>/dev/null || true)"
    echo "tunnel  : $(systemctl --user is-active "$TUNNEL_UNIT" 2>/dev/null || true)"
  fi
  [[ -d "$STATE/current" ]] || return 0
  while IFS= read -r h; do
    line="$(curl -s -o /dev/null --max-time 10 -w '%{http_code} %{redirect_url}' "https://$h/" || echo "unreachable")"
    case "$line" in
      *cloudflareaccess.com*) echo "https://$h  -> login required (Access)";;
      200\ *|302\ *) echo "https://$h  -> public";;
      *) echo "https://$h  -> $line";;
    esac
  done < <(hosts)
}

main() {
  local cmd="${1:-}"; shift || true
  case "$cmd" in
    up) cmd_up "$@" ;;
    update) cmd_update "$@" ;;
    lock) cmd_lock ;;
    go-public) cmd_go_public "$@" ;;
    down) cmd_down ;;
    status) cmd_status ;;
    release) parse_ref "$@"; node_bin >/dev/null; build_release "$REF" ;;
    *) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; [[ -z "$cmd" ]] || exit 1 ;;
  esac
}

main "$@"

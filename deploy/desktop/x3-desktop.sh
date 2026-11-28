#!/usr/bin/env bash
# X3 on your own machine: Agent Forge + GPU mesh coordinator + this machine's GPU as a fleet worker.
#
#   deploy/desktop/x3-desktop.sh install [--ref BRANCH] [--pull-model]   # into $X3_HOME (default ~/Desktop/x3)
#   deploy/desktop/x3-desktop.sh start | stop | status | smoke | logs
#
# Everything binds to 127.0.0.1. Nothing here touches forge.dotx3.net, x3star1 or any chain.
# Keys live under $X3_HOME/keys (mode 0600). The Forge app key for the mesh is minted locally,
# and is printed once so you can paste it into the Comms Rig.
set -euo pipefail

X3_HOME="${X3_HOME:-$HOME/Desktop/x3}"
REPO="${X3_SRC_REPO:-https://github.com/Cyptopimpinainteazy/x3lang.git}"
REF="${X3_REF:-feat/forge-mesh-runner}"
MODEL="${X3_MODEL:-qwen3:8b}"
OLLAMA="${X3_OLLAMA_URL:-http://127.0.0.1:11434}"
FORGE_PORT="${X3_FORGE_PORT:-7420}"
MESH_PORT="${X3_MESH_PORT:-8790}"

SRC="$X3_HOME/src/x3lang"
VENV="$X3_HOME/venv"
PY="$VENV/bin/python"
KEYS="$X3_HOME/keys"
DATA="$X3_HOME/data"
LOGS="$X3_HOME/logs"
RUN="$X3_HOME/run"

say() { printf '\033[1;36mx3\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mx3\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31mx3\033[0m %s\n' "$*" >&2; exit 1; }

version_ge() { # version_ge 3.11.2 3.10 → true
  [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}

mesh() { (cd "$SRC/x3-lang" && PYTHONPATH=. "$PY" -m gpu_mesh "$@"); }

check_prereqs() {
  local ok=1 v
  command -v git >/dev/null || { warn "git is missing"; ok=0; }
  command -v curl >/dev/null || { warn "curl is missing"; ok=0; }
  if command -v python3 >/dev/null; then
    v=$(python3 -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])')
    if version_ge "$v" 3.10; then say "python $v"; else warn "python $v: need 3.10+"; ok=0; fi
  else warn "python3 is missing (3.10+)"; ok=0; fi
  if command -v node >/dev/null; then
    v=$(node -v | tr -d v)
    if { version_ge "$v" 20.19 && ! version_ge "$v" 21; } || version_ge "$v" 22.12; then say "node $v"
    else warn "node $v: need 20.19+ or 22.12+"; ok=0; fi
  else warn "node is missing (20.19+ or 22.12+)"; ok=0; fi
  if curl -fsS -m 3 "$OLLAMA/api/tags" >/dev/null 2>&1; then say "ollama reachable at $OLLAMA"
  else warn "no Ollama at $OLLAMA: the Forge and coordinator still run, but this machine won't lend a GPU"; fi
  if command -v nvidia-smi >/dev/null; then
    say "GPU: $(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader 2>/dev/null | head -n1)"
  fi
  [ "$ok" = 1 ] || die "fix the items above, then run install again"
}

install() {
  local pull=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --ref) REF="$2"; shift 2 ;;
      --pull-model) pull=1; shift ;;
      *) die "unknown install option: $1" ;;
    esac
  done
  check_prereqs
  mkdir -p "$X3_HOME/src" "$DATA/forge" "$DATA/mesh" "$LOGS" "$RUN"
  mkdir -p "$KEYS" && chmod 700 "$KEYS"
  if [ -d "$SRC/.git" ]; then
    say "updating $SRC"
    git -C "$SRC" fetch --quiet origin "$REF"
  else
    say "cloning $REPO"
    git clone --quiet "$REPO" "$SRC"
    git -C "$SRC" fetch --quiet origin "$REF"
  fi
  git -C "$SRC" checkout --quiet --detach FETCH_HEAD
  say "source at $(git -C "$SRC" rev-parse --short HEAD) ($REF)"

  [ -x "$PY" ] || python3 -m venv "$VENV"
  "$PY" -m pip install --quiet --upgrade pip
  "$PY" -m pip install --quiet -r "$SRC/x3-lang/requirements.txt"
  say "python deps installed"

  (cd "$SRC/x3-lang/forge/web" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --silent && npm run --silent build >/dev/null)
  say "Forge web app built"

  [ -f "$KEYS/coordinator.key" ] || mesh keygen --out "$KEYS/coordinator.key" >/dev/null
  if [ ! -f "$KEYS/worker.key" ]; then
    mesh keygen --out "$KEYS/worker.key" >/dev/null
    mesh admin enroll-fleet --db "$DATA/mesh/mesh.db" "$(mesh pubkey --key "$KEYS/worker.key")" --label "$(hostname -s)-desktop" >/dev/null
  fi
  if [ ! -f "$KEYS/forge-app.key" ]; then
    (umask 077; mesh admin app-add --db "$DATA/mesh/mesh.db" forge --rpm 120 --daily-tokens 2000000 --max-tokens 8192 \
      2>/dev/null >"$KEYS/forge-app.key")
    say "minted the Forge's mesh app key: $KEYS/forge-app.key"
  fi
  chmod 600 "$KEYS"/*

  if [ "$pull" = 1 ] && command -v ollama >/dev/null; then
    say "pulling $MODEL (this can take a while)"
    ollama pull "$MODEL"
  fi
  say "installed in $X3_HOME. Next: $0 start"
}

pidfile() { echo "$RUN/$1.pid"; }
running() { local p; p=$(pidfile "$1"); [ -f "$p" ] && kill -0 "$(cat "$p")" 2>/dev/null; }

launch() { # launch NAME CMD...
  local name="$1"; shift
  if running "$name"; then say "$name already running (pid $(cat "$(pidfile "$name")"))"; return; fi
  nohup "$@" >>"$LOGS/$name.log" 2>&1 &
  echo $! >"$(pidfile "$name")"
  say "$name started (pid $!), log: $LOGS/$name.log"
}

wait_http() { # wait_http URL SECONDS
  local i
  for i in $(seq "$(( $2 * 4 ))"); do
    curl -fsS -m 2 -o /dev/null "$1" 2>/dev/null && return 0
    [ "$i" -gt 0 ] && sleep 0.25
  done
  return 1
}

start() {
  [ -x "$PY" ] || die "not installed: run $0 install"
  launch coordinator env PYTHONPATH="$SRC/x3-lang" "$PY" -m gpu_mesh coordinator serve \
    --db "$DATA/mesh/mesh.db" --key "$KEYS/coordinator.key" --host 127.0.0.1 --port "$MESH_PORT"
  wait_http "http://127.0.0.1:$MESH_PORT/healthz" 20 || die "coordinator did not come up; see $LOGS/coordinator.log"
  if curl -fsS -m 3 "$OLLAMA/api/tags" >/dev/null 2>&1; then
    launch worker env PYTHONPATH="$SRC/x3-lang" "$PY" -m gpu_mesh worker run --key "$KEYS/worker.key" \
      --coordinator "http://127.0.0.1:$MESH_PORT" --ollama "$OLLAMA" --label "$(hostname -s)-desktop" --models "$MODEL"
  else
    warn "no Ollama at $OLLAMA: worker not started"
  fi
  launch forge env PYTHONPATH="$SRC/x3-lang" FORGE_ACCESS_MODE=local "$PY" -m forge.server \
    --host 127.0.0.1 --port "$FORGE_PORT" --data "$DATA/forge"
  wait_http "http://127.0.0.1:$FORGE_PORT/health" 20 || die "Forge did not come up; see $LOGS/forge.log"
  status
}

stop() {
  local n
  for n in forge worker coordinator; do
    if running "$n"; then kill "$(cat "$(pidfile "$n")")" && say "$n stopped"; fi
    rm -f "$(pidfile "$n")"
  done
}

status() {
  local n token
  for n in coordinator worker forge; do
    if running "$n"; then say "$n: running (pid $(cat "$(pidfile "$n")"))"; else say "$n: stopped"; fi
  done
  say "Forge:        http://127.0.0.1:$FORGE_PORT"
  say "GPU mesh:     http://127.0.0.1:$MESH_PORT  (Comms Rig → X3 GPU mesh → Coordinator URL)"
  [ -f "$KEYS/forge-app.key" ] && say "mesh app key: $KEYS/forge-app.key  (paste into the Comms Rig key field)"
  if [ -f "$LOGS/forge.log" ]; then
    token=$(grep -o 'One-time setup token: [A-Za-z0-9_-]*' "$LOGS/forge.log" | tail -n1 | awk '{print $NF}' || true)
    if [ -n "$token" ] && [ ! -f "$DATA/forge/operator.json" ]; then say "High Command setup token: $token"; fi
  fi
  if [ -x "$PY" ] && [ -f "$DATA/mesh/mesh.db" ]; then mesh admin list --db "$DATA/mesh/mesh.db" 2>/dev/null || true; fi
}

smoke() {
  local key out
  key=$(cat "$KEYS/forge-app.key")
  wait_http "http://127.0.0.1:$MESH_PORT/healthz" 5 || die "coordinator is not running"
  out=$(curl -fsS -m 200 "http://127.0.0.1:$MESH_PORT/v1/chat/completions" -H "Authorization: Bearer $key" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"$MODEL\",\"max_tokens\":32,\"messages\":[{\"role\":\"user\",\"content\":\"Reply with the single word: ready\"}]}") \
    || die "the mesh did not answer (is the worker up and $MODEL pulled?)"
  printf '%s\n' "$out" | "$PY" -c 'import json,sys; d=json.load(sys.stdin); print("model said:", d["choices"][0]["message"]["content"].strip()[:200]); m=d.get("x3_mesh") or {}; print("served by worker", m.get("worker","?")[:16], "(%s), job %s" % (m.get("worker_tier","?"), m.get("job_id","?")))'
  wait_http "http://127.0.0.1:$FORGE_PORT/health" 5 || die "Forge is not running"
  say "smoke ok: Forge up, mesh answered with your app key"
}

case "${1:-}" in
  install) shift; install "$@" ;;
  start) start ;;
  stop) stop ;;
  restart) stop; start ;;
  status) status ;;
  smoke) smoke ;;
  logs) tail -n 50 -F "$LOGS"/*.log ;;
  *) sed -n '2,9p' "$0"; exit 2 ;;
esac

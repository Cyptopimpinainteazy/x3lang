#!/usr/bin/env bash
# Ensure NEAR Guard never depends on the abandoned wasm32 wee_alloc backend.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo/x3-lang/near/x3-guard"
# Resolve the graph outside an if/pipeline: those contexts suppress errexit
# and could turn a failed cargo invocation into a successful allocator check.
dependency_tree="$(cargo tree --target wasm32-unknown-unknown --locked --prefix none)"
if [[ -z "${dependency_tree//[[:space:]]/}" ]]; then
  echo 'ERROR: cargo returned an empty wasm32 dependency graph' >&2
  exit 1
fi
if grep -Eq '^wee_alloc v' <<< "$dependency_tree"; then
  echo 'ERROR: NEAR Guard still includes unmaintained wee_alloc on wasm32' >&2
  exit 1
else
  inspection_status=$?
  if [[ "$inspection_status" -ne 1 ]]; then
    echo 'ERROR: failed to inspect wasm32 dependency graph' >&2
    exit "$inspection_status"
  fi
fi
# An absent, unreadable or blank lockfile is likewise an inspection failure.
locked_dependencies="$(cat Cargo.lock)"
if [[ -z "${locked_dependencies//[[:space:]]/}" ]]; then
  echo 'ERROR: Cargo.lock is empty' >&2
  exit 1
fi
if grep -Eq '^[[:space:]]*name[[:space:]]*=[[:space:]]*"wee_alloc"[[:space:]]*(#.*)?$' <<< "$locked_dependencies"; then
  echo 'ERROR: NEAR Guard Cargo.lock still resolves wee_alloc' >&2
  exit 1
else
  inspection_status=$?
  if [[ "$inspection_status" -ne 1 ]]; then
    echo 'ERROR: failed to inspect Cargo.lock' >&2
    exit "$inspection_status"
  fi
fi
echo 'OK: no wee_alloc in wasm32 dependency graph or locked dependency set'

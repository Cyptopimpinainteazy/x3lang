#!/usr/bin/env bash
# Reproduce exactly the deployed Guard bytecode from the source it was built from.
# No keys, wallet, testnet transaction, or external blockchain RPC calls.
#
# The deployed binary is pinned to a reviewed source commit, not to whatever the
# current branch contains. Building HEAD would make every Guard source change fail
# this proof even though the deployed bytes never changed. The source is taken from
# Git history with `git archive`, so the working tree cannot influence the build.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# Reviewed release source of the deployed WASM (GUARD_WASM_BYTE_REPRODUCIBILITY.md).
DEPLOYED_SOURCE_COMMIT='800a077eec0b36bc66d7115995cb6903fa9d4f0a'
if ! git cat-file -e "${DEPLOYED_SOURCE_COMMIT}^{commit}" 2>/dev/null; then
  echo "Deployed Guard source commit $DEPLOYED_SOURCE_COMMIT is not in this clone's history." >&2
  echo 'Fetch full history (actions/checkout fetch-depth: 0, or git fetch --unshallow) and retry.' >&2
  exit 2
fi

if [[ -n "${RUSTFLAGS:-}" || -n "${CARGO_ENCODED_RUSTFLAGS:-}" || -n "${RUSTC_WRAPPER:-}" ]]; then
  echo 'Refusing ambient RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS/RUSTC_WRAPPER: exact-byte build must control compiler flags' >&2
  exit 2
fi
version="$(rustc -vV | sed -n 's/^release: //p')"
commit="$(rustc -vV | sed -n 's/^commit-hash: //p')"
if [[ "$version" != '1.99.1' || "$commit" != '58cf8f9c35b355199727627d07ae34a4e2846d67' ]]; then
  echo "Pinned Rust 1.99.1 (58cf8f9c35...) required, got release=$version commit=$commit" >&2
  exit 2
fi

sysroot="$(rustc --print sysroot)"
if [[ ! -f "$sysroot/lib/rustlib/src/rust/library/core/src/lib.rs" ]]; then
  echo 'Pinned Rust toolchain must include the rust-src component for exact deployed byte reproduction' >&2
  exit 2
fi
# The deployed binary also records absolute Cargo registry source paths.
# Its canonical registry path came from the reviewed x3star1 build. A fresh
# CI runner lives under /home/runner, so omitting this mapping changes 24
# bytes of data strings and offsets in the code section. Keep the exact SHA
# check mandatory: this remaps only build-time source locations.
cargo_home="${CARGO_HOME:-${HOME:-/tmp}/.cargo}"
if [[ "$cargo_home" != /* ]]; then
  echo 'CARGO_HOME must be an absolute path for the pinned source remapping' >&2
  exit 2
fi
export RUSTFLAGS="--remap-path-prefix=$sysroot/lib/rustlib/src/rust=/rustc/$commit --remap-path-prefix=$cargo_home/registry/src=/home/lojak/.cargo/registry/src"
export CARGO_TARGET_DIR="${X3_GUARD_REPRO_TARGET:-${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/x3lang/guard-wasm-repro-1.99.1}"

SOURCE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/x3-guard-deployed-src.XXXXXX")"
trap 'rm -rf "$SOURCE_DIR"' EXIT
git archive --format=tar "$DEPLOYED_SOURCE_COMMIT" x3-lang/near/x3-guard | tar -x -C "$SOURCE_DIR"
# This legacy binary was built WITHOUT `--cfg near` (see check_guard_wasm_host_panics.py).
# RUSTFLAGS above replaces any .cargo/config.toml rustflags, so later source cannot change it.
cargo build --locked --release --target wasm32-unknown-unknown \
  --manifest-path "$SOURCE_DIR/x3-lang/near/x3-guard/Cargo.toml"
python3 "$ROOT/scripts/hackathon/verify_guard_wasm_bytes.py" \
  --manifest "$ROOT/evidence/near-guard-testnet/manifest.json" \
  --wasm "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/x3_guard.wasm"

#!/usr/bin/env bash
# Local fixture integration, not a live finality verifier or broadcast.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cargo test -p x3-lang-compiler --test settlement_capability --locked --offline native_vectors_and_blocked_execution -- --nocapture > "$tmp/vectors.log" 2>&1 || { cat "$tmp/vectors.log"; exit 1; }
for key in BINDING WINDOW LOCK CLAIM REFUND; do
    value="$(sed -n "s/^PIR07_$key=//p" "$tmp/vectors.log")"
    [[ "$value" =~ ^0x[0-9a-f]{64}$ ]]
    export "PIR07_$key=$value"
done
mkdir -p "$tmp/src" "$tmp/test"
cp bridge/bsc/src/NearSettledBnbEscrow.sol bridge/bsc/src/INearSettlementVerifier.sol "$tmp/src/"
cp bridge/bsc/test/NearSettledBnbEscrow.t.sol bridge/bsc/test/NearSettlementVerifierDouble.sol "$tmp/test/"
cp bridge/bsc/foundry.toml "$tmp/foundry.toml"
cp integration/pir07/PirSettlementCompiler.t.sol "$tmp/test/"
"${FORGE:-forge}" test --root "$tmp" -vvv

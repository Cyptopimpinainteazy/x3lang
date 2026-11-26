#!/usr/bin/env bash
# Local integration only. Never broadcasts. Requires a reviewed checkout of PR #279.
set -euo pipefail
cd "$(dirname "$0")/.."
executor_checkout="$(realpath "$1")"
pin=a1d5a5f35cb6687e2114617cdd413cc42386f1c6
test "$(git -C "$executor_checkout" rev-parse HEAD)" = "$pin"
forge_bin="${FORGE:-forge}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/src" "$tmp/test"
base=x3-lang/contract-control/evm
git -C "$executor_checkout" show "$pin:$base/src/X3PolicyExecutor.sol" > "$tmp/src/X3PolicyExecutor.sol"
git -C "$executor_checkout" show "$pin:$base/foundry.toml" > "$tmp/foundry.toml"
cp integration/pir06/PirCompilerExecutor.t.sol "$tmp/test/"
cargo build -p x3-lang-compiler --example pir_evm_action --locked --offline
data=0x60fe47b10000000000000000000000000000000000000000000000000000000000000007
proposal="$(target/debug/examples/pir_evm_action sandbox:pir06 100 2 1 "$data" < compiler/tests/fixtures/pir_evm_action.x3)"
export X3_PIR_ACTION="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["abi"])' <<< "$proposal")"
export X3_PIR_ACTION_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["action_id"])' <<< "$proposal")"
printf 'Executor pin: %s\nCompiler proposal: %s\n' "$pin" "$proposal"
"$forge_bin" test --root "$tmp" --match-contract PirCompilerExecutorTest -vvv

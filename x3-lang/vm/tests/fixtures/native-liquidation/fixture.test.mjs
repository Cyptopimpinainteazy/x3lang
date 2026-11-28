import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { startFixture } from "./fixture.mjs";

test("real Morpho callback liquidation repays, transfers owner profit, and charges measured local gas", { timeout: 60_000 }, async () => {
  const fixture = await startFixture();
  try {
    const result = await fixture.executeAndMeasure();
    assert.equal(result.receipt.status, "success");
    assert.equal(result.owner_profit_loan, fixture.manifest.gross_profit);
    assert.equal(result.repaid_loan, fixture.manifest.repay_assets);
    assert.ok(BigInt(result.owner_profit_loan) > 3_080_000_000n);
    assert.ok(BigInt(result.owner_profit_loan) < 3_100_000_000n);
    assert.ok(BigInt(result.gas_paid_wei) > 0n);
    assert.equal(result.gas_paid_wei, result.keeper_native_spent_wei);
    assert.equal(result.after.executor_loan, "0");
    assert.equal(result.after.executor_collateral, "0");
    assert.equal(result.after.router_allowance, "0");
    assert.equal(result.after.morpho_allowance, "0");
    assert.ok(BigInt(result.after.borrow_shares) < BigInt(result.before.borrow_shares));
  } finally {
    await fixture.close();
  }
});

const assertAssetRollback = (result) => {
  assert.equal(result.receipt.status, "reverted");
  for (const key of Object.keys(result.before)) {
    if (!["keeper_native", "keeper_nonce"].includes(key)) assert.equal(result.after[key], result.before[key], `rollback ${key}`);
  }
  assert.equal(result.owner_profit_loan, "0");
  assert.equal(result.repaid_loan, "0");
  assert.equal(result.morpho_events.length, 0);
  assert.ok(BigInt(result.gas_paid_wei) > 0n, "a reverted local transaction still costs gas");
  assert.equal(result.gas_paid_wei, result.keeper_native_spent_wei);
  assert.equal(result.after.keeper_nonce, result.before.keeper_nonce + 1);
};

test("callback settlement failure rolls back a completed token swap and Morpho debt mutation", { timeout: 60_000 }, async () => {
  const fixture = await startFixture();
  try {
    await fixture.swapFail();
    const result = await fixture.executeAndMeasure();
    assertAssetRollback(result);
    assert.equal(result.revert_error, "SwapShort");
    assert.ok(result.trace_calls.some((call) => call.role === "mock DEX" && call.error === null), "DEX call succeeded before callback refused settlement");
  } finally { await fixture.close(); }
});

test("a reverting DEX call in the Morpho callback rolls back seized collateral and temporary approvals", { timeout: 60_000 }, async () => {
  const fixture = await startFixture();
  try {
    const result = await fixture.executeAndMeasure({ swapCallFails: true });
    assertAssetRollback(result);
    assert.equal(result.revert_error, "SwapFailed");
    assert.ok(result.trace_calls.some((call) => call.role === "mock DEX" && call.error !== null));
  } finally { await fixture.close(); }
});

test("serve protocol configures and funds a new keeper without exposing keys and reset preserves that keeper", { timeout: 120_000 }, async () => {
  const child = spawn(process.execPath, [fileURLToPath(new URL("./fixture.mjs", import.meta.url)), "--serve"],
    { stdio: ["pipe", "pipe", "pipe"] });
  let stderr = "";
  child.stderr.on("data", (bytes) => { stderr += bytes; });
  const output = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  async function next() {
    let timer;
    try {
      const line = await Promise.race([output.next(), new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("fixture protocol timeout: " + stderr)), 90_000);
      })]);
      assert.equal(line.done, false, stderr);
      assert.doesNotMatch(line.value, /Private Keys|Mnemonic|private[_ -]?key/i);
      return JSON.parse(line.value);
    } finally { clearTimeout(timer); }
  }
  async function command(request) { child.stdin.write(JSON.stringify(request) + "\n"); return next(); }
  try {
    const manifest = await next();
    assert.equal(manifest.chain_id, 31337);
    assert.match(manifest.rpc_url, /^http:\/\/127\.0\.0\.1:\d+$/);
    assert.match(manifest.executor_code_hash, /^0x[0-9a-f]{64}$/);
    assert.equal(manifest.owner, manifest.owner.toLowerCase());
    const keeper = "0x9999999999999999999999999999999999999999";
    const changed = await command({ op: "setKeeper", address: keeper });
    assert.equal(changed.ok, true, changed.error);
    assert.equal(changed.receipt.status, "success");
    assert.equal(changed.manifest.keeper, keeper);
    const state = await command({ op: "state" });
    assert.equal(state.ok, true);
    assert.equal(state.keeper_nonce, 0);
    assert.equal(state.keeper_native, "1000000000000000000000");
    const restored = await command({ op: "reset" });
    assert.equal(restored.ok, true, restored.error);
    assert.equal(restored.manifest.keeper, keeper);
    assert.equal(restored.state.borrow_shares, state.borrow_shares);
    assert.equal(restored.state.keeper_nonce, 0);
    assert.equal((await command({ op: "stop" })).ok, true);
  } finally {
    child.stdin.end();
    if (child.exitCode === null) await new Promise((resolve) => {
      const timer = setTimeout(() => child.kill("SIGTERM"), 5000);
      child.once("exit", () => { clearTimeout(timer); resolve(); });
    });
  }
});

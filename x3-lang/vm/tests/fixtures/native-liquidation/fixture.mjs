/** Local Anvil only. Genuine pinned Morpho/executor; ERC20, oracle and DEX are explicit mocks. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { createInterface } from "node:readline";
import {
  createPublicClient, createWalletClient, decodeErrorResult, decodeEventLog, encodeAbiParameters,
  encodeFunctionData, http, isAddress, keccak256, zeroAddress,
} from "viem";
import { loadArtifacts } from "./pins.mjs";

const E6 = 10n ** 6n, SAT = 10n ** 8n, WAD = 10n ** 18n, SCALE = 10n ** 36n;
const LLTV = 86n * 10n ** 16n;
const LIF = WAD * WAD / (WAD - 3n * 10n ** 17n * (WAD - LLTV) / WAD);
const price = (usd) => BigInt(usd) * E6 * SCALE / SAT;
const rate = (usd, slipBps) => (BigInt(usd) * E6 * WAD / SAT) * (10_000n - BigInt(slipBps)) / 10_000n;
const json = (value) => JSON.stringify(value, (_, item) => typeof item === "bigint" ? item.toString() : item);
const lower = (address) => address.toLowerCase();
const ownedChildren = new Set();

async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

export async function startFixture() {
  const artifacts = loadArtifacts(), art = artifacts.contracts;
  const port = await freePort();
  const rpcUrl = `http://127.0.0.1:${port}`;
  const child = spawn(process.env.X3_ANVIL_BIN ?? "anvil", [
    "--host", "127.0.0.1", "--port", String(port), "--chain-id", "31337",
    "--timestamp", "1800000000", "--hardfork", "cancun", "--threads", "1", "--silent",
  ], { stdio: ["ignore", "ignore", "pipe"], env: { ...process.env, RAYON_NUM_THREADS: "1", TOKIO_WORKER_THREADS: "1" } });
  ownedChildren.add(child);
  let stderr = "";
  child.stderr.on("data", (bytes) => { stderr = (stderr + bytes).slice(-1200); });
  let closed = false, sequence = 0;
  async function close() {
    if (closed) return;
    closed = true;
    ownedChildren.delete(child);
    if (child.exitCode !== null || child.signalCode !== null) return;
    await new Promise((resolve) => {
      const timer = setTimeout(() => child.kill("SIGKILL"), 3000);
      child.once("exit", () => { clearTimeout(timer); resolve(); });
      child.kill("SIGTERM");
    });
  }
  async function rpc(method, params = []) {
    const response = await fetch(rpcUrl, { method: "POST", headers: { "content-type": "application/json" },
      body: json({ jsonrpc: "2.0", id: ++sequence, method, params }), signal: AbortSignal.timeout(10_000) });
    if (!response.ok) throw new Error(`local fixture RPC HTTP ${response.status}`);
    const reply = await response.json();
    if (reply.error) throw Object.assign(new Error(`local fixture RPC ${method}: ${reply.error.message}`), { data: reply.error.data });
    return reply.result;
  }
  try {
    await new Promise((resolve, reject) => { child.once("spawn", resolve); child.once("error", reject); });
    for (let attempt = 0; ; attempt++) {
      if (child.exitCode !== null || child.signalCode !== null) {
        throw new Error(`owned local Anvil exited before becoming ready (${child.signalCode ?? child.exitCode}): ${stderr}`);
      }
      try { assert.equal(await rpc("eth_chainId"), "0x7a69"); break; }
      catch (error) { if (attempt >= 100) throw error; await new Promise((resolve) => setTimeout(resolve, 25)); }
    }
    const chain = { id: 31337, name: "X3 local liquidation fixture", nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
      rpcUrls: { default: { http: [rpcUrl] } } };
    const pub = createPublicClient({ chain, transport: http(rpcUrl), pollingInterval: 10 });
    const wallet = createWalletClient({ chain, transport: http(rpcUrl) });
    const [deployer, supplier, borrower, initialKeeper, owner] = (await wallet.getAddresses()).map(lower);
    let keeper = initialKeeper;
    const read = (address, name, fn, args = []) => pub.readContract({ address, abi: art[name].abi, functionName: fn, args });
    async function send(address, name, fn, args, from = deployer, allowRevert = false) {
      const hash = await wallet.writeContract({ address, abi: art[name].abi, functionName: fn, args,
        account: from, gas: 3_000_000n, gasPrice: 1_000_000_000n });
      const receipt = await pub.waitForTransactionReceipt({ hash });
      if (!allowRevert) assert.equal(receipt.status, "success", `${name}.${fn} must succeed`);
      return receipt;
    }
    async function deploy(name, args = []) {
      const hash = await wallet.deployContract({ abi: art[name].abi, bytecode: art[name].bytecode, args,
        account: deployer, gas: 10_000_000n, gasPrice: 1_000_000_000n });
      const receipt = await pub.waitForTransactionReceipt({ hash });
      assert.equal(receipt.status, "success", `${name} deployment`);
      return lower(receipt.contractAddress);
    }
    const loan = await deploy("MockERC20", ["Fixture USD", "FUSD", 6]);
    const collateral = await deploy("MockERC20", ["Fixture BTC", "FBTC", 8]);
    const oracle = await deploy("MockOracle");
    await send(oracle, "MockOracle", "setPrice", [price(100_000)]);
    const morpho = await deploy("Morpho", [deployer]);
    await send(morpho, "Morpho", "enableIrm", [zeroAddress]);
    await send(morpho, "Morpho", "enableLltv", [LLTV]);
    const params = { loanToken: loan, collateralToken: collateral, oracle, irm: zeroAddress, lltv: LLTV };
    await send(morpho, "Morpho", "createMarket", [params]);
    await send(loan, "MockERC20", "mint", [supplier, 2_000_000n * E6]);
    await send(loan, "MockERC20", "approve", [morpho, (1n << 256n) - 1n], supplier);
    await send(morpho, "Morpho", "supply", [params, 1_000_000n * E6, 0n, supplier, "0x"], supplier);
    await send(collateral, "MockERC20", "mint", [borrower, SAT]);
    await send(collateral, "MockERC20", "approve", [morpho, SAT], borrower);
    await send(morpho, "Morpho", "supplyCollateral", [params, SAT, borrower, "0x"], borrower);
    await send(morpho, "Morpho", "borrow", [params, 80_000n * E6, 0n, borrower, borrower], borrower);
    const router = await deploy("MockSwapRouter");
    await send(loan, "MockERC20", "mint", [router, 10_000_000n * E6]);
    const executor = await deploy("MonkeyLiquidatorSwap", [morpho, owner, keeper, 50n]);
    await send(executor, "MonkeyLiquidatorSwap", "setRouter", [router, true], owner);
    await send(oracle, "MockOracle", "setPrice", [price(92_000)]);
    await send(router, "MockSwapRouter", "setPrice", [collateral, loan, rate(92_000, 50)]);
    const marketId = keccak256(encodeAbiParameters(
      [{ type: "address" }, { type: "address" }, { type: "address" }, { type: "address" }, { type: "uint256" }],
      [loan, collateral, oracle, zeroAddress, LLTV]));
    const seized = ((80_000n * E6 * LIF / WAD) * SCALE / price(92_000)) - 10n;
    const minProfit = 1_000n * E6;
    const swapData = encodeFunctionData({ abi: art.MockSwapRouter.abi, functionName: "swapExactIn",
      args: [collateral, loan, seized, 0n, executor] });
    const order = { borrower, seizedAssets: seized, repaidShares: 0n, minProfit, router, swapData };
    const calldata = encodeFunctionData({ abi: art.MonkeyLiquidatorSwap.abi, functionName: "liquidate", args: [params, order] });
    const initialBorrowShares = (await read(morpho, "Morpho", "position", [marketId, borrower]))[1];
    let snapshotId = await rpc("evm_snapshot"), manifest;
    const compactReceipt = (receipt) => ({ hash: receipt.transactionHash, status: receipt.status,
      block_number: receipt.blockNumber.toString(), block_hash: receipt.blockHash,
      gas_used: receipt.gasUsed.toString(), effective_gas_price_wei: receipt.effectiveGasPrice.toString() });
    async function makeManifest() {
      const quoted = await pub.simulateContract({ address: morpho, abi: art.Morpho.abi, functionName: "liquidate",
        args: [params, borrower, seized, 0n, "0x"], account: supplier });
      const profit = await pub.simulateContract({ address: executor, abi: art.MonkeyLiquidatorSwap.abi, functionName: "liquidate",
        args: [params, order], account: keeper });
      const head = await pub.getBlock();
      return {
        schema: "x3.native-liquidation.local-fixture.v1",
        classification: "owned loopback Anvil only; genuine Morpho and executor; mock tokens, oracle and DEX",
        rpc_url: rpcUrl, chain_id: 31337, keeper, owner, morpho, executor,
        executor_code_hash: keccak256(await pub.getCode({ address: executor })), router,
        market_id: marketId,
        market: { loan_token: loan, collateral_token: collateral, oracle, irm: zeroAddress, lltv: LLTV.toString() },
        borrower, seized_assets: seized.toString(), repaid_shares: "0", min_profit: minProfit.toString(),
        repay_assets: quoted.result[1].toString(), gross_profit: profit.result.toString(), swap_data: swapData, calldata,
        gas_limit: "3000000", loan_decimals: 6, collateral_decimals: 8,
        block_number: head.number.toString(), block_hash: head.hash, timestamp: head.timestamp.toString(),
        source_manifest_sha256: artifacts.source_manifest_sha256,
      };
    }
    async function state() {
      const position = await read(morpho, "Morpho", "position", [marketId, borrower]);
      const market = await read(morpho, "Morpho", "market", [marketId]);
      const [ownerLoan, poolLoan, poolCollateral, routerLoan, routerCollateral, executorLoan, executorCollateral, routerAllowance, morphoAllowance, native, nonce] = await Promise.all([
        read(loan, "MockERC20", "balanceOf", [owner]), read(loan, "MockERC20", "balanceOf", [morpho]),
        read(collateral, "MockERC20", "balanceOf", [morpho]), read(loan, "MockERC20", "balanceOf", [router]),
        read(collateral, "MockERC20", "balanceOf", [router]), read(loan, "MockERC20", "balanceOf", [executor]),
        read(collateral, "MockERC20", "balanceOf", [executor]), read(collateral, "MockERC20", "allowance", [executor, router]),
        read(loan, "MockERC20", "allowance", [executor, morpho]), pub.getBalance({ address: keeper }),
        pub.getTransactionCount({ address: keeper }),
      ]);
      return { owner_loan: ownerLoan.toString(), morpho_loan: poolLoan.toString(), morpho_collateral: poolCollateral.toString(),
        router_loan: routerLoan.toString(), router_collateral: routerCollateral.toString(), executor_loan: executorLoan.toString(),
        executor_collateral: executorCollateral.toString(), router_allowance: routerAllowance.toString(), morpho_allowance: morphoAllowance.toString(),
        borrow_shares: position[1].toString(), posted_collateral: position[2].toString(), keeper_native: native.toString(), keeper_nonce: nonce,
        total_supply_assets: market[0].toString(), total_supply_shares: market[1].toString(),
        total_borrow_assets: market[2].toString(), total_borrow_shares: market[3].toString(), market_last_update: market[4].toString() };
    }
    async function reset() {
      assert.equal(await rpc("evm_revert", [snapshotId]), true, "owned snapshot must restore");
      snapshotId = await rpc("evm_snapshot");
      manifest = await makeManifest();
      return { manifest, state: await state() };
    }
    async function setKeeper(address) {
      if (!isAddress(address)) throw new Error("setKeeper requires an EVM address");
      assert.equal((await read(morpho, "Morpho", "position", [marketId, borrower]))[1], initialBorrowShares,
        "reset before changing keeper after a liquidation");
      const receipt = await send(executor, "MonkeyLiquidatorSwap", "setKeeper", [lower(address)], owner);
      keeper = lower(address);
      await rpc("anvil_setBalance", [keeper, "0x3635c9adc5dea00000"]); // 1000 synthetic ETH, never external funds.
      snapshotId = await rpc("evm_snapshot");
      manifest = await makeManifest();
      return { receipt: compactReceipt(receipt), manifest };
    }
    async function swapFail() {
      const receipt = await send(router, "MockSwapRouter", "setPrice", [collateral, loan, rate(92_000, 500)]);
      return { receipt: compactReceipt(receipt), state: await state() };
    }
    async function executeAndMeasure({ swapCallFails = false } = {}) {
      const before = await state();
      const callOrder = swapCallFails ? { ...order, swapData: encodeFunctionData({ abi: art.MockSwapRouter.abi,
        functionName: "swapExactIn", args: [collateral, loan, seized, 10_000_000n * E6, executor] }) } : order;
      const receipt = await send(executor, "MonkeyLiquidatorSwap", "liquidate", [params, callOrder], keeper, true);
      const after = await state();
      const trace = await rpc("debug_traceTransaction", [receipt.transactionHash, { tracer: "callTracer" }]);
      const traceCalls = [];
      const roles = new Map([[morpho, "Morpho"], [executor, "executor"], [router, "mock DEX"],
        [loan, "mock loan token"], [collateral, "mock collateral token"], [oracle, "mock oracle"]]);
      function visit(call) {
        if (call.to) traceCalls.push({ role: roles.get(lower(call.to)) ?? "other", address: lower(call.to),
          selector: (call.input ?? "0x").slice(0, 10), error: call.error ?? null });
        for (const nested of call.calls ?? []) visit(nested);
      }
      visit(trace);
      let revertError = null;
      if (receipt.status === "reverted" && trace.output) {
        try { revertError = decodeErrorResult({ abi: art.MonkeyLiquidatorSwap.abi, data: trace.output }).errorName; }
        catch { revertError = "unrecognized contract revert"; }
      }
      const events = receipt.logs.flatMap((log) => {
        try { return [decodeEventLog({ abi: art.EventsLib.abi, ...log })]; } catch { return []; }
      }).filter((event) => event.eventName === "Liquidate");
      return { receipt: compactReceipt(receipt), before, after,
        owner_profit_loan: (BigInt(after.owner_loan) - BigInt(before.owner_loan)).toString(),
        repaid_loan: (BigInt(after.morpho_loan) - BigInt(before.morpho_loan)).toString(),
        gas_paid_wei: (receipt.gasUsed * receipt.effectiveGasPrice).toString(),
        keeper_native_spent_wei: (BigInt(before.keeper_native) - BigInt(after.keeper_native)).toString(),
        revert_error: revertError, trace_calls: traceCalls,
        morpho_events: events.map((event) => ({ name: event.eventName, repaid_assets: event.args.repaidAssets.toString(),
          seized_assets: event.args.seizedAssets.toString() })),
      };
    }
    manifest = await makeManifest();
    return { get manifest() { return manifest; }, state, reset, setKeeper, swapFail, executeAndMeasure, close };
  } catch (error) {
    await close();
    throw error;
  }
}

export async function runSelfTest() {
  const fixture = await startFixture();
  try {
    const manifest = fixture.manifest;
    const success = await fixture.executeAndMeasure();
    assert.equal(success.receipt.status, "success");
    assert.equal(success.owner_profit_loan, manifest.gross_profit);
    assert.equal(success.repaid_loan, manifest.repay_assets);
    assert.equal(success.morpho_events[0]?.repaid_assets, manifest.repay_assets);
    assert.ok(BigInt(success.owner_profit_loan) > 3_080_000_000n && BigInt(success.owner_profit_loan) < 3_100_000_000n);
    assert.equal(success.gas_paid_wei, success.keeper_native_spent_wei);
    for (const key of ["executor_loan", "executor_collateral", "router_allowance", "morpho_allowance"]) assert.equal(success.after[key], "0");
    await fixture.reset();
    await fixture.swapFail();
    const settlementFailure = await fixture.executeAndMeasure();
    await fixture.reset();
    const swapFailure = await fixture.executeAndMeasure({ swapCallFails: true });
    for (const [result, error] of [[settlementFailure, "SwapShort"], [swapFailure, "SwapFailed"]]) {
      assert.equal(result.receipt.status, "reverted");
      assert.equal(result.revert_error, error);
      for (const key of Object.keys(result.before)) {
        if (!["keeper_native", "keeper_nonce"].includes(key)) assert.equal(result.after[key], result.before[key], `atomic rollback ${key}`);
      }
      assert.equal(result.owner_profit_loan, "0"); assert.equal(result.repaid_loan, "0");
      assert.ok(BigInt(result.gas_paid_wei) > 0n); assert.equal(result.gas_paid_wei, result.keeper_native_spent_wei);
      assert.equal(result.after.keeper_nonce, result.before.keeper_nonce + 1);
    }
    assert.ok(settlementFailure.trace_calls.some((call) => call.role === "mock DEX" && call.error === null));
    assert.ok(swapFailure.trace_calls.some((call) => call.role === "mock DEX" && call.error !== null));
    return { schema: "x3.native-liquidation.fixture-proof.v1", classification: "local Anvil contract proof; not an X3 runtime or public-chain deployment claim",
      manifest, checks: { success: true, repayment: true, owner_profit: true, gas_charged: true, callback_settlement_rollback: true, callback_swap_rollback: true },
      success, settlement_failure: settlementFailure, swap_failure: swapFailure };
  } finally { await fixture.close(); }
}

async function main() {
  const stop = () => { for (const child of ownedChildren) child.kill("SIGTERM"); process.exit(0); };
  process.once("SIGTERM", stop); process.once("SIGINT", stop);
  if (process.argv.includes("--self-test")) {
    const record = await runSelfTest();
    const outAt = process.argv.indexOf("--out");
    if (outAt >= 0) {
      const out = process.argv[outAt + 1];
      if (!out || out.startsWith("--")) throw new Error("--out requires an evidence JSON path");
      writeFileSync(out, json(record) + "\n");
      console.log(json({ ok: true, classification: record.classification, checks: record.checks }));
    } else console.log(json(record));
    return;
  }
  if (!process.argv.includes("--serve")) throw new Error("usage: node fixture.mjs --serve | --self-test [--out proof.local.json]");
  const fixture = await startFixture();
  try {
    console.log(json(fixture.manifest));
    for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
      try {
        const request = JSON.parse(line);
        if (request.op === "stop") { console.log(json({ ok: true })); break; }
        let result;
        if (request.op === "state") result = await fixture.state();
        else if (request.op === "reset") result = await fixture.reset();
        else if (request.op === "setKeeper") result = await fixture.setKeeper(request.address);
        else if (request.op === "swapFail") result = await fixture.swapFail();
        else throw new Error("unknown fixture operation");
        console.log(json({ ok: true, ...result }));
      } catch (error) { console.log(json({ ok: false, error: error.message })); }
    }
  } finally { await fixture.close(); }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}

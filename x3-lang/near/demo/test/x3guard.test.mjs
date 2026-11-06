// node --test x3-lang/near/demo/test
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import * as x3 from "../x3guard.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fixtures = join(here, "..", "..", "..", "tests", "fixtures");
const capture = (name) => JSON.parse(readFileSync(join(fixtures, name), "utf8"));
const CAPTURES = {
  "near_intents_quote_dry_live_capture.json": "EgJWANgZ5TFdSxtxDHUYaGj5JzyG9CN6b6SY8npneAN5",
  "near_intents_quote_dry_implicit_capture.json": "HAgUJ3X5LXgjvg2bQEAs1JF94ynSG146uPL7cEKLPnvA",
};
const TOKENS = JSON.parse(readFileSync(join(fixtures, "near_intents_tokens.json"), "utf8"));
const EXAMPLE = readFileSync(join(here, "..", "..", "..", "examples", "near_intents_sol_to_near.x3"), "utf8");
const NOW = Date.parse("2026-11-03T03:00:00.000Z");

for (const [name, hash] of Object.entries(CAPTURES)) {
  test(`real 1Click quote ${name} verifies, with the SDK's hash`, async () => {
    const r = capture(name);
    assert.equal(await x3.quoteHash(r), hash);
    assert.deepEqual(await x3.checkQuoteSignature(r), { valid: true, reason: "verified", hash });
  });

  test(`tampering with ${name} breaks the signature`, async () => {
    const tampers = [
      (r) => { r.quote.minAmountOut = String(BigInt(r.quote.minAmountOut) + 1n); },
      (r) => { r.quoteRequest.recipient = "attacker.near"; },
      (r) => { r.timestamp = "2026-11-03T05:00:00.000Z"; },
      (r) => { r.signature = "FAKE_SIGNATURE"; },
      (r) => { r.signature = r.signature.replace(/^ed25519:/, ""); },
    ];
    for (const t of tampers) {
      const r = capture(name);
      t(r);
      assert.equal((await x3.checkQuoteSignature(r)).valid, false);
    }
  });
}

test("unsigned echo fields do not change the signature", async () => {
  const r = capture("near_intents_quote_dry_live_capture.json");
  r.quoteRequest.appFees = [{ recipient: "x.near", fee: 9999 }];
  r.correlationId = "anything";
  assert.equal((await x3.checkQuoteSignature(r)).valid, true);
});

test("the example lowers to the request the Python adapter builds", () => {
  const leg = x3.lower(x3.parse(EXAMPLE), NOW);
  assert.equal(leg.slippage_bps, 50);
  assert.equal(leg.fee_cap_bps, 100);
  assert.equal(leg.deadline, "2026-11-03T03:10:00.000Z");
  const req = x3.buildQuoteRequest(leg, TOKENS);
  assert.equal(req.originAsset, "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near");
  assert.equal(req.destinationAsset, "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1");
  assert.equal(req.amount, "25000000");
});

test("slippage bounds round toward the stricter side, exactly", () => {
  const bps = (bound) => x3.lower(x3.parse(EXAMPLE.replace("require slippage <= 0.5%", bound)), NOW).slippage_bps;
  assert.equal(bps("require slippage <= 0.29%"), 29); // a float would give 28
  assert.equal(bps("require slippage < 0.5%"), 49);
  assert.equal(bps("require slippage <= 0.125%"), 12);
  assert.equal(bps("require slippage <= 0.003"), 30);
});

test("BNB Chain aliases resolve and refunds require a complete EVM address", () => {
  assert.equal(x3.normalizeChain("BNB"), "bsc");
  assert.equal(x3.normalizeChain("binance"), "bsc");
  assert.equal(x3.isValidReceiver("bsc", "0x" + "a".repeat(40)), true);
  assert.equal(x3.isValidReceiver("bsc", "0xA1"), false);
  assert.equal(x3.isValidReceiver("ethereum", "0xA1"), true);
});

test("BSC BNB lowers with the live provider asset IDs", () => {
  const source = `intent bsc_demo {
    from Bsc.BNB amount 2000000000000000000 receiver 0x${"b".repeat(40)}
    to Near.wNEAR receiver wrap.near
    route { bridge NearIntents Bsc.BNB -> Near.wNEAR }
    require slippage <= 1%
    require fees <= 100 bps
    require nonce unused bsc_demo_001
    timeout 10m refund Bsc.BNB
    on_fail rollback
  }`;
  const request = x3.buildQuoteRequest(x3.lower(x3.parse(source), NOW), TOKENS, { dry: true });
  assert.equal(request.originAsset, "nep245:v2_1.omni.hot.tg:56_11111111111111111111");
  assert.equal(request.destinationAsset, "nep141:wrap.near");
  assert.equal(request.amount, "2000000000000000000");
});

test("refusals carry the adapter's codes and the offending line", () => {
  const refuse = (from, to) => {
    try { x3.lower(x3.parse(EXAMPLE.replace(from, to)), NOW); } catch (e) { return [e.code, e.line]; }
    return null;
  };
  assert.deepEqual(refuse("require slippage <= 0.5%", "require finality Solana >= 32"), ["X3_NEAR_GUARD_UNENFORCEABLE", 30]);
  assert.deepEqual(refuse("on_fail rollback", "on_fail halt"), ["X3_NEAR_ON_FAIL_UNSUPPORTED", 34]);
  assert.deepEqual(refuse("timeout 10m refund Solana.USDC", "timeout 10m refund Solana.SOL"), ["X3_NEAR_REFUND_ASSET", 33]);
  assert.equal(refuse("require slippage <= 0.5%", "")[0], "X3_NEAR_SLIPPAGE_REQUIRED");
});

test("guards on a real quote match the adapter's numbers", async () => {
  const r = capture("near_intents_quote_dry_live_capture.json");
  const keys = ["dry", "swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset", "amount", "refundTo", "refundType", "recipient", "recipientType", "deadline"];
  const request = Object.fromEntries(keys.map((k) => [k, r.quoteRequest[k]]));
  const leg = { amount: "25000000", min_receive: "24800000", slippage_bps: 50, fee_cap_bps: 100, deadline: request.deadline };
  const { results, verdict } = await x3.verifyQuote(leg, request, r, { nowMs: x3.parseTimestampMs(r.timestamp) });
  assert.equal(verdict, "dry_pass");
  const by = Object.fromEntries(results.map((g) => [g.guard, g]));
  assert.equal(by.slippage.quoted_spread_bps, "50.00");
  assert.equal(by.fees.effective_cost_bps, "0.83");
  assert.equal(by.refund_fee.refund_fee_bps, "97.77");
  assert.equal(by.refund_fee.signed, false);
  const tighter = await x3.verifyQuote({ ...leg, min_receive: "24900000" }, request, r, { nowMs: x3.parseTimestampMs(r.timestamp) });
  assert.deepEqual(tighter.failed, ["min_receive"]);
});

test("a forged quote stops at the signature and nothing else is read", async () => {
  const r = capture("near_intents_quote_dry_live_capture.json");
  r.quote.amountOut = "99999999";
  const { results, verdict } = await x3.verifyQuote({ amount: "25000000", slippage_bps: 50, deadline: r.quoteRequest.deadline }, r.quoteRequest, r, { nowMs: Date.now() });
  assert.equal(verdict, "invalid_signature");
  assert.deepEqual(results.map((g) => g.guard), ["quote_signature"]);
});

test("an agent's intent is checked against its owner's mandate, term by term", async () => {
  const tokens = JSON.parse(readFileSync(new URL("../../../tests/fixtures/near_intents_tokens.json", import.meta.url), "utf8"));
  const mandateSource = readFileSync(new URL("../../../examples/near_mandate_agent.x3", import.meta.url), "utf8");
  const intentSource = readFileSync(new URL("../../../examples/near_intents_agent_payment.x3", import.meta.url), "utf8");
  const now = Date.UTC(2026, 10, 3, 3, 0, 0);
  const compiled = x3.lowerMandate(x3.parseMandate(mandateSource), tokens, now, await x3.sha256Hex(mandateSource));
  const mandate = { ...compiled, version: 1, revoked: false, spends: [] };
  const leg = x3.lower(x3.parse(intentSource), now);
  assert.equal(leg.mandate_owner, "x3-owner.near");
  const policy = x3.policyFor(leg, x3.buildQuoteRequest(leg, tokens, { dry: true }), "0".repeat(64));
  const failed = (m, p, t = now) => x3.checkMandate(m, p, t).filter((c) => !c.ok).map((c) => c.guard);
  assert.deepEqual(failed(mandate, policy), []);
  assert.deepEqual(failed(mandate, { ...policy, recipient: "attacker.near" }), ["mandate_recipient"]);
  assert.deepEqual(failed({ ...mandate, revoked: true }, policy), ["mandate_active"]);
  const spends = [{ at_ms: now - 1000, amount: "190000000", nonce: "x" }];
  assert.deepEqual(failed({ ...mandate, spends }, policy), ["mandate_daily"]);
  assert.deepEqual(failed({ ...mandate, spends }, policy, now + 86_400_000 - 999), []);
  assert.equal(x3.approvalsNeeded(mandate, "30000001"), 1);
  assert.throws(() => x3.parseMandate(mandateSource.replace("expires 30d", "expires 30d\n    drain everything")), /not a mandate clause/);
});

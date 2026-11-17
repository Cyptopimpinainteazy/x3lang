import { test } from "node:test";
import assert from "node:assert/strict";
import { units, matches } from "./format.mjs";
test("base units stay exact beyond JS safe integers", () => {
  assert.equal(units("1000000000000000001", 18, 18), "1.000000000000000001");
  assert.equal(units("-2500000", 6), "−2.5");
  assert.equal(units(null, 6), "Unknown");
});
test("unknown spreads never pass a positive profit filter", () => {
  const row = {
    label: "SOL to NEAR",
    legs: [],
    kind: "cross-chain",
    net_profit_bps: null,
    paper_eligible: false,
  };
  assert.equal(matches(row, { minimum: "0" }), false);
  assert.equal(matches(row, { search: "near" }), true);
  assert.equal(matches(row, { eligible: true }), false);
});

import { selectTokenPair, pairIssue } from "./format.mjs";
test("token clicks select exact chain assets instead of matching ticker symbols", () => {
  const sol = {
    asset_id: "sol-usdc",
    network: "sol",
    symbol: "USDC",
    deprecated: false,
  };
  const bsc = {
    asset_id: "bsc-usdc",
    network: "bsc",
    symbol: "USDC",
    deprecated: false,
  };
  let pair = selectTokenPair({}, sol, "origin_asset");
  pair = selectTokenPair(pair, bsc, "destination_asset");
  assert.deepEqual(pair, {
    origin_asset: "sol-usdc",
    destination_asset: "bsc-usdc",
  });
  assert.equal(pairIssue(pair, [sol, bsc]), null);
  assert.throws(
    () =>
      selectTokenPair(pair, { ...bsc, deprecated: true }, "destination_asset"),
    /deprecated/,
  );
  assert.equal(
    pairIssue({ ...pair, destination_asset: "sol-usdc" }, [sol, bsc]),
    "SAME_CHAIN",
  );
});

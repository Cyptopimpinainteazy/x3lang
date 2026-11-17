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

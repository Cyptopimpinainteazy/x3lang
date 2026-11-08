import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as x3 from "../x3guard.mjs";

const SOURCE = readFileSync(new URL("../../../examples/near_mandate_agent.x3", import.meta.url), "utf8");
const TOKENS = JSON.parse(readFileSync(new URL("../../../tests/fixtures/near_intents_tokens.json", import.meta.url), "utf8"));
const NOW = Date.parse("2026-11-03T03:00:00.000Z");
const U128_MAX = (1n << 128n) - 1n;
const UTC_MAX_MS = 253402300799999;
const lower = (source = SOURCE, now = NOW) => x3.lowerMandate(x3.parseMandate(source), TOKENS, now, "0".repeat(64));
const errorIs = (code, field, line = null) => (err) => err instanceof x3.X3Error
  && err.code === code && err.field === field && (line === null || err.line === line);

test("mandate caps retain every digit at u128 max, and approval can be one unit below", () => {
  const source = SOURCE.replace("per_payment 50000000", `per_payment ${U128_MAX}`)
    .replace("per_24h 200000000", `per_24h ${U128_MAX}`)
    .replace("approve above 30000000", `approve above ${U128_MAX - 1n}`);
  const terms = lower(source).terms;
  assert.equal(terms.per_payment_cap, U128_MAX.toString());
  assert.equal(terms.daily_cap, U128_MAX.toString());
  assert.equal(terms.approval_above, (U128_MAX - 1n).toString());
});

test("mandate caps preserve leading zeros and underscores at u128 max", () => {
  const source = SOURCE.replace("per_payment 50000000", `per_payment 000_${U128_MAX}`)
    .replace("per_24h 200000000", `per_24h ${"0".repeat(5000)}${U128_MAX}`);
  assert.equal(lower(source).terms.per_payment_cap, U128_MAX.toString());
  assert.equal(lower(source).terms.daily_cap, U128_MAX.toString());
});

for (const [name, field, source] of [
  ["u128 payment overflow", "per_payment", SOURCE.replace("per_payment 50000000", `per_payment ${U128_MAX + 1n}`)
    .replace("per_24h 200000000", `per_24h ${U128_MAX + 1n}`)],
  ["u128 daily overflow", "per_24h", SOURCE.replace("per_24h 200000000", `per_24h ${U128_MAX + 1n}`)],
  ["payment digit limit", "per_payment", SOURCE.replace("per_payment 50000000", `per_payment ${"9".repeat(5000)}`)],
  ["daily digit limit", "per_24h", SOURCE.replace("per_24h 200000000", `per_24h ${"9".repeat(5000)}`)],
]) {
  test(`mandate ${name} is refused with its source line`, () => {
    assert.throws(() => lower(source), errorIs("X3_MANDATE_LIMIT", field, x3.parseMandate(source).line_of[field]));
  });
}

for (const [name, amount] of [["u128 overflow", U128_MAX + 1n], ["integer digit limit", "9".repeat(5000)]]) {
  test(`approval ${name} is refused with its source line`, () => {
    const source = SOURCE.replace("approve above 30000000", `approve above ${amount}`);
    assert.throws(() => lower(source), errorIs("X3_MANDATE_APPROVAL", "approve", x3.parseMandate(source).line_of.approve));
  });
}

test("oversized approval threshold is refused with its source line", () => {
  const source = SOURCE.replace("approve above 30000000", `approve above ${"9".repeat(5000)}`);
  assert.throws(() => lower(source), errorIs("X3_MANDATE_APPROVAL", "approve", x3.parseMandate(source).line_of.approve));
});

test("approval preserves leading zeros and underscores below the payment cap", () => {
  const source = SOURCE.replace("approve above 30000000", `approve above ${"0".repeat(5000)}30_000_000`);
  assert.equal(lower(source).terms.approval_above, "30000000");
});

for (const [kind, original, term] of [
  ["slippage", "<= 1%", "max_slippage_bps"], ["fees", "<= 100 bps", "max_fee_bps"],
]) {
  for (const bound of ["<= 10001bps", "<= 100.01%", "< 10002bps"]) {
    test(`mandate ${kind} refuses ${bound}`, () => {
      const source = SOURCE.replace(`require ${kind} ${original}`, `require ${kind} ${bound}`);
      assert.throws(() => lower(source), errorIs("X3_MANDATE_BOUND", `require ${kind}`));
    });
  }
  for (const [bound, expected] of [["<= 0bps", 0], ["<= 10000bps", 10000], ["< 10001bps", 10000],
    ["<= 10000.9bps", 10000], ["< 10000bps", 9999], ["<= 100%", 10000]]) {
    test(`mandate ${kind} preserves ${bound} as ${expected} bps`, () => {
      const source = SOURCE.replace(`require ${kind} ${original}`, `require ${kind} ${bound}`);
      assert.equal(lower(source).terms[term], expected);
    });
  }
  test(`mandate ${kind} uses the strictest clause before range validation`, () => {
    const source = SOURCE.replace("expires 30d", `require ${kind} <= 10001bps\n    expires 30d`);
    assert.equal(lower(source).terms[term], 100);
  });
}

for (const [name, duration] of [
  ["timedelta overflow", "1000000000d"], ["year 10000", "3000000d"],
  ["last UTC millisecond plus one", `${UTC_MAX_MS - NOW + 1}ms`],
  ["u64 overflow", `${1n << 64n}ms`], ["infinite Number", "9".repeat(400) + "d"],
  ["integer digit limit", "9".repeat(5000) + "ms"],
]) {
  test(`mandate expiry refuses ${name} with a structured error`, () => {
    const source = SOURCE.replace("expires 30d", `expires ${duration}`);
    assert.throws(() => lower(source), errorIs("X3_MANDATE_EXPIRES", "expires", x3.parseMandate(source).line_of.expires));
  });
}

test("the last four-digit UTC millisecond compiles exactly", () => {
  const compiled = lower(SOURCE.replace("expires 30d", `expires ${UTC_MAX_MS - NOW}ms`));
  assert.equal(compiled.terms.expires_at_ms, UTC_MAX_MS);
  assert.equal(compiled.expires, "9999-12-31T23:59:59.999Z");
});

test("mandate expiry parser rejects unrepresentable durations directly", () => {
  for (const text of ["1000000000d", `${1n << 64n}ms`, "9".repeat(5000) + "ms"]) {
    assert.throws(() => x3.parseExpiryMs(text, 12), errorIs("X3_MANDATE_EXPIRES", "expires", 12));
  }
});

test("mandate expiry preserves all duration units and ordinary timestamps", () => {
  for (const [text, ms] of [["1ms", 1], ["2s", 2000], ["3m", 180000], ["4h", 14400000], ["30d", 2592000000]]) {
    assert.equal(x3.parseExpiryMs(text), ms);
    assert.equal(lower(SOURCE.replace("expires 30d", `expires ${text}`)).terms.expires_at_ms, NOW + ms);
  }
});

test("expiry addition refuses invalid clocks and timestamps outside the common range", () => {
  const source = SOURCE.replace("expires 30d", "expires 1ms");
  for (const now of [UTC_MAX_MS, -2, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, NOW + 0.5]) {
    assert.throws(() => lower(source, now), errorIs("X3_MANDATE_EXPIRES", "expires"));
  }
});

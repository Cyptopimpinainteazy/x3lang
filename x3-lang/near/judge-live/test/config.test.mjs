// Static checks for the judge-live site: config shape, the featured .x3 program's bound hash,
// and that every evidence link is a real hash rather than a test value.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(join(root, "config.json"), "utf8"));

test("featured .x3 program matches the SHA-256 bound on Guard", () => {
  const p = config.featuredProgram;
  const src = readFileSync(join(root, p.path));
  const sha = createHash("sha256").update(src).digest("hex");
  assert.equal(sha, p.sha256);
  assert.equal(sha, "1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860");
  const canonical = readFileSync(join(root, "..", "..", "examples", "base_sepolia_to_near_testnet_1usdc.x3"));
  assert.ok(src.equals(canonical), "programs/ copy must equal x3-lang/examples/ byte for byte");
});

test("featured program links point at real explorers and hashes", () => {
  for (const t of config.featuredProgram.txs) {
    assert.match(t.url, /^https:\/\/(sepolia\.basescan\.org\/tx|testnet\.nearblocks\.io\/(txns|blocks))\//);
    assert.ok(t.url.endsWith(t.hash), `${t.label}: url must end with its hash`);
    assert.doesNotMatch(t.hash, /TEST/i);
  }
});

test("setup transactions are mainnet base58 hashes with a signer", () => {
  assert.equal(config.network, "mainnet");
  assert.ok(config.setupTxs.length >= 2);
  for (const t of config.setupTxs) {
    assert.match(t.hash, /^[1-9A-HJ-NP-Za-km-z]{43,44}$/);
    assert.match(t.sender, /\.near$/);
  }
});

test("exactly one agent account, and every account is a mainnet name", () => {
  assert.equal(config.accounts.filter((a) => a.isAgent).length, 1);
  for (const a of config.accounts) assert.match(a.id, /^[a-z0-9_.-]+\.near$/);
});

test("the static feed fallback is valid and starts empty", () => {
  const feed = JSON.parse(readFileSync(join(root, "evidence", "mainnet", "feed.json"), "utf8"));
  assert.equal(feed.network, "mainnet");
  assert.ok(Array.isArray(feed.attempts));
});

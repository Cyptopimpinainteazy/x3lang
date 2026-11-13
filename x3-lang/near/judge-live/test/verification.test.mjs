// Exercise the production page's result boundary with offline RPC fixtures only.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../app.js", import.meta.url), "utf8");
assert.match(source, /\ninit\(\);\s*$/);

function page() {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, { innerHTML: "", textContent: "", hidden: false, disabled: false, value: "offline proposal" });
    return elements.get(id);
  };
  const context = vm.createContext({ document: { getElementById: element } });
  // Leave startup offline; the tested functions are the unchanged production functions.
  vm.runInContext(source.replace(/\ninit\(\);\s*$/, "\n"), context);
  vm.runInContext(`S.cfg = { explorer: "https://example.invalid", turnstileSiteKey: "",
    accounts: [{ id: "expected-agent.near", isAgent: true }] };`, context);
  return { context, element };
}

function fixture(outcome = "authorized") {
  return {
    attempt: { status: "final", chain: { txHash: "fixture-hash", sender: "expected-agent.near", receiver: "expected-vault.near", outcome } },
    result: { final_execution_status: "FINAL", status: { SuccessValue: "" },
      transaction: { hash: "fixture-hash", signer_id: "expected-agent.near", receiver_id: "expected-vault.near", actions: [] },
      transaction_outcome: { block_hash: "fixture-block" }, receipts_outcome: [] }
  };
}

async function inspect(data, status = null) {
  const p = page();
  Object.assign(p.context, data, { runnerStatus: status });
  vm.runInContext('S.status = runnerStatus; rpc = async () => ({ url: "offline fixture", result });', p.context);
  await vm.runInContext("verifyAttemptOnChain(attempt)", p.context);
  return { chip: p.element("chain-chip").innerHTML, details: p.element("chain-kv").innerHTML };
}

test("matching final success remains a basic RPC lookup, not payment proof", async () => {
  const result = await inspect(fixture());
  assert.match(result.chip, /RPC fields match/);
  assert.match(result.chip, /payment unverified/);
  assert.doesNotMatch(result.chip, /chip ok|Chain agrees/);
  assert.match(result.details, /program.*not verified/i);
});

test("matching final failure does not prove the reported Guard rejection reason", async () => {
  const data = fixture("rejected");
  data.result.status = { Failure: { ActionError: {} } };
  const result = await inspect(data);
  assert.match(result.chip, /RPC fields match/);
  assert.match(result.chip, /payment unverified/);
  assert.doesNotMatch(result.chip, /chip ok|Chain agrees/);
});

for (const [name, status] of [
  ["missing", undefined], ["empty", {}], ["pending", { SuccessReceiptId: "pending" }],
  ["ambiguous", { SuccessValue: "", Failure: {} }], ["malformed success", { SuccessValue: null }],
  ["malformed failure", { Failure: null }], ["empty failure", { Failure: {} }]
]) {
  test(`${name} RPC status cannot be displayed as success or agreement`, async () => {
    const data = fixture();
    data.result.status = status;
    const result = await inspect(data);
    assert.match(result.chip, /Unverified|Mismatch/);
    assert.doesNotMatch(result.chip, /RPC fields match|Chain agrees|chip ok/);
    assert.doesNotMatch(result.details, />Succeeded</);
  });
}

for (const outcome of ["authorized", "rejected"]) {
  test(`a contradictory ${outcome} claim is refused`, async () => {
    const data = fixture(outcome);
    data.result.status = outcome === "authorized" ? { Failure: { ActionError: {} } } : { SuccessValue: "" };
    assert.match((await inspect(data)).chip, /Mismatch/);
  });
}

for (const [name, mutate] of [
  ["non-final execution", d => { d.result.final_execution_status = "EXECUTED"; }],
  ["missing finality", d => { delete d.result.final_execution_status; }],
  ["different transaction hash", d => { d.result.transaction.hash = "unrelated-hash"; }],
  ["different signer", d => { d.result.transaction.signer_id = "unrelated.near"; }],
  ["different receiver", d => { d.result.transaction.receiver_id = "unrelated.near"; }],
  ["missing reported receiver", d => { delete d.attempt.chain.receiver; }],
  ["unknown reported outcome", d => { d.attempt.chain.outcome = "unexpected"; }]
]) {
  test(`${name} stays unverified`, async () => {
    const data = fixture();
    mutate(data);
    const result = await inspect(data);
    assert.match(result.chip, /Unverified|Mismatch/);
    assert.doesNotMatch(result.chip, /RPC fields match|Chain agrees|chip ok/);
  });
}

test("runner status cannot replace the configured agent identity", async () => {
  const data = fixture();
  data.result.transaction.signer_id = "substitute.near";
  data.attempt.chain.sender = "substitute.near";
  const result = await inspect(data, { mandate: { agent: "substitute.near" } });
  assert.match(result.chip, /Mismatch/);
});

test("runner-reported verdicts do not assert payment or that no funds moved", () => {
  for (const outcome of ["authorized", "rejected", "not_submitted", "unexpected"]) {
    const p = page();
    p.context.attempt = fixture(outcome).attempt;
    vm.runInContext("renderVerdict(attempt)", p.context);
    const verdict = p.element("verdict");
    assert.doesNotMatch(verdict.innerHTML, /vault paid|No funds moved|Guard approved it on-chain|Guard refused it on-chain|so nothing was sent/);
    assert.doesNotMatch(verdict.className, /\bok\b/);
    assert.match(verdict.innerHTML, /Runner reports|Unverified/);
  }
});

test("an offline runner keeps the action button disabled", () => {
  const p = page();
  vm.runInContext("S.online = false; updateRunButton();", p.context);
  assert.equal(p.element("run-btn").disabled, true);
  assert.match(p.element("run-help").textContent, /offline/);
});

test("runner-supplied mandate and network cannot paint an on-chain verification badge", () => {
  const p = page();
  p.context.runnerStatus = { network: "mainnet", mandate: { agent: "expected-agent.near", guard: "guard.near", recipients: [] } };
  vm.runInContext("S.status = runnerStatus; renderStatus(null);", p.context);
  assert.match(p.element("mandate-chip").textContent, /Runner-reported; chain binding unverified/);
  assert.doesNotMatch(p.element("mandate-chip").className, /\bok\b/);
  assert.match(p.element("try-chip").textContent, /Runner online.*reports mainnet/);
  assert.doesNotMatch(p.element("try-chip").className, /\bok\b/);
});

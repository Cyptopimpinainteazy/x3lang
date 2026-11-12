import assert from "node:assert/strict";
import { test } from "node:test";
import { verifyMainnetSetup } from "../judge/mainnet-setup.mjs";

const owner = "x3lang.near";
const agent = "x3-agent.x3lang.near";

function fixture(change = () => {}) {
  return async (_url, request) => {
    const { id, method, params } = JSON.parse(request.body);
    const terms = {
      program_sha256: "414976c98f6a94d3e0216e6a402435e31a3e23a4876f8383c847b61cd5ffddf8",
      asset: "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near",
      destination_assets: ["nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"],
      recipients: [owner], per_payment_cap: "25000000", daily_cap: "25000000",
      approval_above: "24999999", approval_threshold: 1, approvers: [owner],
      max_slippage_bps: 100, max_fee_bps: 100, expires_at_ms: 1794412477909,
      refund_to: "58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ",
    };
    const accountCreation = params.tx_hash?.startsWith("Cvx");
    const record = {
      final_execution_status: "FINAL",
      transaction: {
        hash: params.tx_hash, signer_id: owner, receiver_id: accountCreation ? agent : owner,
        actions: accountCreation
          ? ["CreateAccount", { Transfer: { deposit: "100000000000000000000000" } }, { AddKey: {} }]
          : [{ FunctionCall: { method_name: "create_mandate" } }],
      },
      status: { SuccessValue: "" },
    };
    const result = method === "query"
      ? { code_hash: "GRy6z84ucCR9WMVXyepibVDC34TyKwjWtEi9rteKJtny", block_height: 42 }
      : record;
    const data = { id, result };
    change({ method, accountCreation, data, terms, record });
    if (method === "tx" && !accountCreation && record.status.SuccessValue === "") {
      record.status.SuccessValue = Buffer.from(JSON.stringify({ owner, agent, terms, version: 1, revoked: false })).toString("base64");
    }
    return { ok: true, json: async () => data };
  };
}

test("verifies finalized historical setup and current code hash", async () => {
  assert.deepEqual(await verifyMainnetSetup(fixture()), {
    block: 42, expiresAt: new Date(1794412477909).toISOString(),
  });
});

test("refuses a pending transaction", async () => {
  await assert.rejects(verifyMainnetSetup(fixture(({ accountCreation, record }) => {
    if (accountCreation) record.final_execution_status = "EXECUTED";
  })), /Finalized setup/);
});

test("refuses changed mandate terms", async () => {
  await assert.rejects(verifyMainnetSetup(fixture(({ terms }) => {
    terms.refund_to = "other-address";
  })), /terms did not match/);
});

test("refuses different current code and treats RPC errors as inconclusive", async () => {
  await assert.rejects(verifyMainnetSetup(fixture(({ method, data }) => {
    if (method === "query") data.result.code_hash = "changed";
  })), /code differs/);
  await assert.rejects(verifyMainnetSetup(fixture(({ method, data }) => {
    if (method === "query") data.error = { message: "unavailable" };
  })), /did not return evidence/);
});

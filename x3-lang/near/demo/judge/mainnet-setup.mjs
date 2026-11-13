const RPC = "https://rpc.mainnet.near.org";
const OWNER = "x3lang.near";
const AGENT = "x3-agent.x3lang.near";
const CREATE_AGENT = "Cvx9yj7bfL4TV1VfVsgyUbcYpQjQQCDrnTWxpPXiAi3d";
const CREATE_MANDATE = "7yiTpwktY5ZD1CMk9aLtEHE7VUeJtd1e71r8NSqrxAer";
const OBSERVED_CODE = "GRy6z84ucCR9WMVXyepibVDC34TyKwjWtEi9rteKJtny";
const PROGRAM = "414976c98f6a94d3e0216e6a402435e31a3e23a4876f8383c847b61cd5ffddf8";
const SOLANA_USDC = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near";
const NEAR_USDC = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";

function requireEvidence(condition, message) {
  if (!condition) throw new Error(message);
}

function verifyTransaction(result, hash, receiver) {
  const tx = result?.transaction;
  requireEvidence(result?.final_execution_status === "FINAL" &&
    tx?.hash === hash && tx.signer_id === OWNER && tx.receiver_id === receiver &&
    typeof result.status?.SuccessValue === "string",
  "Finalized setup transaction did not match.");
  return tx.actions;
}

function decodeJson(encoded) {
  return JSON.parse(new TextDecoder("utf-8", { fatal: true })
    .decode(Uint8Array.from(atob(encoded), (char) => char.charCodeAt(0))));
}

export async function verifyMainnetSetup(fetcher = globalThis.fetch) {
  async function rpc(method, params) {
    const response = await fetcher(RPC, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: method, method, params }),
      signal: AbortSignal.timeout(15000),
    });
    if (!response.ok) throw new Error("NEAR RPC returned an HTTP error.");
    const data = await response.json();
    requireEvidence(data.id === method && !data.error && data.result, "NEAR RPC did not return evidence.");
    return data.result;
  }
  const transaction = (tx_hash) => rpc("tx", {
    tx_hash, sender_account_id: OWNER, wait_until: "FINAL",
  });
  const [agent, mandateTx, account] = await Promise.all([
    transaction(CREATE_AGENT), transaction(CREATE_MANDATE),
    rpc("query", { request_type: "view_account", finality: "final", account_id: OWNER }),
  ]);
  const agentActions = verifyTransaction(agent, CREATE_AGENT, AGENT);
  requireEvidence(agentActions?.length === 3 && agentActions[0] === "CreateAccount" &&
    agentActions[1]?.Transfer?.deposit === "100000000000000000000000" && agentActions[2]?.AddKey,
  "Agent creation actions did not match.");

  const actions = verifyTransaction(mandateTx, CREATE_MANDATE, OWNER);
  requireEvidence(actions?.length === 1 && actions[0]?.FunctionCall?.method_name === "create_mandate",
    "Expected one create_mandate action.");
  const { owner, agent: installedAgent, terms, version, revoked } = decodeJson(mandateTx.status.SuccessValue);
  requireEvidence(owner === OWNER && installedAgent === AGENT && version === 1 && revoked === false &&
    terms?.program_sha256 === PROGRAM && terms.asset === SOLANA_USDC &&
    JSON.stringify(terms.destination_assets) === JSON.stringify([NEAR_USDC]) &&
    JSON.stringify(terms.recipients) === JSON.stringify([OWNER]) &&
    terms.per_payment_cap === "25000000" && terms.daily_cap === "25000000" &&
    terms.approval_above === "24999999" && terms.approval_threshold === 1 &&
    JSON.stringify(terms.approvers) === JSON.stringify([OWNER]) &&
    terms.max_slippage_bps === 100 && terms.max_fee_bps === 100 &&
    terms.refund_to === "58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ" &&
    terms.expires_at_ms === 1794412477909, "Installed mandate terms did not match.");
  requireEvidence(account.code_hash === OBSERVED_CODE && Number.isSafeInteger(account.block_height),
    "Current Guard code differs from the recorded deployment identity.");
  return { block: account.block_height, expiresAt: new Date(terms.expires_at_ms).toISOString() };
}

const button = globalThis.document?.getElementById("check-mainnet-setup");
if (button) {
  const output = document.getElementById("mainnet-setup-result");
  button.addEventListener("click", async () => {
    button.disabled = true;
    output.textContent = "Checking finalized transactions and current Guard code…";
    try {
      const { block, expiresAt } = await verifyMainnetSetup();
      output.textContent = `RPC confirmed both setup transactions and the expected Guard code at block ${block}. The installed mandate had an expiry of ${expiresAt}. This check covers historical setup and current code identity.`;
    } catch {
      output.textContent = "Inconclusive. The RPC was unavailable or the evidence did not match.";
    } finally {
      button.disabled = false;
    }
  });
}

// The judge console's evaluation: `track_control.evaluate`, in the browser.
//
// Same order and the same decisions as x3-lang/track_control.py, on the same x3guard.mjs
// port of the contract's `check_mandate` that the X3 Guard page uses. `scenarios.json`
// carries Python's answer for each scenario; `agrees` compares the two guard by guard.
import * as x3 from "../x3guard.mjs";

/** What each decision is allowed to claim. Nothing here authorizes, signs or settles. */
export const EVIDENCE_LADDER = [
  { level: "local_policy", label: "Locally valid policy", proven: true,
    how: "The mandate and the payment compile, and the shared checker decides them. Python and this page agree." },
  { level: "approved_request", label: "Approved request", proven: false,
    how: "x3-guard records approvals on NEAR (approve_intent). No approval has been recorded for these scenarios." },
  { level: "onchain_authorization", label: "On-chain authorization", proven: false,
    how: "x3-guard's agent_register_intent and agent_accept_quote on a deployed contract. Not deployed for this console." },
  { level: "wallet_transfer", label: "Wallet-controlled transfer", proven: false,
    how: "x3-guard does not custody funds. A keyholder can still transfer directly; only a wallet that calls the Guard is bound by it." },
  { level: "signed_quote", label: "Signed quote", proven: false,
    how: "The X3 Guard page verifies real 1Click dry quotes in the browser. This console asks 1Click for nothing." },
  { level: "observed_transaction", label: "Observed transaction", proven: false,
    how: "No deposit or delivery transaction exists for these scenarios." },
  { level: "finalized_settlement", label: "Finalized settlement", proven: false,
    how: "No settlement has been observed." },
  { level: "proven_causality", label: "Proven cross-chain causality", proven: false,
    how: "Not claimed. Separate source and destination transactions would not prove that one caused the other." },
];

export const DECISIONS = {
  preflight_passed: { tone: "pass", label: "Passes the local policy check",
    detail: "Every term of the mandate holds. On NEAR the agent could register it; the deposit still needs a wallet signature." },
  approval_required: { tone: "note", label: "Needs approval",
    detail: "Every term holds, but the amount is over the approval threshold. x3-guard waits for the listed approvers." },
  denied_by_policy: { tone: "fail", label: "Refused",
    detail: "A term of the owner's mandate does not hold. Nothing is quoted, registered or sent." },
  contract_only: { tone: "seal", label: "Decided by the contract",
    detail: "The local checker cannot see this. x3-guard refuses it on NEAR; the cited contract tests show how." },
};

/** Evaluate one scenario from scenarios.json. Throws an X3Error for a program that won't compile. */
export async function evaluate(data, scenario) {
  const nowMs = Date.parse(data.evaluated_at);
  const tokens = data.tokens.records;
  const parsedMandate = x3.parseMandate(data.mandate.source);
  const mandate = x3.lowerMandate(parsedMandate, tokens, nowMs, await x3.sha256Hex(data.mandate.source));
  const intent = x3.parse(scenario.intent.source);
  const errors = x3.typecheck(intent);
  if (errors.length) throw new x3.X3Error(errors[0].code, errors[0].message, { line: errors[0].line });
  const leg = x3.lower(intent, nowMs);
  const base = { intent, parsedMandate, mandate, leg, checks: [], denied_guards: [], approvals_needed: 0 };
  if (leg.mandate_owner !== mandate.owner) {
    return { ...base, decision: "denied_by_policy", denied_guards: ["mandate_owner"] };
  }
  const request = x3.buildQuoteRequest(leg, tokens, { dry: true });
  const policy = x3.policyFor(leg, request, await x3.sha256Hex(scenario.intent.source));
  const state = { revoked: false, spends: [], ...(scenario.assumed_guard_state ?? {}) };
  const preview = { ...mandate, version: null, ...state };
  const checks = x3.checkMandate(preview, policy, nowMs);
  const denied = checks.filter((c) => !c.ok).map((c) => c.guard);
  const approvals = x3.approvalsNeeded(preview, policy.amount);
  const decision = denied.length ? "denied_by_policy" : approvals ? "approval_required" : "preflight_passed";
  return { ...base, request, policy, state, checks, denied_guards: denied, approvals_needed: approvals, decision };
}

/** The decision a judge should see. A contract-only case never shows as "passed". */
export function shownDecision(scenario, result) {
  return scenario.contract_only ? "contract_only" : result.decision;
}

/** True when the browser and Python reached the same decision through the same guards. */
export function agrees(scenario, result) {
  const py = scenario.python;
  return py.decision === result.decision
    && py.approvals_needed === result.approvals_needed
    && JSON.stringify(py.denied_guards) === JSON.stringify(result.denied_guards)
    && JSON.stringify(py.checks) === JSON.stringify(result.checks.map((c) => ({ guard: c.guard, ok: c.ok })));
}

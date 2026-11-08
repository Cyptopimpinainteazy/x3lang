/**
 * Non-authoritative OFFLINE judge preview. This module makes no blockchain calls,
 * does not sign transactions, and must never be treated as enforcement.
 * The real testnet-only Rust vault is in draft PR #125 and its local Sandbox
 * receipts have a separate, explicitly classified evidence artifact.
 *
 * One implementation serves the page, the scripted attack scenarios and the
 * tests: `tracePreviewRequest` evaluates the rules in a fixed order and stops at
 * the first refusal; `previewRequest` returns only its decision. The session
 * ledger is browser memory for the preview, not chain state.
 */
export const PREVIEW = Object.freeze({
  agent: 'agent.testnet',
  receiver: 'allowed.testnet',
  vault: 'vault.testnet',
  perPayment: '0.2',
  budget: '0.4',
  asset: 'native test NEAR',
  classification: 'offline_preview',
});

const YOCTO_PER_NEAR = 10n ** 24n;
const AMOUNT_RE = /^(?:0|[1-9][0-9]*)(?:\.[0-9]{1,24})?$/;
const NONCE_RE = /^[a-zA-Z0-9_-]{1,64}$/;
const ALLOW = Object.freeze({allowed: true, code: 'POLICY_PREVIEW_ALLOWED', level: 'offline_preview'});
const DENY = (code) => ({allowed: false, code, level: 'offline_preview'});

/** Exact decimal NEAR string -> yoctoNEAR BigInt, or null. Never uses floating point. */
export function parseNearAmount(input) {
  if (typeof input !== 'string' || !AMOUNT_RE.test(input)) return null;
  const [whole, fraction = ''] = input.split('.');
  try { return BigInt(whole) * YOCTO_PER_NEAR + BigInt(fraction.padEnd(24, '0') || '0'); }
  catch { return null; }
}

/** yoctoNEAR BigInt -> shortest exact decimal NEAR string accepted by parseNearAmount. */
export function formatNearAmount(yocto) {
  if (typeof yocto !== 'bigint' || yocto < 0n) throw new TypeError('yoctoNEAR must be a non-negative BigInt');
  const whole = yocto / YOCTO_PER_NEAR;
  const fraction = yocto % YOCTO_PER_NEAR;
  if (fraction === 0n) return whole.toString();
  return `${whole}.${fraction.toString().padStart(24, '0').replace(/0+$/, '')}`;
}

const PER_PAYMENT_YOCTO = parseNearAmount(PREVIEW.perPayment);
const BUDGET_YOCTO = parseNearAmount(PREVIEW.budget);
export const LIMITS = Object.freeze({perPayment: PER_PAYMENT_YOCTO, budget: BUDGET_YOCTO});

/** Rules in evaluation order. The first failing rule decides; later rules are not reached. */
export const CHECKS = Object.freeze([
  {id: 'context', code: 'INVALID_CONTEXT', label: 'Request is a well-formed object'},
  {id: 'scope', code: 'ACCESS_KEY_SCOPE', label: 'Key scope allows only request_payment'},
  {id: 'agent', code: 'AGENT_DENIED', label: `Caller is the designated agent (${PREVIEW.agent})`},
  {id: 'lifecycle', code: 'INVALID_CONTEXT', label: 'Revocation and expiry flags are explicit booleans'},
  {id: 'revoked', code: 'MANDATE_REVOKED', label: 'Owner has not revoked the mandate'},
  {id: 'expiry', code: 'MANDATE_EXPIRED', label: 'Mandate has not expired'},
  {id: 'nonce_format', code: 'NONCE_INVALID', label: 'Nonce is 1–64 letters, digits, _ or -'},
  {id: 'nonce_fresh', code: 'NONCE_REPLAY', label: 'Nonce has never been consumed'},
  {id: 'recipient', code: 'RECIPIENT_DENIED', label: `Recipient is owner-approved (${PREVIEW.receiver})`},
  {id: 'amount', code: 'INVALID_AMOUNT', label: 'Amount is a positive exact decimal'},
  {id: 'accounting', code: 'INVALID_CONTEXT', label: 'Spent and pending totals are valid'},
  {id: 'cap', code: 'PER_PAYMENT_CAP', label: `At most ${PREVIEW.perPayment} NEAR per payment`},
  {id: 'budget', code: 'TOTAL_BUDGET_EXCEEDED', label: `Spent + pending + amount ≤ ${PREVIEW.budget} NEAR`},
].map((check) => Object.freeze(check)));

/**
 * Evaluate every rule in order and report each one as pass, fail or not_reached.
 * Returns {decision, steps, amountYocto}; amountYocto is set only when the amount parsed.
 */
export function tracePreviewRequest(data) {
  const parsed = {amount: null, spent: null, pending: null};
  const rules = {
    context: () => Boolean(data) && typeof data === 'object',
    scope: () => data.action === 'request_payment',
    agent: () => data.agent === PREVIEW.agent,
    // Missing or non-boolean flags fail closed (a string 'true' must never read as "not revoked").
    lifecycle: () => typeof data.revoked === 'boolean' && typeof data.expired === 'boolean',
    revoked: () => data.revoked !== true,
    expiry: () => data.expired !== true,
    nonce_format: () => typeof data.nonce === 'string' && NONCE_RE.test(data.nonce),
    nonce_fresh: () => Array.isArray(data.used) && !data.used.includes(data.nonce),
    recipient: () => data.recipient === PREVIEW.receiver,
    amount: () => {
      parsed.amount = parseNearAmount(data.amount);
      return parsed.amount !== null && parsed.amount !== 0n;
    },
    accounting: () => {
      parsed.spent = parseNearAmount(data.spent);
      parsed.pending = parseNearAmount(data.pending);
      return parsed.spent !== null && parsed.pending !== null;
    },
    cap: () => parsed.amount <= PER_PAYMENT_YOCTO,
    budget: () => parsed.spent + parsed.pending + parsed.amount <= BUDGET_YOCTO,
  };
  const steps = [];
  let failed = null;
  for (const check of CHECKS) {
    if (failed) {
      steps.push({...check, status: 'not_reached'});
      continue;
    }
    const passed = rules[check.id]();
    steps.push({...check, status: passed ? 'pass' : 'fail'});
    if (!passed) failed = check;
  }
  return {
    decision: failed ? DENY(failed.code) : {...ALLOW},
    steps,
    amountYocto: parsed.amount,
  };
}

export function previewRequest(data) {
  return tracePreviewRequest(data).decision;
}

/* ── Session ledger (browser memory for the preview; never chain state) ───────── */

export function createLedger() {
  return Object.freeze({spent: 0n, pending: 0n, used: Object.freeze([])});
}

function assertLedger(ledger) {
  if (!ledger || typeof ledger.spent !== 'bigint' || typeof ledger.pending !== 'bigint'
      || ledger.spent < 0n || ledger.pending < 0n || !Array.isArray(ledger.used)) {
    throw new TypeError('invalid preview ledger');
  }
}

/**
 * Evaluate `request` against the ledger. The ledger supplies spent, pending and used
 * nonces, overriding any such fields on the request. Only an allowed request consumes
 * its nonce and adds its amount to spent; a refusal returns the ledger unchanged.
 */
export function submitToLedger(ledger, request) {
  assertLedger(ledger);
  const full = {
    ...request,
    used: [...ledger.used],
    spent: formatNearAmount(ledger.spent),
    pending: formatNearAmount(ledger.pending),
  };
  const trace = tracePreviewRequest(full);
  if (!trace.decision.allowed) return {request: full, trace, ledger};
  const next = Object.freeze({
    spent: ledger.spent + trace.amountYocto,
    pending: ledger.pending,
    used: Object.freeze([...ledger.used, full.nonce]),
  });
  return {request: full, trace, ledger: next};
}

/* ── Scripted scenarios: shared by the page and the tests ────────────────────── */

export const LEGIT_REQUEST = Object.freeze({
  action: 'request_payment',
  agent: PREVIEW.agent,
  recipient: PREVIEW.receiver,
  amount: '0.1',
  nonce: 'pay-1',
  revoked: false,
  expired: false,
});

const scenario = (s) => Object.freeze({...s, steps: Object.freeze(s.steps.map((x) => Object.freeze(x)))});

export const SCENARIOS = Object.freeze([
  scenario({id: 'allowed', label: 'Legit payment', kind: 'control', expect: 'POLICY_PREVIEW_ALLOWED',
    story: 'The agent pays the approved recipient 0.1 NEAR inside its limits.',
    steps: [{nonce: 'pay-1'}]}),
  scenario({id: 'redirect', label: 'Redirect funds', kind: 'attack', expect: 'RECIPIENT_DENIED',
    story: 'A prompt-injected agent tries to send the payment to attacker.testnet.',
    steps: [{recipient: 'attacker.testnet', nonce: 'atk-redirect'}]}),
  scenario({id: 'impostor', label: 'Impostor agent', kind: 'attack', expect: 'AGENT_DENIED',
    story: 'A different account calls the vault pretending to be the agent.',
    steps: [{agent: 'attacker.testnet', nonce: 'atk-impostor'}]}),
  scenario({id: 'oversize', label: 'Oversized payment', kind: 'attack', expect: 'PER_PAYMENT_CAP',
    story: 'The agent asks for 0.3 NEAR in one payment, above the 0.2 cap.',
    steps: [{amount: '0.3', nonce: 'atk-oversize'}]}),
  scenario({id: 'drain', label: 'Budget drain', kind: 'attack', expect: 'TOTAL_BUDGET_EXCEEDED',
    story: 'Two max-size payments use the whole 0.4 budget; a third small one tries to go over it.',
    steps: [{amount: '0.2', nonce: 'drain-1'}, {amount: '0.2', nonce: 'drain-2'}, {amount: '0.1', nonce: 'drain-3'}]}),
  scenario({id: 'replay', label: 'Nonce replay', kind: 'attack', expect: 'NONCE_REPLAY',
    story: 'A valid payment goes through, then the exact same request is resubmitted.',
    steps: [{nonce: 'replay-1'}, {nonce: 'replay-1'}]}),
  scenario({id: 'revoked', label: 'After revocation', kind: 'attack', expect: 'MANDATE_REVOKED',
    story: 'The owner revoked the mandate; the agent keeps trying to pay.',
    steps: [{revoked: true, nonce: 'atk-revoked'}]}),
  scenario({id: 'expired', label: 'Expired mandate', kind: 'attack', expect: 'MANDATE_EXPIRED',
    story: 'The mandate deadline has passed; the agent submits anyway.',
    steps: [{expired: true, nonce: 'atk-expired'}]}),
  scenario({id: 'direct', label: 'Direct transfer', kind: 'attack', expect: 'ACCESS_KEY_SCOPE',
    story: 'The agent skips the vault and tries a native NEAR Transfer with its key.',
    steps: [{action: 'native_transfer', nonce: 'atk-direct'}]}),
  scenario({id: 'smuggle', label: 'Number smuggle', kind: 'attack', expect: 'INVALID_AMOUNT',
    story: 'The agent sends "1e9" hoping a float parser reads it as a small number.',
    steps: [{amount: '1e9', nonce: 'atk-smuggle'}]}),
]);

/** Run one scenario from an empty ledger. `matched` is true when the last step's code is the expected one. */
export function runScenario(definition) {
  let ledger = createLedger();
  const runs = [];
  for (const step of definition.steps) {
    const run = submitToLedger(ledger, {...LEGIT_REQUEST, ...step});
    runs.push(run);
    ledger = run.ledger;
  }
  const final = runs[runs.length - 1];
  return {runs, final, ledger, matched: final.trace.decision.code === definition.expect};
}

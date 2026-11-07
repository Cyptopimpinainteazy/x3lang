/**
 * Non-authoritative OFFLINE judge preview. This module makes no blockchain calls,
 * does not sign transactions, and must never be treated as enforcement.
 * The real testnet-only Rust vault is in draft PR #125 and its local Sandbox
 * receipts have a separate, explicitly classified evidence artifact.
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

const DENY = (code) => ({allowed:false,code,level:'offline_preview'});
function parseYocto(input) {
  if (typeof input !== 'string' || !/^(?:0|[1-9][0-9]*)(?:\.[0-9]{1,24})?$/.test(input)) return null;
  const [whole, fraction=''] = input.split('.');
  try { return BigInt(whole)*10n**24n + BigInt(fraction.padEnd(24,'0') || '0'); }
  catch { return null; }
}
export function previewRequest(data) {
  if (!data || typeof data !== 'object') return DENY('INVALID_CONTEXT');
  if (data.action !== 'request_payment') return DENY('ACCESS_KEY_SCOPE');
  if (data.agent !== PREVIEW.agent) return DENY('AGENT_DENIED');
  if (data.revoked === true) return DENY('MANDATE_REVOKED');
  if (data.expired === true) return DENY('MANDATE_EXPIRED');
  if (typeof data.nonce !== 'string' || !/^[a-zA-Z0-9_-]{1,64}$/.test(data.nonce)) return DENY('NONCE_INVALID');
  if (!Array.isArray(data.used) || data.used.includes(data.nonce)) return DENY('NONCE_REPLAY');
  if (data.recipient !== PREVIEW.receiver) return DENY('RECIPIENT_DENIED');

  const amount=parseYocto(data.amount);
  const spent=parseYocto(data.spent), pending=parseYocto(data.pending);
  if (amount===null || amount===0n) return DENY('INVALID_AMOUNT');
  if (spent===null || pending===null) return DENY('INVALID_CONTEXT');
  const max=parseYocto(PREVIEW.perPayment), total=parseYocto(PREVIEW.budget);
  if (amount>max) return DENY('PER_PAYMENT_CAP');
  if (spent+pending+amount>total) return DENY('TOTAL_BUDGET_EXCEEDED');
  return {allowed:true,code:'POLICY_PREVIEW_ALLOWED',level:'offline_preview'};
}

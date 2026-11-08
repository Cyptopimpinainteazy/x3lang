import assert from 'node:assert/strict';
import {test} from 'node:test';
import {
  CHECKS, LEGIT_REQUEST, LIMITS, PREVIEW, SCENARIOS, createLedger, formatNearAmount,
  parseNearAmount, previewRequest, runScenario, submitToLedger, tracePreviewRequest,
} from './attack-lab-policy.mjs';

const base = {
  action:'request_payment',agent:'agent.testnet',recipient:'allowed.testnet',
  amount:'0.1',nonce:'fresh',used:[],revoked:false,expired:false,
  spent:'0',pending:'0'
};
const check=(overrides={})=>previewRequest({...base,...overrides});

test('one allowed request passes this offline preview',()=>{
  assert.deepEqual(check(),{allowed:true,code:'POLICY_PREVIEW_ALLOWED',level:'offline_preview'});
});
test('attacker recipient fails',()=>assert.equal(check({recipient:'attacker.testnet'}).code,'RECIPIENT_DENIED'));
test('missing agent permission fails',()=>assert.equal(check({agent:'attacker.testnet'}).code,'AGENT_DENIED'));
test('spending cap fails',()=>assert.equal(check({amount:'0.3'}).code,'PER_PAYMENT_CAP'));
test('replay fails',()=>assert.equal(check({used:['fresh']}).code,'NONCE_REPLAY'));
test('revocation fails',()=>assert.equal(check({revoked:true}).code,'MANDATE_REVOKED'));
test('deadline fails',()=>assert.equal(check({expired:true}).code,'MANDATE_EXPIRED'));
test('missing or malformed lifecycle flags fail closed',()=>{
  for (const field of ['revoked','expired']) {
    for (const value of [undefined,null,'false','true',0,1,[],{}]) {
      assert.deepEqual(check({[field]:value}),{
        allowed:false,code:'INVALID_CONTEXT',level:'offline_preview'
      },`${field}=${JSON.stringify(value)}`);
    }
    const missing={...base};delete missing[field];
    assert.equal(previewRequest(missing).code,'INVALID_CONTEXT');
  }
});
test('pending commitments count towards cumulative budget',()=>assert.equal(check({spent:'0.2',pending:'0.15'}).code,'TOTAL_BUDGET_EXCEEDED'));
test('invalid numeric amounts refuse, never float round',()=>{
  for (const amount of ['NaN','-0.1','0','0.1000000000000000000000001','1e9']) {
    assert.equal(check({amount}).allowed,false,amount);
  }
});
test('non-function-call action never allowed in preview',()=>assert.equal(check({action:'native_transfer'}).code,'ACCESS_KEY_SCOPE'));
test('everything is labeled preview, never a chain tx',()=>{
  const r=check({recipient:'attacker.testnet'});
  assert.equal(r.level,'offline_preview');assert.notEqual(r.code,'CHAIN_FINALIZED');
});

/* ── Refactor parity: frozen copy of the PR #126 implementation (test-only reference) ── */
function referencePreviewRequest(data) {
  const DENY = (code) => ({allowed:false,code,level:'offline_preview'});
  const parseYocto = (input) => {
    if (typeof input !== 'string' || !/^(?:0|[1-9][0-9]*)(?:\.[0-9]{1,24})?$/.test(input)) return null;
    const [whole, fraction=''] = input.split('.');
    try { return BigInt(whole)*10n**24n + BigInt(fraction.padEnd(24,'0') || '0'); } catch { return null; }
  };
  if (!data || typeof data !== 'object') return DENY('INVALID_CONTEXT');
  if (data.action !== 'request_payment') return DENY('ACCESS_KEY_SCOPE');
  if (data.agent !== 'agent.testnet') return DENY('AGENT_DENIED');
  // master 5df23da: missing or non-boolean lifecycle flags fail closed.
  if (typeof data.revoked !== 'boolean' || typeof data.expired !== 'boolean') return DENY('INVALID_CONTEXT');
  if (data.revoked === true) return DENY('MANDATE_REVOKED');
  if (data.expired === true) return DENY('MANDATE_EXPIRED');
  if (typeof data.nonce !== 'string' || !/^[a-zA-Z0-9_-]{1,64}$/.test(data.nonce)) return DENY('NONCE_INVALID');
  if (!Array.isArray(data.used) || data.used.includes(data.nonce)) return DENY('NONCE_REPLAY');
  if (data.recipient !== 'allowed.testnet') return DENY('RECIPIENT_DENIED');
  const amount=parseYocto(data.amount), spent=parseYocto(data.spent), pending=parseYocto(data.pending);
  if (amount===null || amount===0n) return DENY('INVALID_AMOUNT');
  if (spent===null || pending===null) return DENY('INVALID_CONTEXT');
  if (amount>parseYocto('0.2')) return DENY('PER_PAYMENT_CAP');
  if (spent+pending+amount>parseYocto('0.4')) return DENY('TOTAL_BUDGET_EXCEEDED');
  return {allowed:true,code:'POLICY_PREVIEW_ALLOWED',level:'offline_preview'};
}

test('refactored evaluator returns identical decisions to the original on 20,000 seeded inputs',()=>{
  let seed=0x5eed1234;
  const rand=()=>{seed=(seed*1103515245+12345)>>>0;return seed/2**32;};
  const pick=(xs)=>xs[Math.floor(rand()*xs.length)];
  const pools={
    action:['request_payment','native_transfer','',undefined,null,'REQUEST_PAYMENT'],
    agent:['agent.testnet','attacker.testnet','',undefined,'agent.testnet '],
    recipient:['allowed.testnet','attacker.testnet',undefined,'Allowed.testnet'],
    amount:['0.1','0.2','0.2000000000000000000000001','0.3','0','0.0','1e9','NaN','-1','01','.5','0.000000000000000000000001',undefined,1,'0.15','0.05'],
    nonce:['fresh','n-1','','a'.repeat(64),'a'.repeat(65),'bad nonce','ok_1',undefined,7],
    used:[[],['fresh'],['n-1','ok_1'],undefined,'fresh',null],
    revoked:[false,true,'true',undefined,1],
    expired:[false,true,'yes',undefined],
    spent:['0','0.1','0.2','0.25','0.3','0.4','x',undefined,'-0.1'],
    pending:['0','0.05','0.15','0.2',undefined,'1e3'],
  };
  for (let i=0;i<20000;i++) {
    const data={};
    for (const [k,xs] of Object.entries(pools)) { const v=pick(xs); if (v!==undefined || rand()<0.5) data[k]=v; }
    assert.deepEqual(previewRequest(data),referencePreviewRequest(data),JSON.stringify(data));
  }
  for (const odd of [null,undefined,0,'',false,'string',42,[]]) {
    assert.deepEqual(previewRequest(odd),referencePreviewRequest(odd),String(odd));
  }
});

/* ── Rule trace ── */
test('trace marks the deciding rule as fail and every later rule as not reached',()=>{
  const t=tracePreviewRequest({...base,recipient:'attacker.testnet'});
  const ids=t.steps.map(s=>s.id);
  assert.deepEqual(ids,CHECKS.map(c=>c.id));
  const at=ids.indexOf('recipient');
  assert.ok(t.steps.slice(0,at).every(s=>s.status==='pass'));
  assert.equal(t.steps[at].status,'fail');
  assert.ok(t.steps.slice(at+1).every(s=>s.status==='not_reached'));
  assert.equal(t.decision.code,'RECIPIENT_DENIED');
});
test('an allowed trace passes all thirteen rules',()=>{
  const t=tracePreviewRequest(base);
  assert.equal(t.steps.length,13);
  assert.ok(t.steps.every(s=>s.status==='pass'));
  assert.equal(t.amountYocto,10n**23n);
});
test('exactly one rule fails in any denied trace and its code is the decision',()=>{
  for (const o of [{amount:'0.3'},{used:['fresh']},{nonce:'no spaces'},{spent:'x'},{action:'x'},{expired:true}]) {
    const t=tracePreviewRequest({...base,...o});
    const fails=t.steps.filter(s=>s.status==='fail');
    assert.equal(fails.length,1,JSON.stringify(o));
    assert.equal(fails[0].code,t.decision.code);
  }
});

/* ── Exact amounts ── */
test('amount formatting round-trips exactly with no floating point',()=>{
  for (const s of ['0','0.1','0.2','0.4','1','12.5','0.000000000000000000000001','3.141592653589793238462643']) {
    assert.equal(formatNearAmount(parseNearAmount(s)),s);
  }
  assert.equal(LIMITS.perPayment,parseNearAmount(PREVIEW.perPayment));
  assert.equal(LIMITS.budget,parseNearAmount(PREVIEW.budget));
  assert.throws(()=>formatNearAmount(-1n),TypeError);
  assert.throws(()=>formatNearAmount(1),TypeError);
});

/* ── Session ledger ── */
test('an allowed request consumes its nonce and adds to spent; a refusal changes nothing',()=>{
  const l0=createLedger();
  const ok=submitToLedger(l0,{...LEGIT_REQUEST,nonce:'a1'});
  assert.equal(ok.trace.decision.allowed,true);
  assert.equal(ok.ledger.spent,10n**23n);
  assert.deepEqual([...ok.ledger.used],['a1']);
  const replay=submitToLedger(ok.ledger,{...LEGIT_REQUEST,nonce:'a1'});
  assert.equal(replay.trace.decision.code,'NONCE_REPLAY');
  assert.equal(replay.ledger,ok.ledger);
  assert.deepEqual([...l0.used],[],'original ledger is not mutated');
});
test('ledger values override spent, pending and used supplied on the request',()=>{
  const r=submitToLedger(createLedger(),{...LEGIT_REQUEST,spent:'0.4',pending:'0.4',used:[LEGIT_REQUEST.nonce]});
  assert.equal(r.trace.decision.allowed,true);
  assert.equal(r.request.spent,'0');
});
test('the ledger refuses once the 0.4 budget is reached',()=>{
  let l=createLedger();
  for (const n of ['b1','b2']) l=submitToLedger(l,{...LEGIT_REQUEST,amount:'0.2',nonce:n}).ledger;
  assert.equal(l.spent,LIMITS.budget);
  assert.equal(submitToLedger(l,{...LEGIT_REQUEST,amount:'0.000000000000000000000001',nonce:'b3'}).trace.decision.code,'TOTAL_BUDGET_EXCEEDED');
});
test('malformed ledgers fail closed',()=>{
  for (const bad of [null,{},{spent:0,pending:0n,used:[]},{spent:-1n,pending:0n,used:[]},{spent:0n,pending:0n,used:'x'}]) {
    assert.throws(()=>submitToLedger(bad,LEGIT_REQUEST),TypeError);
  }
});

/* ── Scripted scenarios (the page runs exactly these) ── */
test('every scripted scenario ends with its expected decision',()=>{
  for (const s of SCENARIOS) {
    const r=runScenario(s);
    assert.equal(r.final.trace.decision.code,s.expect,s.id);
    assert.equal(r.matched,true,s.id);
  }
});
test('scenario set covers one control and every refusal code a judge can trigger',()=>{
  assert.equal(new Set(SCENARIOS.map(s=>s.id)).size,SCENARIOS.length);
  assert.deepEqual(SCENARIOS.filter(s=>s.kind==='control').map(s=>s.expect),['POLICY_PREVIEW_ALLOWED']);
  const attacked=new Set(SCENARIOS.filter(s=>s.kind==='attack').map(s=>s.expect));
  for (const code of ['RECIPIENT_DENIED','AGENT_DENIED','PER_PAYMENT_CAP','TOTAL_BUDGET_EXCEEDED','NONCE_REPLAY','MANDATE_REVOKED','MANDATE_EXPIRED','ACCESS_KEY_SCOPE','INVALID_AMOUNT']) {
    assert.ok(attacked.has(code),code);
  }
});
test('multi-step scenarios allow their setup steps before the attack is refused',()=>{
  const drain=runScenario(SCENARIOS.find(s=>s.id==='drain'));
  assert.deepEqual(drain.runs.map(r=>r.trace.decision.allowed),[true,true,false]);
  const replay=runScenario(SCENARIOS.find(s=>s.id==='replay'));
  assert.deepEqual(replay.runs.map(r=>r.trace.decision.code),['POLICY_PREVIEW_ALLOWED','NONCE_REPLAY']);
});

import assert from 'node:assert/strict';
import {test} from 'node:test';
import {previewRequest} from './attack-lab-policy.mjs';

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

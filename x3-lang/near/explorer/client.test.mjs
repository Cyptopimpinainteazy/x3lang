import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createExplorerClient,queryParams,normalizeTransactions} from './client.mjs';
const row={depositAddress:'deposit',depositMemo:'7',status:'SUCCESS',recipient:'owner.near'};
function setup(status=200,body=[row]) {
 const env={NEAR_INTENTS_EXPLORER_JWT:`test.${Buffer.from(JSON.stringify({partner_id:'test',key_type:'explorer'})).toString('base64url')}.test`,X3_EXPLORER_SEARCH:'owner.near',X3_EXPLORER_REDIS_URL:'https://redis.invalid',X3_EXPLORER_REDIS_TOKEN:'test-only'};
 const values=new Map();let calls=0;let time=0;
 const fetchImpl=async(url,options)=>{
  if(url===env.X3_EXPLORER_REDIS_URL){
   const [op,key,...args]=JSON.parse(options.body);let result=null;
   if(op==='GET') result=values.get(key)?.value || null;
   if(op==='SET') {
    if(!args.includes('NX') || !values.has(key) || values.get(key).until<=time){values.set(key,{value:args[0],until:time+30000});result='OK';}
   }
   if(op==='EVAL'){const lock=args[1];const owner=args[2];if(values.get(lock)?.value===owner)values.get(lock).until=time+Number(args[3]);result=1;}
   return Response.json({result});
  }
  calls++; assert.equal(options.redirect,'error');assert.ok(url.startsWith('https://explorer.near-intents.org/api/v0/transactions?'));
  assert.equal(options.headers.Authorization,`Bearer ${env.NEAR_INTENTS_EXPLORER_JWT}`);
  return Response.json(body,{status,headers:{'Retry-After':'12'}});
 };
 return {env,fetchImpl,get calls(){return calls},advance:n=>time+=n};
}
test('rejects arbitrary query keys, override of scope, duplicate filters, and orphan memo',()=>{
 for(const query of ['search=attacker','url=https://attacker','statuses=SUCCESS&statuses=FAILED','lastDepositMemo=1','statuses=UNKNOWN'])
  assert.throws(()=>queryParams(new URLSearchParams(query),'owner.near'));
 assert.throws(()=>queryParams(new URLSearchParams(),''));
});
test('provider SUCCESS never upgrades evidence and unknown fields are dropped',()=>{
 const data=normalizeTransactions([{...row,independentlyVerified:true,secret:'do not expose'}],'now');
 assert.equal(data.independentlyVerified,false);assert.equal(data.executionAllowed,false);assert.equal(data.transactions[0].secret,undefined);
 assert.throws(()=>normalizeTransactions({},'now'));assert.throws(()=>normalizeTransactions([{...row,status:'invented'}],'now'));
});
test('cursor retains memo when page is full',()=>{
 assert.deepEqual(normalizeTransactions(Array(25).fill(row),'now').nextCursor,{lastDepositAddress:'deposit',lastDepositMemo:'7'});
});
test('cached reads and shared limiter span client instances and filters',async()=>{
 const s=setup();const a=createExplorerClient(s);const b=createExplorerClient(s);
 assert.equal((await a.transactions()).cached,false);
 assert.equal((await b.transactions()).cached,true);assert.equal(s.calls,1);
 await assert.rejects(()=>b.transactions(new URLSearchParams('statuses=FAILED')),e=>e.status===429);
 s.advance(5001);await b.transactions(new URLSearchParams('statuses=FAILED'));assert.equal(s.calls,2);
});
test('concurrent clients make at most one upstream call',async()=>{
 const s=setup();const results=await Promise.allSettled([createExplorerClient(s).transactions(),createExplorerClient(s).transactions(new URLSearchParams('statuses=FAILED'))]);
 assert.equal(s.calls,1);assert.equal(results.filter(r=>r.status==='rejected').length,1);
});
test('upstream 429 respects retry-after and errors never contain credentials',async()=>{
 const s=setup(429,{token:'must not leak'});
 await assert.rejects(()=>createExplorerClient(s).transactions(),e=>e.retryAfter===12 && !e.message.includes('leak'));
 s.advance(6000);await assert.rejects(()=>createExplorerClient(s).transactions(),e=>e.status===429);assert.equal(s.calls,1);
});
test('auth failures are sanitized',async()=>{
 const s=setup(401,{secret:'sensitive'});
 await assert.rejects(()=>createExplorerClient(s).transactions(),e=>e.code==='EXPLORER_AUTH_REJECTED' && !e.message.includes('sensitive'));
});
test('missing config and coordinator failure fail closed',async()=>{
 await assert.rejects(()=>createExplorerClient({env:{}}).transactions(),e=>e.code==='EXPLORER_NOT_CONFIGURED');
 const s=setup();s.fetchImpl=async()=>{throw Error(s.env.NEAR_INTENTS_EXPLORER_JWT)};
 await assert.rejects(()=>createExplorerClient(s).transactions(),e=>e.message==='EXPLORER_UNAVAILABLE');
});

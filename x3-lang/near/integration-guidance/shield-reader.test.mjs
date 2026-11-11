import {test} from 'node:test';import assert from 'node:assert/strict';
import {publicIncidents,createShieldReader} from './shield.mjs';
test('strips internal fields and never infers modes',()=>{const r=publicIncidents({status:'incidents',incidents:[{scopeType:'chain',scopeValue:'eth',description:'PRIVATE',createdBy:'private',publicDescription:'Delayed'}]},'now');assert.ok(!JSON.stringify(r).includes('PRIVATE'));assert.equal(r.securityMode,'unknown');assert.equal(publicIncidents({status:'operational'},'now').securityMode,'unknown');assert.throws(()=>publicIncidents({status:'incidents',incidents:[{}]},'now'));});
test('unconfigured and failed feeds return unknown without leaking',async()=>{assert.equal((await createShieldReader({env:{}})()).status,'unknown');const read=createShieldReader({env:{NEAR_INTENTS_SHIELD_JWT:'test-only'},fetcher:async()=>{throw Error('secret')}});assert.equal((await read()).reason,'unavailable');});
test('cache expires, coalesces, and stale success is not returned on failure',async()=>{let t=0,calls=0;const read=createShieldReader({env:{NEAR_INTENTS_SHIELD_JWT:'test-only'},clock:()=>t,fetcher:async()=>{calls++;if(calls>1)throw Error();return Response.json({status:'operational'});}});await Promise.all([read(),read()]);assert.equal(calls,1);t=31000;assert.equal((await read()).status,'unknown');});

test("contradictory operational responses and blank scopes fail closed", async () => {
  const bad = [
    { status: "operational", incidents: [{ scopeType: "chain", scopeValue: "eth" }] },
    { status: "operational", incidents: null },
    { status: "operational", incidents: {} },
    { status: "incidents", incidents: [{ scopeType: "chain", scopeValue: "  " }] },
    { status: "incidents", incidents: [null] },
  ];
  for (const payload of bad) {
    let calls = 0;
    const read = createShieldReader({
      env: { NEAR_INTENTS_SHIELD_JWT: "test-only" },
      fetcher: async () => Response.json(++calls === 1 ? payload : { status: "operational" }),
    });
    assert.equal((await read()).status, "unknown");
    assert.equal((await read()).status, "operational");
    assert.equal(calls, 2, "invalid evidence must not be cached as operational");
  }
});

test("oversize provider bodies are cancelled before parsing", async () => {
  for (const declaredLength of [false, true]) {
    let pulls = 0;
    let cancelled = false;
    const stream = new ReadableStream({
      pull(controller) {
        pulls++;
        controller.enqueue(new Uint8Array(512 * 1024).fill(32));
      },
      cancel() { cancelled = true; },
    }, { highWaterMark: 0 });
    const read = createShieldReader({
      env: { NEAR_INTENTS_SHIELD_JWT: "test-only" },
      fetcher: async () => new Response(stream, {
        headers: declaredLength ? { "content-length": String(1024 * 1024 + 1) } : {},
      }),
    });
    const result = await read();
    assert.equal(result.status, "unknown");
    assert.equal(result.reason, "unavailable");
    assert.deepEqual(result.incidents, []);
    assert.equal(cancelled, true);
    assert.equal(pulls, declaredLength ? 0 : 3);
  }
});

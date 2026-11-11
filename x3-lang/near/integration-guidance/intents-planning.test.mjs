import {test} from 'node:test';
import assert from 'node:assert/strict';
import {planDeadline,genericPublicFees} from './planning.ts';
const args={dry:false,nowMs:1000000,defaultSeconds:1800,maximumSeconds:3600,ownerExpiresMs:4600000};
test('deadline boundaries and owner cap fail closed',()=>{
 for(const [minutes,window,ok] of [[5,30,true],[35,35,true],[60,60,true],[75,60,false]]){const r=planDeadline({...args,estimateSeconds:minutes*60});assert.equal(r.ok,ok);assert.equal(Date.parse(r.deadline),args.nowMs+window*60000);}
 assert.equal(planDeadline({...args,estimateSeconds:2100,ownerExpiresMs:args.nowMs+600000}).ok,false);
 for(const estimateSeconds of [undefined,NaN,-1,0])assert.equal(planDeadline({...args,estimateSeconds}).ok,false);
 assert.equal(planDeadline({...args,dry:true}).reason,'Informational only; never executable');
});
test('generic fee floor, rounding and cap',()=>{
 assert.deepEqual(genericPublicFees(21,true,false),{developer:11,provider:20,total:31,withinCap:true});
 assert.equal(genericPublicFees(40,true,false).total,40);
 assert.equal(genericPublicFees(0,true,true).total,1);
 assert.equal(genericPublicFees(480,false,false).withinCap,false);
 assert.throws(()=>genericPublicFees(1.1,true,false));
});

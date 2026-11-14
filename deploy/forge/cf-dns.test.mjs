import assert from 'node:assert/strict';
import test from 'node:test';
import {client, route, zoneFor, DnsError} from './cf-dns.mjs';

const ACCOUNT = 'a'.repeat(32);
const TUNNEL = '12345678-1234-1234-1234-123456789abc';

function fake({zones = {'dotx3.net': 'z1'}, records = []} = {}) {
  const calls = [];
  const fetchImpl = async (url, init) => {
    const u = new URL(url);
    calls.push({method: init.method, path: u.pathname, body: init.body ? JSON.parse(init.body) : undefined});
    let result;
    if (u.pathname === '/client/v4/zones') {
      const id = zones[u.searchParams.get('name')];
      assert.equal(u.searchParams.get('account.id'), ACCOUNT);
      result = id ? [{id, name: u.searchParams.get('name')}] : [];
    } else if (init.method === 'GET') {
      result = records;
    } else {
      result = {id: 'r-new'};
    }
    return {ok: true, json: async () => ({success: true, result})};
  };
  return {c: client({token: 't', accountId: ACCOUNT, fetchImpl}), calls};
}

test('finds the zone for a subdomain', async () => {
  const {c} = fake();
  assert.deepEqual(await zoneFor(c, 'forge.dotx3.net'), {id: 'z1', name: 'dotx3.net'});
});

test('refuses a host outside the account', async () => {
  const {c} = fake({zones: {}});
  await assert.rejects(zoneFor(c, 'forge.dotx3.net'), /not in an active Cloudflare zone/);
});

test('creates a proxied CNAME to the tunnel', async () => {
  const {c, calls} = fake();
  const out = await route(c, 'forge.dotx3.net', TUNNEL);
  assert.equal(out.changed, true);
  const post = calls.find((x) => x.method === 'POST');
  assert.deepEqual([post.body.type, post.body.content, post.body.proxied], ['CNAME', `${TUNNEL}.cfargotunnel.com`, true]);
});

test('leaves a correct record alone and repoints an old tunnel', async () => {
  let {c, calls} = fake({records: [{id: 'r1', type: 'CNAME', content: `${TUNNEL}.cfargotunnel.com`, proxied: true}]});
  assert.equal((await route(c, 'forge.dotx3.net', TUNNEL)).changed, false);
  assert.equal(calls.filter((x) => x.method !== 'GET').length, 0);
  ({c, calls} = fake({records: [{id: 'r1', type: 'CNAME', content: 'aaaaaaaa-1234-1234-1234-123456789abc.cfargotunnel.com', proxied: true}]}));
  assert.equal((await route(c, 'forge.dotx3.net', TUNNEL)).changed, true);
  assert.equal(calls.find((x) => x.method === 'PUT').path, '/client/v4/zones/z1/dns_records/r1');
});

test('never overwrites a record that is not a tunnel', async () => {
  const {c, calls} = fake({records: [{id: 'r1', type: 'A', content: '203.0.113.7'}]});
  await assert.rejects(route(c, 'forge.dotx3.net', TUNNEL), /refusing to overwrite/);
  assert.equal(calls.filter((x) => x.method !== 'GET').length, 0);
});

test('validates inputs', async () => {
  const {c} = fake();
  await assert.rejects(route(c, 'forge.dotx3.net', 'not-a-uuid'), DnsError);
  await assert.rejects(zoneFor(c, 'bad host'), /invalid hostname/);
  assert.throws(() => client({token: '', accountId: ACCOUNT}), /CF_API_TOKEN/);
});

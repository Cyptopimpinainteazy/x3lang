import assert from 'node:assert/strict';
import {test} from 'node:test';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {APP_NAME, POLICY_NAME, CfError, createClient, ensure, hostsFrom, isOwnerOnly, remove, verify} from './cf-access.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const ACCOUNT = '0123456789abcdef0123456789abcdef';
const TOKEN = 'test-token-never-printed';
const HOSTS = hostsFrom(path.join(here, 'sites.json'));
const EMAIL = 'Owner@Example.com';

/** In-memory Cloudflare Access API: enough of /access/{organizations,policies,apps} to exercise the helper. */
function fakeCloudflare({authDomain = 'x3team.cloudflareaccess.com'} = {}) {
  const state = {policies: new Map(), apps: new Map(), seq: 0, calls: []};
  const id = () => `00000000-0000-4000-8000-${String(++state.seq).padStart(12, '0')}`;
  const ok = (result, info) => ({ok: true, status: 200, json: async () => ({success: true, errors: [], result, ...(info ? {result_info: info} : {})})});
  const fail = (status, message) => ({ok: false, status, json: async () => ({success: false, errors: [{code: status, message}]})});
  const expand = (app) => ({...app, policies: app.policies.map((p) => ({...state.policies.get(p.id), precedence: p.precedence}))});
  async function fetchImpl(url, init) {
    assert.equal(init.headers.Authorization, `Bearer ${TOKEN}`);
    const u = new URL(url);
    assert.equal(u.origin, 'https://api.cloudflare.com', 'token only ever goes to the Cloudflare API');
    const route = u.pathname.replace(`/client/v4/accounts/${ACCOUNT}`, '');
    const body = init.body ? JSON.parse(init.body) : undefined;
    state.calls.push(`${init.method} ${route}`);
    if (route === '/access/organizations') return ok({auth_domain: authDomain});
    if (route === '/access/policies' && init.method === 'GET') return ok([...state.policies.values()], {page: 1, total_pages: 1});
    if (route === '/access/policies' && init.method === 'POST') { const p = {...body, id: id()}; state.policies.set(p.id, p); return ok(p); }
    let m = route.match(/^\/access\/policies\/([\w-]+)$/);
    if (m && init.method === 'PUT') { const p = {...body, id: m[1]}; state.policies.set(m[1], p); return ok(p); }
    if (route === '/access/apps' && init.method === 'GET') return ok([...state.apps.values()].map(expand), {page: 1, total_pages: 1});
    if (route === '/access/apps' && init.method === 'POST') { const a = {...body, id: id(), aud: 'f'.repeat(64)}; state.apps.set(a.id, a); return ok(expand(a)); }
    m = route.match(/^\/access\/apps\/([\w-]+)$/);
    if (m && init.method === 'GET') return state.apps.has(m[1]) ? ok(expand(state.apps.get(m[1]))) : fail(404, 'not found');
    if (m && init.method === 'PUT') { const a = {...state.apps.get(m[1]), ...body, id: m[1]}; state.apps.set(m[1], a); return ok(expand(a)); }
    if (m && init.method === 'DELETE') { state.apps.delete(m[1]); return ok({id: m[1]}); }
    m = route.match(/^\/access\/apps\/([\w-]+)\/policies\/([\w-]+)$/);
    if (m && init.method === 'GET') return ok(state.policies.get(m[2]));
    return fail(400, `unhandled ${init.method} ${route}`);
  }
  return {state, fetchImpl};
}

const clientFor = (cf) => createClient({token: TOKEN, accountId: ACCOUNT, fetchImpl: cf.fetchImpl});

test('hosts come from the manifest', () => {
  assert.deepEqual(HOSTS, ['controlroom.x3star.net', 'guard.x3star.net', 'judge.x3star.net', 'x3lang.x3star.net']);
});

test('ensure creates one owner-only policy and one app covering every host, then verifies it', async () => {
  const cf = fakeCloudflare();
  const result = await ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL});
  assert.deepEqual(result, {teamDomain: 'https://x3team.cloudflareaccess.com', aud: 'f'.repeat(64), appId: result.appId});
  assert.equal(cf.state.policies.size, 1);
  assert.equal(cf.state.apps.size, 1);
  const app = [...cf.state.apps.values()][0];
  assert.equal(app.name, APP_NAME);
  assert.equal(app.type, 'self_hosted');
  assert.deepEqual(app.destinations.map((d) => d.uri), HOSTS);
  assert.equal(app.app_launcher_visible, false);
  const policy = [...cf.state.policies.values()][0];
  assert.equal(policy.name, POLICY_NAME);
  assert.deepEqual(policy.include, [{email: {email: 'owner@example.com'}}]);
});

test('ensure is idempotent: a second run updates in place instead of duplicating', async () => {
  const cf = fakeCloudflare();
  const first = await ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL});
  const second = await ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL});
  assert.equal(second.appId, first.appId);
  assert.equal(cf.state.apps.size, 1);
  assert.equal(cf.state.policies.size, 1);
  assert.ok(cf.state.calls.includes(`PUT /access/apps/${first.appId}`));
});

test('verify refuses when a host is not covered', async () => {
  const cf = fakeCloudflare();
  await ensure(clientFor(cf), {hosts: HOSTS.slice(0, 2), email: EMAIL});
  await assert.rejects(verify(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /does not cover: judge\.x3star\.net, x3lang\.x3star\.net/);
});

test('verify refuses when someone widened the policy', async () => {
  const cf = fakeCloudflare();
  await ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL});
  const policy = [...cf.state.policies.values()][0];
  policy.include = [{email: {email: 'owner@example.com'}}, {everyone: {}}];
  await assert.rejects(verify(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /not owner-only/);
  policy.include = [{email_domain: {domain: 'example.com'}}];
  await assert.rejects(verify(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /not owner-only/);
});

test('verify refuses when the app is missing or duplicated', async () => {
  const cf = fakeCloudflare();
  await assert.rejects(verify(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /found 0/);
  await ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL});
  const dup = {...[...cf.state.apps.values()][0], id: 'dup'};
  cf.state.apps.set('dup', dup);
  await assert.rejects(verify(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /found 2/);
});

test('remove deletes the app and verify then fails', async () => {
  const cf = fakeCloudflare();
  const {appId} = await ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL});
  assert.deepEqual(await remove(clientFor(cf)), {removed: [appId]});
  await assert.rejects(verify(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /found 0/);
});

test('input validation fails before any API call', async () => {
  assert.throws(() => createClient({token: '', accountId: ACCOUNT}), CfError);
  assert.throws(() => createClient({token: TOKEN, accountId: 'nope'}), /32-character/);
  const cf = fakeCloudflare();
  await assert.rejects(ensure(clientFor(cf), {hosts: HOSTS, email: 'not-an-email'}), /valid email/);
  assert.equal(cf.state.calls.length, 0);
});

test('an unexpected Access auth domain is refused', async () => {
  const cf = fakeCloudflare({authDomain: 'evil.example.com'});
  await assert.rejects(ensure(clientFor(cf), {hosts: HOSTS, email: EMAIL}), /unexpected Access auth_domain/);
});

test('API errors surface Cloudflare messages, never the token', async () => {
  const client = createClient({token: TOKEN, accountId: ACCOUNT, fetchImpl: async () => ({ok: false, status: 403, json: async () => ({success: false, errors: [{code: 10000, message: 'Authentication error'}]})})});
  await assert.rejects(ensure(client, {hosts: HOSTS, email: EMAIL}), (error) => {
    assert.match(error.message, /10000: Authentication error/);
    assert.ok(!error.message.includes(TOKEN));
    return true;
  });
});

test('isOwnerOnly accepts narrowing rules and rejects widening ones', () => {
  const base = {decision: 'allow', include: [{email: {email: 'owner@example.com'}}]};
  assert.equal(isOwnerOnly(base, 'owner@example.com'), true);
  assert.equal(isOwnerOnly({...base, require: [{country: {country_code: 'US'}}]}, 'owner@example.com'), true);
  assert.equal(isOwnerOnly({...base, decision: 'bypass'}, 'owner@example.com'), false);
  assert.equal(isOwnerOnly({...base, include: [{email: {email: 'other@example.com'}}]}, 'owner@example.com'), false);
});

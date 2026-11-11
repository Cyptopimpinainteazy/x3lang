#!/usr/bin/env node
/**
 * Cloudflare Access gate for the x3star.net demo hostnames. Zero dependencies (Node >= 20).
 *
 *   node cf-access.mjs ensure   create or update the owner-only policy and the Access app
 *   node cf-access.mjs verify   exit 0 only if the app covers every host and only the owner may pass
 *   node cf-access.mjs remove   delete the Access app (release day: the pages become public)
 *
 * Environment: CF_API_TOKEN (Access: Apps and Policies Edit; Access: Organizations Read),
 * CF_ACCOUNT_ID, X3_OWNER_EMAIL (ensure/verify). The token is sent only to api.cloudflare.com
 * and is never printed. On success ensure/verify print one JSON line:
 *   {"teamDomain":"https://<team>.cloudflareaccess.com","aud":"<hex>","appId":"<uuid>"}
 */
import {readFileSync, realpathSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

export const APP_NAME = 'X3Lang demos (x3star.net)';
export const POLICY_NAME = 'X3Lang demos: owner only';
const API = 'https://api.cloudflare.com/client/v4';
const SESSION = '24h';

export class CfError extends Error {}

export function createClient({token, accountId, fetchImpl = globalThis.fetch}) {
  if (!token) throw new CfError('CF_API_TOKEN is not set');
  if (!/^[0-9a-f]{32}$/i.test(accountId || '')) throw new CfError('CF_ACCOUNT_ID must be the 32-character account id');
  const base = `${API}/accounts/${accountId}`;
  async function call(method, route, body) {
    const response = await fetchImpl(`${base}${route}`, {
      method,
      headers: {Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', Accept: 'application/json'},
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    let json = null;
    try { json = await response.json(); } catch { /* reported below */ }
    if (!response.ok || !json?.success) {
      const detail = (json?.errors || []).map((e) => `${e.code}: ${e.message}`).join('; ') || `HTTP ${response.status}`;
      throw new CfError(`${method} ${route} failed (${detail})`);
    }
    return json;
  }
  async function list(route) {
    const out = [];
    for (let page = 1; page <= 50; page += 1) {
      const json = await call('GET', `${route}?page=${page}&per_page=100`);
      out.push(...(json.result || []));
      const info = json.result_info;
      if (!info || page >= (info.total_pages || 1)) return out;
    }
    throw new CfError(`${route}: too many pages`);
  }
  return {call, list};
}

export function hostsFrom(manifestPath) {
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  const hosts = Object.keys(manifest.sites || {}).sort();
  if (hosts.length === 0) throw new CfError('manifest lists no sites');
  for (const host of hosts) if (!/^[a-z0-9.-]+$/.test(host)) throw new CfError(`invalid host ${host}`);
  return hosts;
}

const normalizeEmail = (email) => {
  const value = String(email || '').trim().toLowerCase();
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value)) throw new CfError('X3_OWNER_EMAIL must be a valid email address');
  return value;
};

const policyBody = (email) => ({
  name: POLICY_NAME,
  decision: 'allow',
  include: [{email: {email}}],
  exclude: [],
  require: [],
  session_duration: SESSION,
});

const appBody = (hosts, policyId) => ({
  name: APP_NAME,
  type: 'self_hosted',
  domain: hosts[0],
  destinations: hosts.map((uri) => ({type: 'public', uri})),
  session_duration: SESSION,
  app_launcher_visible: false,
  policies: [{id: policyId, precedence: 1}],
});

const appHosts = (app) => new Set([
  ...(app.destinations || []).filter((d) => d?.type === 'public' || d?.uri).map((d) => String(d.uri).toLowerCase().replace(/\/.*$/, '')),
  ...(app.self_hosted_domains || []).map((d) => String(d).toLowerCase().replace(/\/.*$/, '')),
  ...(app.domain ? [String(app.domain).toLowerCase().replace(/\/.*$/, '')] : []),
]);

/**
 * A policy is owner-only when it is an allow rule whose single include is the owner's email.
 * Extra require/exclude rules only narrow who passes, so they are accepted.
 */
export function isOwnerOnly(policy, email) {
  return policy?.decision === 'allow'
    && Array.isArray(policy.include) && policy.include.length === 1
    && Object.keys(policy.include[0] || {}).length === 1
    && String(policy.include[0]?.email?.email || '').toLowerCase() === email;
}

async function teamDomainOf(client) {
  const org = (await client.call('GET', '/access/organizations')).result;
  const domain = String(org?.auth_domain || '').toLowerCase();
  if (!/^[a-z0-9-]+\.cloudflareaccess\.com$/.test(domain)) throw new CfError(`unexpected Access auth_domain: ${domain || '(none)'}`);
  return `https://${domain}`;
}

const findByName = async (client, route, name) => (await client.list(route)).filter((x) => x.name === name);

export async function ensure(client, {hosts, email}) {
  email = normalizeEmail(email);
  const teamDomain = await teamDomainOf(client);

  const policies = await findByName(client, '/access/policies', POLICY_NAME);
  if (policies.length > 1) throw new CfError(`more than one reusable policy named "${POLICY_NAME}"; remove the extras first`);
  const policy = policies[0]
    ? (await client.call('PUT', `/access/policies/${policies[0].id}`, policyBody(email))).result
    : (await client.call('POST', '/access/policies', policyBody(email))).result;

  const apps = await findByName(client, '/access/apps', APP_NAME);
  if (apps.length > 1) throw new CfError(`more than one Access app named "${APP_NAME}"; remove the extras first`);
  const app = apps[0]
    ? (await client.call('PUT', `/access/apps/${apps[0].id}`, appBody(hosts, policy.id))).result
    : (await client.call('POST', '/access/apps', appBody(hosts, policy.id))).result;

  return verify(client, {hosts, email, teamDomain, appId: app.id});
}

export async function verify(client, {hosts, email, teamDomain, appId}) {
  email = normalizeEmail(email);
  teamDomain ||= await teamDomainOf(client);
  let app;
  if (appId) app = (await client.call('GET', `/access/apps/${appId}`)).result;
  else {
    const apps = await findByName(client, '/access/apps', APP_NAME);
    if (apps.length !== 1) throw new CfError(`expected exactly one Access app named "${APP_NAME}", found ${apps.length}. Run: deploy.sh lock`);
    app = apps[0];
  }
  if (app.type !== 'self_hosted') throw new CfError(`Access app ${app.id} is not self_hosted`);
  const covered = appHosts(app);
  const uncovered = hosts.filter((h) => !covered.has(h));
  if (uncovered.length) throw new CfError(`Access app does not cover: ${uncovered.join(', ')}`);
  const attached = (app.policies || []).map((p) => (typeof p === 'string' ? p : p?.id)).filter(Boolean);
  if (attached.length === 0) throw new CfError('Access app has no policies (it would allow nobody, or everybody under a bypass)');
  for (const id of attached) {
    const policy = (await client.call('GET', `/access/apps/${app.id}/policies/${id}`)).result;
    if (!isOwnerOnly(policy, email)) throw new CfError(`policy "${policy?.name || id}" on the Access app is not owner-only for ${email}`);
  }
  if (!/^[0-9a-f]{32,128}$/i.test(app.aud || '')) throw new CfError('Access app has no AUD tag');
  return {teamDomain, aud: app.aud, appId: app.id};
}

export async function remove(client) {
  const apps = await findByName(client, '/access/apps', APP_NAME);
  for (const app of apps) await client.call('DELETE', `/access/apps/${app.id}`);
  return {removed: apps.map((a) => a.id)};
}

// realpath on both sides: systemd and deploy.sh start this through the release's `current` symlink.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  const command = process.argv[2];
  const here = path.dirname(fileURLToPath(import.meta.url));
  const manifestPath = process.env.X3_DEMOS_MANIFEST || path.join(here, 'sites.json');
  const run = async () => {
    const client = createClient({token: process.env.CF_API_TOKEN, accountId: process.env.CF_ACCOUNT_ID});
    const hosts = hostsFrom(manifestPath);
    if (command === 'ensure') return ensure(client, {hosts, email: process.env.X3_OWNER_EMAIL});
    if (command === 'verify') return verify(client, {hosts, email: process.env.X3_OWNER_EMAIL});
    if (command === 'remove') return remove(client);
    throw new CfError('usage: cf-access.mjs ensure|verify|remove');
  };
  run().then((result) => { process.stdout.write(`${JSON.stringify(result)}\n`); })
    .catch((error) => { console.error(`cf-access: ${error.message}`); process.exit(error instanceof CfError ? 3 : 1); });
}

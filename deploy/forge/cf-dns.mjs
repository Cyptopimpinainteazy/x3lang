#!/usr/bin/env node
/**
 * DNS for the Agent Forge hostname through the Cloudflare API. Zero dependencies (Node >= 20).
 *
 *   node cf-dns.mjs zone  HOST                  exit 0 if HOST sits in a Cloudflare zone of this account
 *   node cf-dns.mjs route HOST TUNNEL_ID         point HOST at the tunnel (proxied CNAME)
 *
 * `cloudflared tunnel route dns` only writes to the zone chosen at `cloudflared tunnel login`, so
 * a hostname on another zone (dotx3.net when the login was for x3star.net) needs the API.
 * Never overwrites a record that points somewhere other than a Cloudflare tunnel.
 *
 * Environment: CF_API_TOKEN (Zone: Read and DNS: Edit on the zone), CF_ACCOUNT_ID.
 * The token is sent only to api.cloudflare.com and never printed.
 */
import {realpathSync} from 'node:fs';
import {fileURLToPath} from 'node:url';

const API = 'https://api.cloudflare.com/client/v4';
export class DnsError extends Error {}

const HOST_RE = /^(?=.{4,253}$)([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/;
const TUNNEL_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function client({token, accountId, fetchImpl = globalThis.fetch}) {
  if (!token) throw new DnsError('CF_API_TOKEN is not set');
  if (!/^[0-9a-f]{32}$/i.test(accountId || '')) throw new DnsError('CF_ACCOUNT_ID must be the 32-character account id');
  async function call(method, route, body) {
    const res = await fetchImpl(`${API}${route}`, {
      method,
      headers: {Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', Accept: 'application/json'},
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    let json = null;
    try { json = await res.json(); } catch { /* reported below */ }
    if (!res.ok || !json?.success) {
      const detail = (json?.errors || []).map((e) => `${e.code}: ${e.message}`).join('; ') || `HTTP ${res.status}`;
      throw new DnsError(`${method} ${route.replace(/\?.*$/, '')} failed (${detail})`);
    }
    return json.result;
  }
  return {call, accountId};
}

/** The most specific zone of this account that contains host. */
export async function zoneFor(c, host) {
  host = String(host || '').toLowerCase();
  if (!HOST_RE.test(host)) throw new DnsError(`invalid hostname ${host}`);
  const labels = host.split('.');
  for (let i = 0; i < labels.length - 1; i += 1) {
    const name = labels.slice(i).join('.');
    const zones = await c.call('GET', `/zones?name=${encodeURIComponent(name)}&account.id=${c.accountId}&status=active`);
    if (Array.isArray(zones) && zones.length === 1) return {id: zones[0].id, name: zones[0].name};
  }
  throw new DnsError(`${host} is not in an active Cloudflare zone of account ${c.accountId}. Add the domain to Cloudflare (or use a hostname on a zone that is), then retry.`);
}

export async function route(c, host, tunnelId) {
  if (!TUNNEL_RE.test(tunnelId || '')) throw new DnsError('tunnel id must be a UUID');
  const zone = await zoneFor(c, host);
  const target = `${tunnelId}.cfargotunnel.com`;
  const records = await c.call('GET', `/zones/${zone.id}/dns_records?name=${encodeURIComponent(host)}`);
  const body = {type: 'CNAME', name: host, content: target, proxied: true, ttl: 1, comment: 'X3 Agent Forge tunnel (deploy/forge)'};
  if (records.length > 1) throw new DnsError(`${host} has ${records.length} DNS records; remove the extras by hand first`);
  const existing = records[0];
  if (existing) {
    const ours = existing.type === 'CNAME' && String(existing.content).endsWith('.cfargotunnel.com');
    if (!ours) throw new DnsError(`${host} already has a ${existing.type} record (${existing.content}) that is not a Cloudflare tunnel; refusing to overwrite it`);
    if (existing.content === target && existing.proxied) return {zone: zone.name, host, target, changed: false};
    await c.call('PUT', `/zones/${zone.id}/dns_records/${existing.id}`, body);
    return {zone: zone.name, host, target, changed: true, replaced: existing.content};
  }
  await c.call('POST', `/zones/${zone.id}/dns_records`, body);
  return {zone: zone.name, host, target, changed: true};
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  const [command, host, tunnelId] = process.argv.slice(2);
  const run = async () => {
    const c = client({token: process.env.CF_API_TOKEN, accountId: process.env.CF_ACCOUNT_ID});
    if (command === 'zone') return zoneFor(c, host);
    if (command === 'route') return route(c, host, tunnelId);
    throw new DnsError('usage: cf-dns.mjs zone HOST | route HOST TUNNEL_ID');
  };
  run().then((r) => process.stdout.write(`${JSON.stringify(r)}\n`))
    .catch((e) => { console.error(`cf-dns: ${e.message}`); process.exit(e instanceof DnsError ? 3 : 1); });
}

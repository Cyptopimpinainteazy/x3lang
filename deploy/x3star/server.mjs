#!/usr/bin/env node
/**
 * x3star.net host for the X3Lang judge demos, run behind a Cloudflare Tunnel.
 * Zero dependencies (Node >= 20).
 *
 * - Routes by Host header to the sites in sites.json. Each site serves its mount folder at "/";
 *   /page serves page.html, and a listed notFound page answers unknown paths with HTTP 404.
 * - Serves only the exact files listed in sites.json, from inside X3_DEMOS_ROOT.
 *   Dot-segments, traversal, symlink escapes and unlisted files all return 404.
 * - X3_ACCESS_MODE=access (the default) requires a valid Cloudflare Access JWT on every
 *   request: RS256, signed by the team's published keys, matching issuer and AUD, unexpired.
 *   Missing configuration stops the server from starting. X3_ACCESS_MODE=public must be set
 *   explicitly to serve without login.
 * - Each HTML page gets a Content-Security-Policy that allows only its own inline scripts,
 *   pinned by SHA-256, and only the outside origins listed in sites.json.
 */
import http from 'node:http';
import crypto from 'node:crypto';
import {readFileSync, realpathSync, statSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.md': 'text/markdown; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.x3': 'text/plain; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};
const CLOCK_SKEW_SECONDS = 30;
const JWKS_TTL_MS = 5 * 60 * 1000;
const JWKS_MIN_REFRESH_MS = 30 * 1000;

const plain = (res, status, text, extra = {}) => {
  res.writeHead(status, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
    ...extra,
  });
  res.end(`${text}\n`);
};

/* ── Configuration ───────────────────────────────────────────────────────────── */

export function loadManifest(manifestPath) {
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  if (!manifest || typeof manifest.sites !== 'object' || !Array.isArray(manifest.files)) {
    throw new Error(`${manifestPath}: needs "sites" and "files"`);
  }
  const files = new Set();
  for (const file of manifest.files) {
    if (typeof file !== 'string' || file.startsWith('/') || file.split('/').some((s) => s === '' || s.startsWith('.'))) {
      throw new Error(`invalid file entry in manifest: ${JSON.stringify(file)}`);
    }
    if (!MIME[path.posix.extname(file)]) throw new Error(`no content type for ${file}`);
    files.add(file);
  }
  const cleanPath = (value, what) => {
    if (typeof value !== 'string' || value.startsWith('/') || value.endsWith('/')
        || (value !== '' && value.split('/').some((s) => s === '' || s.startsWith('.')))) {
      throw new Error(`invalid ${what}: ${JSON.stringify(value)}`);
    }
    return value;
  };
  const sites = {};
  for (const [host, site] of Object.entries(manifest.sites)) {
    if (!/^[a-z0-9.-]+$/.test(host)) throw new Error(`invalid host: ${host}`);
    if (!site || typeof site !== 'object') throw new Error(`${host}: site must be an object`);
    const mount = cleanPath(site.mount ?? '', `${host} mount`);
    const join = (rel) => (mount ? `${mount}/${rel}` : rel);
    const entry = cleanPath(site.entry, `${host} entry`);
    if (!files.has(join(entry))) throw new Error(`${host}: entry ${join(entry)} is not in "files"`);
    let notFound = null;
    if (site.notFound !== undefined) {
      notFound = join(cleanPath(site.notFound, `${host} notFound`));
      if (!files.has(notFound) || !notFound.endsWith('.html')) throw new Error(`${host}: notFound ${notFound} must be a listed .html file`);
    }
    sites[host] = {title: String(site.title || host), mount, entry, notFound};
  }
  if (Object.keys(sites).length === 0) throw new Error('manifest has no sites');
  const csp = manifest.csp || {};
  const origins = (list) => (Array.isArray(list) ? list : []).map((o) => {
    const url = new URL(o);
    if (url.protocol !== 'https:' || url.pathname !== '/' || url.search || url.hash) throw new Error(`CSP origin must be a bare https origin: ${o}`);
    return url.origin;
  });
  return {sites, files, csp: {connect: origins(csp.connect), style: origins(csp.style), font: origins(csp.font)}};
}

export function loadConfig(env, here = path.dirname(fileURLToPath(import.meta.url))) {
  const root = path.resolve(env.X3_DEMOS_ROOT || path.join(here, '..', '..'));
  const manifestPath = env.X3_DEMOS_MANIFEST || path.join(here, 'sites.json');
  const port = Number(env.X3_DEMOS_PORT || 7410);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error(`invalid X3_DEMOS_PORT: ${env.X3_DEMOS_PORT}`);
  const bind = env.X3_DEMOS_BIND || '127.0.0.1';
  const mode = env.X3_ACCESS_MODE || 'access';
  if (mode !== 'access' && mode !== 'public') throw new Error(`X3_ACCESS_MODE must be "access" or "public", got ${JSON.stringify(mode)}`);
  let ownerEmail = null;
  let teamDomain = null;
  let aud = null;
  if (mode === 'access') {
    ownerEmail = normalizeOwnerEmail(env.X3_OWNER_EMAIL);
    teamDomain = normalizeTeamDomain(env.X3_ACCESS_TEAM_DOMAIN);
    aud = (env.X3_ACCESS_AUD || '').trim();
    if (!/^[0-9a-f]{32,128}$/i.test(aud)) throw new Error('X3_ACCESS_AUD must be the Access application AUD tag (hex). Run deploy.sh lock, or set X3_ACCESS_MODE=public deliberately.');
  }
  return {root, manifest: loadManifest(manifestPath), port, bind, mode, teamDomain, aud, ownerEmail};
}

export function normalizeTeamDomain(value) {
  const raw = (value || '').trim();
  if (!raw) throw new Error('X3_ACCESS_TEAM_DOMAIN is required in access mode (https://<team>.cloudflareaccess.com)');
  const url = new URL(raw.includes('://') ? raw : `https://${raw}`);
  if (url.protocol !== 'https:' || !/^[a-z0-9-]+\.cloudflareaccess\.com$/i.test(url.hostname) || url.pathname !== '/') {
    throw new Error(`X3_ACCESS_TEAM_DOMAIN must be https://<team>.cloudflareaccess.com, got ${raw}`);
  }
  return url.origin;
}

/* ── Cloudflare Access JWT verification ──────────────────────────────────────── */

const b64urlJson = (part) => JSON.parse(Buffer.from(part, 'base64url').toString('utf8'));

function normalizeOwnerEmail(value) {
  const email = typeof value === 'string' ? value.trim().toLowerCase() : '';
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) throw new Error('X3_OWNER_EMAIL is required in Access mode');
  return email;
}

export function createAccessVerifier({teamDomain, aud, ownerEmail, fetchImpl = globalThis.fetch, now = () => Date.now()}) {
  const owner = normalizeOwnerEmail(ownerEmail);
  const certsUrl = `${teamDomain}/cdn-cgi/access/certs`;
  let keys = new Map();
  let fetchedAt = 0;
  let inflight = null;

  async function refresh() {
    if (inflight) return inflight;
    inflight = (async () => {
      const response = await fetchImpl(certsUrl, {headers: {accept: 'application/json'}});
      if (!response.ok) throw new Error(`certs fetch failed: HTTP ${response.status}`);
      const body = await response.json();
      const next = new Map();
      for (const jwk of Array.isArray(body?.keys) ? body.keys : []) {
        if (jwk?.kty !== 'RSA' || typeof jwk.kid !== 'string') continue;
        next.set(jwk.kid, crypto.createPublicKey({key: {kty: 'RSA', n: jwk.n, e: jwk.e}, format: 'jwk'}));
      }
      if (next.size === 0) throw new Error('certs response has no RSA keys');
      keys = next;
      fetchedAt = now();
    })();
    try { await inflight; } finally { inflight = null; }
  }

  async function keyFor(kid) {
    const stale = now() - fetchedAt > JWKS_TTL_MS;
    if (stale || (!keys.has(kid) && now() - fetchedAt > JWKS_MIN_REFRESH_MS)) await refresh();
    return keys.get(kid) || null;
  }

  return async function verify(token) {
    if (typeof token !== 'string' || token.length > 16384) return {ok: false, reason: 'missing token'};
    const parts = token.split('.');
    if (parts.length !== 3 || parts.some((p) => !/^[A-Za-z0-9_-]+$/.test(p))) return {ok: false, reason: 'malformed token'};
    let header;
    let payload;
    try { header = b64urlJson(parts[0]); payload = b64urlJson(parts[1]); }
    catch { return {ok: false, reason: 'malformed token'}; }
    if (header?.alg !== 'RS256') return {ok: false, reason: 'unsupported algorithm'};
    if (typeof header.kid !== 'string') return {ok: false, reason: 'missing key id'};
    let key;
    try { key = await keyFor(header.kid); }
    catch (error) { return {ok: false, reason: `cannot load signing keys: ${error.message}`}; }
    if (!key) return {ok: false, reason: 'unknown key id'};
    const signed = Buffer.from(`${parts[0]}.${parts[1]}`);
    if (!crypto.verify('RSA-SHA256', signed, key, Buffer.from(parts[2], 'base64url'))) return {ok: false, reason: 'bad signature'};
    const t = Math.floor(now() / 1000);
    if (payload?.iss !== teamDomain) return {ok: false, reason: 'wrong issuer'};
    const audiences = Array.isArray(payload.aud) ? payload.aud : [payload.aud];
    if (!audiences.includes(aud)) return {ok: false, reason: 'wrong audience'};
    if (typeof payload.exp !== 'number' || payload.exp + CLOCK_SKEW_SECONDS <= t) return {ok: false, reason: 'expired'};
    if (typeof payload.nbf === 'number' && payload.nbf - CLOCK_SKEW_SECONDS > t) return {ok: false, reason: 'not yet valid'};
    if (typeof payload.email !== 'string' || payload.email.toLowerCase() !== owner) {
      return {ok: false, reason: 'wrong owner'};
    }
    return {ok: true, email: owner};
  };
}

/* ── Static files ────────────────────────────────────────────────────────────── */

function inlineScriptHashes(html) {
  const hashes = [];
  const re = /<script\b([^>]*)>([\s\S]*?)<\/script>/gi;
  for (const match of html.matchAll(re)) {
    if (/\bsrc\s*=/i.test(match[1])) continue;
    hashes.push(`'sha256-${crypto.createHash('sha256').update(match[2], 'utf8').digest('base64')}'`);
  }
  return hashes;
}

export function buildCsp(manifest, scriptHashes) {
  return [
    "default-src 'none'",
    `script-src 'self'${scriptHashes.length ? ` ${scriptHashes.join(' ')}` : ''}`,
    `style-src 'self' 'unsafe-inline'${manifest.csp.style.map((o) => ` ${o}`).join('')}`,
    `font-src 'self'${manifest.csp.font.map((o) => ` ${o}`).join('')}`,
    "img-src 'self' data:",
    `connect-src 'self'${manifest.csp.connect.map((o) => ` ${o}`).join('')}`,
    "base-uri 'none'",
    "form-action 'none'",
    "frame-ancestors 'none'",
  ].join('; ');
}

/** Load every listed file once, refusing to start if any is missing or resolves outside root. */
export function loadFiles(root, manifest) {
  const realRoot = realpathSync(root);
  const table = new Map();
  const missing = [];
  for (const rel of manifest.files) {
    const abs = path.join(realRoot, ...rel.split('/'));
    let real;
    try { real = realpathSync(abs); } catch { missing.push(rel); continue; }
    if (real !== abs || !statSync(real).isFile()) throw new Error(`${rel} is not a regular file inside ${realRoot}`);
    const body = readFileSync(real);
    const ext = path.posix.extname(rel);
    const headers = {
      'Content-Type': MIME[ext],
      'Cache-Control': 'no-cache',
      ETag: `"${crypto.createHash('sha256').update(body).digest('base64url').slice(0, 27)}"`,
    };
    if (ext === '.html') headers['Content-Security-Policy'] = buildCsp(manifest, inlineScriptHashes(body.toString('utf8')));
    table.set(rel, {body, headers});
  }
  if (missing.length) throw new Error(`missing files listed in sites.json: ${missing.join(', ')}`);
  return table;
}

const SECURITY_HEADERS = {
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'no-referrer',
  'X-Frame-Options': 'DENY',
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Resource-Policy': 'same-origin',
  'Permissions-Policy': 'camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=()',
  'Strict-Transport-Security': 'max-age=31536000',
};

/* ── Request handling ────────────────────────────────────────────────────────── */

export function createHandler({config, files, verifier, log = () => {}}) {
  const {manifest, mode} = config;
  return async function handle(req, res) {
    const started = Date.now();
    const host = String(req.headers.host || '').toLowerCase().replace(/:\d+$/, '');
    const done = (status) => log({status, host, path: req.url, ms: Date.now() - started});
    try {
      if (req.method !== 'GET' && req.method !== 'HEAD') { plain(res, 405, 'Method not allowed', {Allow: 'GET, HEAD'}); return done(405); }
      const rawPath = String(req.url || '/').split('?')[0];
      if (rawPath === '/healthz') { plain(res, 200, 'ok'); return done(200); }
      const site = manifest.sites[host];
      if (!site) { plain(res, 404, 'Unknown site'); return done(404); }

      if (mode === 'access') {
        const result = await verifier(req.headers['cf-access-jwt-assertion']);
        if (!result.ok) { plain(res, 403, `Forbidden: Cloudflare Access login required (${result.reason})`); return done(403); }
      }

      let decoded;
      try { decoded = decodeURIComponent(rawPath); } catch { plain(res, 400, 'Bad request'); return done(400); }
      if (decoded.includes('\0') || decoded.includes('\\')) { plain(res, 400, 'Bad request'); return done(400); }
      const inMount = (rel) => (site.mount ? `${site.mount}/${rel}` : rel);

      let file = null;
      if (decoded === '/') {
        // An entry at the top of the mount is the home page; a deeper one is reached by redirect,
        // so its relative links (../../evidence/...) resolve exactly as in the repository.
        if (site.entry.includes('/')) {
          res.writeHead(302, {Location: `/${site.entry}`, 'Cache-Control': 'no-store', ...SECURITY_HEADERS});
          res.end();
          return done(302);
        }
        file = files.get(inMount(site.entry));
      } else {
        const rel = decoded.slice(1);
        if (!rel.split('/').some((s) => s === '' || s.startsWith('.'))) {
          // Exact listed file, else the extensionless form of a listed page (/judge -> judge.html).
          file = files.get(inMount(rel)) || (path.posix.extname(rel) === '' ? files.get(inMount(`${rel}.html`)) : null) || null;
        }
      }
      if (!file) {
        const page = site.notFound && files.get(site.notFound);
        if (page) {
          res.writeHead(404, {...SECURITY_HEADERS, ...page.headers, 'Cache-Control': 'no-store', 'Content-Length': page.body.length});
          res.end(req.method === 'HEAD' ? undefined : page.body);
        } else plain(res, 404, 'Not found');
        return done(404);
      }

      const headers = {...SECURITY_HEADERS, ...file.headers};
      if (req.headers['if-none-match'] === file.headers.ETag) { res.writeHead(304, headers); res.end(); return done(304); }
      res.writeHead(200, {...headers, 'Content-Length': file.body.length});
      res.end(req.method === 'HEAD' ? undefined : file.body);
      return done(200);
    } catch (error) {
      if (!res.headersSent) plain(res, 500, 'Internal error');
      else res.destroy();
      log({status: 500, host, path: req.url, error: error.message});
      return undefined;
    }
  };
}

export function startServer(config, {verifier, log} = {}) {
  const files = loadFiles(config.root, config.manifest);
  const verify = config.mode === 'access'
    ? (verifier || createAccessVerifier({teamDomain: config.teamDomain, aud: config.aud, ownerEmail: config.ownerEmail}))
    : null;
  const server = http.createServer(createHandler({config, files, verifier: verify, log}));
  server.headersTimeout = 10_000;
  server.requestTimeout = 15_000;
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(config.port, config.bind, () => resolve({server, files}));
  });
}

// realpath on both sides: systemd and deploy.sh start this through the release's `current` symlink.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  let config;
  try { config = loadConfig(process.env); }
  catch (error) { console.error(`x3lang-demos: refusing to start: ${error.message}`); process.exit(2); }
  const log = (entry) => console.log(JSON.stringify({t: new Date().toISOString(), ...entry}));
  startServer(config, {log}).then(({server, files}) => {
    const {address, port} = server.address();
    console.log(`x3lang-demos: ${files.size} files, mode=${config.mode}, http://${address}:${port}, sites: ${Object.keys(config.manifest.sites).join(', ')}`);
    const stop = () => server.close(() => process.exit(0));
    process.on('SIGTERM', stop);
    process.on('SIGINT', stop);
  }).catch((error) => { console.error(`x3lang-demos: refusing to start: ${error.message}`); process.exit(2); });
}

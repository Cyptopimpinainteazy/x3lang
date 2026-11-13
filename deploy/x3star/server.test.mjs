import assert from 'node:assert/strict';
import {after, before, describe, test} from 'node:test';
import crypto from 'node:crypto';
import http from 'node:http';
import {mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {
  buildCsp, createAccessVerifier, loadConfig, loadFiles, loadManifest, normalizeTeamDomain, startServer,
} from './server.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const TEAM = 'https://x3team.cloudflareaccess.com';
const AUD = 'a'.repeat(64);

/* ── JWT helpers (test-only signer) ── */
const {privateKey, publicKey} = crypto.generateKeyPairSync('rsa', {modulusLength: 2048});
const {privateKey: otherKey} = crypto.generateKeyPairSync('rsa', {modulusLength: 2048});
const jwk = {...publicKey.export({format: 'jwk'}), kid: 'k1', alg: 'RS256', use: 'sig'};
const b64 = (o) => Buffer.from(JSON.stringify(o)).toString('base64url');
function sign(payload, {kid = 'k1', alg = 'RS256', key = privateKey} = {}) {
  const head = b64({alg, kid, typ: 'JWT'});
  const body = b64(payload);
  const sig = crypto.sign('RSA-SHA256', Buffer.from(`${head}.${body}`), key).toString('base64url');
  return `${head}.${body}.${sig}`;
}
const NOW = 1_800_000_000_000;
const good = (extra = {}) => ({iss: TEAM, aud: [AUD], email: 'owner@example.com', exp: NOW / 1000 + 600, iat: NOW / 1000, ...extra});
function mockCerts(keys = [jwk]) {
  const calls = {n: 0};
  const fetchImpl = async (url) => {
    calls.n += 1;
    assert.equal(url, `${TEAM}/cdn-cgi/access/certs`);
    return {ok: true, status: 200, json: async () => ({keys})};
  };
  return {fetchImpl, calls};
}

describe('Cloudflare Access JWT verification', () => {
  const verifier = () => createAccessVerifier({teamDomain: TEAM, aud: AUD, ownerEmail: 'owner@example.com', fetchImpl: mockCerts().fetchImpl, now: () => NOW});

  test('accepts a correctly signed, unexpired token for this team and AUD', async () => {
    assert.deepEqual(await verifier()(sign(good())), {ok: true, email: 'owner@example.com'});
  });
  test('accepts aud given as a string', async () => {
    assert.equal((await verifier()(sign(good({aud: AUD})))).ok, true);
  });
  const refusals = {
    'missing token': [undefined],
    'valid token for another user': [sign(good({email: 'other@example.com'})), 'wrong owner'],
    'valid token without an owner claim': [sign(good({email: undefined})), 'wrong owner'],
    'malformed token': ['abc.def'],
    'unsupported algorithm': [sign(good(), {alg: 'none'})],
    'unknown key id': [sign(good(), {kid: 'nope'})],
    'bad signature': [sign(good(), {key: otherKey})],
    'wrong issuer': [sign(good({iss: 'https://evil.cloudflareaccess.com'}))],
    'wrong audience': [sign(good({aud: ['b'.repeat(64)]}))],
    expired: [sign(good({exp: NOW / 1000 - 31}))],
    'not yet valid': [sign(good({nbf: NOW / 1000 + 120}))],
  };
  for (const [reason, [token, expected = reason]] of Object.entries(refusals)) {
    test(`refuses: ${reason}`, async () => {
      const result = await verifier()(token);
      assert.equal(result.ok, false);
      assert.equal(result.reason, expected);
    });
  }
  test('refuses a token whose payload was altered after signing', async () => {
    const [h, , s] = sign(good()).split('.');
    const forged = `${h}.${b64(good({email: 'attacker@example.com'}))}.${s}`;
    assert.equal((await verifier()(forged)).reason, 'bad signature');
  });
  test('refuses an HS256 token signed with the public key as an HMAC secret', async () => {
    const head = b64({alg: 'HS256', kid: 'k1'});
    const body = b64(good());
    const sig = crypto.createHmac('sha256', publicKey.export({type: 'spki', format: 'pem'})).update(`${head}.${body}`).digest('base64url');
    assert.equal((await verifier()(`${head}.${body}.${sig}`)).reason, 'unsupported algorithm');
  });
  test('fails closed when the certs endpoint is down', async () => {
    const v = createAccessVerifier({teamDomain: TEAM, aud: AUD, ownerEmail: 'owner@example.com', now: () => NOW, fetchImpl: async () => ({ok: false, status: 503, json: async () => ({})})});
    const result = await v(sign(good()));
    assert.equal(result.ok, false);
    assert.match(result.reason, /cannot load signing keys/);
  });
  test('caches keys and refetches at most every 30s for unknown key ids', async () => {
    let t = NOW;
    const {fetchImpl, calls} = mockCerts();
    const v = createAccessVerifier({teamDomain: TEAM, aud: AUD, ownerEmail: 'owner@example.com', fetchImpl, now: () => t});
    await v(sign(good()));
    await v(sign(good()));
    assert.equal(calls.n, 1);
    await v(sign(good(), {kid: 'rotated'}));
    assert.equal(calls.n, 1, 'no refetch within 30s');
    t += 31_000;
    await v(sign(good(), {kid: 'rotated'}));
    assert.equal(calls.n, 2);
  });
});

describe('configuration fails closed', () => {
  const manifest = path.join(here, 'sites.json');
  test('access mode is the default and requires team domain and AUD', () => {
    assert.throws(() => loadConfig({X3_DEMOS_MANIFEST: manifest}), /X3_OWNER_EMAIL is required/);
    assert.throws(() => loadConfig({X3_DEMOS_MANIFEST: manifest, X3_OWNER_EMAIL: 'owner@example.com', X3_ACCESS_TEAM_DOMAIN: TEAM}), /X3_ACCESS_AUD/);
    const ok = loadConfig({X3_DEMOS_MANIFEST: manifest, X3_OWNER_EMAIL: 'owner@example.com', X3_ACCESS_TEAM_DOMAIN: TEAM, X3_ACCESS_AUD: AUD});
    assert.equal(ok.mode, 'access');
    assert.equal(ok.bind, '127.0.0.1');
  });
  test('public mode must be spelled out and nothing else is accepted', () => {
    assert.equal(loadConfig({X3_DEMOS_MANIFEST: manifest, X3_ACCESS_MODE: 'public'}).mode, 'public');
    assert.throws(() => loadConfig({X3_DEMOS_MANIFEST: manifest, X3_ACCESS_MODE: 'off'}), /must be "access" or "public"/);
  });
  test('team domain must be a cloudflareaccess.com origin', () => {
    assert.equal(normalizeTeamDomain('x3team.cloudflareaccess.com'), TEAM);
    for (const bad of ['https://evil.com', 'http://x3team.cloudflareaccess.com', 'https://x3team.cloudflareaccess.com/path']) {
      assert.throws(() => normalizeTeamDomain(bad), /cloudflareaccess/);
    }
  });
  test('manifest rejects traversal, dotfiles and entries outside the file list', () => {
    const dir = mkdtempSync(path.join(tmpdir(), 'x3m-'));
    const write = (m) => { const p = path.join(dir, 'm.json'); writeFileSync(p, JSON.stringify(m)); return p; };
    assert.throws(() => loadManifest(write({sites: {'a.x3star.net': {entry: 'a.html'}}, files: ['../a.html']})), /invalid file entry/);
    assert.throws(() => loadManifest(write({sites: {'a.x3star.net': {entry: 'a.html'}}, files: ['.env']})), /invalid file entry/);
    assert.throws(() => loadManifest(write({sites: {'a.x3star.net': {entry: 'b.html'}}, files: ['a.html']})), /not in "files"/);
    assert.throws(() => loadManifest(write({sites: {'a.x3star.net': {entry: 'a.html'}}, files: ['a.html'], csp: {connect: ['http://x.com']}})), /bare https origin/);
    assert.throws(() => loadManifest(write({sites: {'a.x3star.net': {mount: '../x', entry: 'a.html'}}, files: ['a.html']})), /invalid a\.x3star\.net mount/);
    assert.throws(() => loadManifest(write({sites: {'a.x3star.net': {mount: 'd', entry: 'a.html', notFound: 'nf.html'}}, files: ['d/a.html']})), /notFound/);
    assert.equal(loadManifest(write({sites: {'a.x3star.net': {mount: 'd', entry: 'a.html'}}, files: ['d/a.html']})).sites['a.x3star.net'].mount, 'd');
    rmSync(dir, {recursive: true});
  });
  test('manifest media and image origins must be bare https origins', () => {
    const dir = mkdtempSync(path.join(tmpdir(), 'x3m-'));
    const write = (m) => { const p = path.join(dir, 'm.json'); writeFileSync(p, JSON.stringify(m)); return p; };
    const base = {sites: {'a.x3star.net': {entry: 'a.html'}}, files: ['a.html']};
    assert.throws(() => loadManifest(write({...base, csp: {media: ['http://x.com']}})), /bare https origin/);
    assert.throws(() => loadManifest(write({...base, csp: {media: ['https://x.com/video.mp4']}})), /bare https origin/);
    assert.throws(() => loadManifest(write({...base, csp: {img: ['https://x.com/?q=1']}})), /bare https origin/);
    const m = loadManifest(write({...base, csp: {media: ['https://x.com'], img: ['https://x.com']}}));
    assert.deepEqual(m.csp.media, ['https://x.com']);
    assert.deepEqual(m.csp.img, ['https://x.com']);
    assert.deepEqual(loadManifest(write(base)).csp.media, []);
    rmSync(dir, {recursive: true});
  });
  test('the real manifest allows judge media only from dotx3.net', () => {
    const m = loadManifest(manifest);
    assert.deepEqual(m.csp.media, ['https://dotx3.net']);
    assert.deepEqual(m.csp.img, ['https://dotx3.net']);
  });
  test('the real manifest: every site entry exists and judge/evidence.json is never listed', () => {
    const m = loadManifest(manifest);
    assert.ok(!m.files.has('x3-lang/near/demo/judge/evidence.json'));
    assert.deepEqual(Object.keys(m.sites).sort(), ['controlroom.x3star.net', 'guard.x3star.net', 'judge.x3star.net', 'x3lang.x3star.net']);
    const files = loadFiles(path.join(here, '..', '..'), m);
    assert.equal(files.size, m.files.size);
  });
});

describe('HTTP behavior', () => {
  let root;
  let outside;
  let base;
  let server;
  const manifestFor = (r) => {
    const p = path.join(r, 'sites.json');
    writeFileSync(p, JSON.stringify({
      sites: {
        'a.x3star.net': {title: 'A', entry: 'site/index.html'},
        'b.x3star.net': {title: 'B', entry: 'site/b.html'},
        'c.x3star.net': {title: 'C', mount: 'site', entry: 'index.html', notFound: '404.html'},
      },
      files: ['site/index.html', 'site/b.html', 'site/app.mjs', 'site/data.json', 'site/link.html', 'site/404.html', 'site/notes.md', 'top.json'],
      csp: {connect: ['https://api.example.com']},
    }));
    return p;
  };
  // fetch() drops a custom Host header, so use http.request; returns a fetch-like response.
  const get = (host, p, headers = {}, method = 'GET') => new Promise((resolve, reject) => {
    // Raw path on purpose: a URL string would be normalized client-side (/./, /../), hiding what an attacker can send.
    const {port} = new URL(base);
    const req = http.request({hostname: '127.0.0.1', port, path: p, method, headers: {...headers, host}}, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const body = Buffer.concat(chunks).toString('utf8');
        resolve({
          status: res.statusCode,
          headers: {get: (name) => res.headers[name.toLowerCase()] ?? null},
          text: async () => body,
          json: async () => JSON.parse(body),
        });
      });
    });
    req.on('error', reject);
    req.end();
  });

  before(async () => {
    root = mkdtempSync(path.join(tmpdir(), 'x3root-'));
    outside = mkdtempSync(path.join(tmpdir(), 'x3out-'));
    mkdirSync(path.join(root, 'site'));
    writeFileSync(path.join(root, 'site/index.html'), '<!doctype html><script type="module">console.log(1)</script><script src="app.mjs"></script>');
    writeFileSync(path.join(root, 'site/b.html'), '<!doctype html><p>b</p>');
    writeFileSync(path.join(root, 'site/app.mjs'), 'export const x = 1;');
    writeFileSync(path.join(root, 'site/data.json'), '{"ok":true}');
    writeFileSync(path.join(root, 'site/404.html'), '<!doctype html><h1>Page missing</h1>');
    writeFileSync(path.join(root, 'site/notes.md'), '# notes');
    writeFileSync(path.join(root, 'top.json'), '{"top":true}');
    writeFileSync(path.join(root, 'site/evidence.json'), '{"secret":"local paths"}');
    writeFileSync(path.join(outside, 'secret.html'), 'outside');
    writeFileSync(path.join(root, '.env'), 'TOKEN=nope');
    const config = {root, manifest: loadManifest(manifestFor(root)), port: 0, bind: '127.0.0.1', mode: 'access', teamDomain: TEAM, aud: AUD, ownerEmail: 'owner@example.com'};
    // a listed name that is a symlink to a file outside the root must refuse to load
    symlinkSync(path.join(outside, 'secret.html'), path.join(root, 'site/link.html'));
    assert.throws(() => loadFiles(root, config.manifest), /not a regular file inside/);
    rmSync(path.join(root, 'site/link.html'));
    writeFileSync(path.join(root, 'site/link.html'), 'inside');
    const verifier = createAccessVerifier({teamDomain: TEAM, aud: AUD, ownerEmail: 'owner@example.com', fetchImpl: mockCerts().fetchImpl, now: () => Date.now()});
    ({server} = await startServer(config, {verifier}));
    base = `http://127.0.0.1:${server.address().port}`;
  });
  after(() => { server?.close(); rmSync(root, {recursive: true, force: true}); rmSync(outside, {recursive: true, force: true}); });

  const token = () => sign({iss: TEAM, aud: [AUD], email: 'owner@example.com', exp: Math.floor(Date.now() / 1000) + 600});
  const auth = () => ({'cf-access-jwt-assertion': token()});

  test('healthz answers without login', async () => {
    const r = await get('anything', '/healthz');
    assert.equal(r.status, 200);
  });
  test('every page refuses requests without a valid Access JWT', async () => {
    for (const p of ['/', '/site/index.html', '/site/data.json']) {
      assert.equal((await get('a.x3star.net', p)).status, 403, p);
      assert.equal((await get('a.x3star.net', p, {'cf-access-jwt-assertion': 'x.y.z'})).status, 403, p);
    }
  });
  test('root redirects to the site entry for each host', async () => {
    const a = await get('a.x3star.net', '/', auth());
    assert.equal(a.status, 302);
    assert.equal(a.headers.get('location'), '/site/index.html');
    assert.equal((await get('b.x3star.net:443', '/', auth())).headers.get('location'), '/site/b.html');
  });
  test('unknown hosts are refused before any auth or file lookup', async () => {
    assert.equal((await get('evil.example.com', '/site/index.html', auth())).status, 404);
  });
  test('listed files are served with content type, CSP pinned to inline script hashes, and security headers', async () => {
    const r = await get('a.x3star.net', '/site/index.html', auth());
    assert.equal(r.status, 200);
    assert.equal(r.headers.get('content-type'), 'text/html; charset=utf-8');
    const hash = crypto.createHash('sha256').update('console.log(1)').digest('base64');
    const csp = r.headers.get('content-security-policy');
    assert.ok(csp.includes(`script-src 'self' 'sha256-${hash}'`), csp);
    assert.ok(!/script-src[^;]*unsafe-inline/.test(csp), 'no unsafe-inline scripts');
    assert.ok(csp.includes('connect-src \'self\' https://api.example.com'));
    assert.ok(csp.includes("frame-ancestors 'none'"));
    assert.equal(r.headers.get('x-content-type-options'), 'nosniff');
    assert.equal(r.headers.get('x-frame-options'), 'DENY');
    assert.equal(r.headers.get('referrer-policy'), 'no-referrer');
    assert.equal((await get('a.x3star.net', '/site/app.mjs', auth())).headers.get('content-type'), 'text/javascript; charset=utf-8');
  });
  test('ETag revalidation returns 304 and HEAD returns no body', async () => {
    const first = await get('a.x3star.net', '/site/data.json', auth());
    const etag = first.headers.get('etag');
    assert.ok(etag);
    assert.equal((await get('a.x3star.net', '/site/data.json', {...auth(), 'if-none-match': etag})).status, 304);
    const head = await get('a.x3star.net', '/site/data.json', auth(), 'HEAD');
    assert.equal(head.status, 200);
    assert.equal(await head.text(), '');
  });
  test('unlisted files, dotfiles, traversal and encoded traversal all 404', async () => {
    for (const p of ['/site/evidence.json', '/.env', '/site/../.env', '/site/%2e%2e/.env', '/%2e%2e/%2e%2e/etc/passwd', '/site//index.html', '/site/./index.html', '/sites.json']) {
      const r = await get('a.x3star.net', p, auth());
      assert.equal(r.status, 404, p);
    }
  });
  test('malformed encodings, NUL and backslashes are rejected', async () => {
    assert.equal((await get('a.x3star.net', '/site/%E0%A4%A', auth())).status, 400);
    assert.equal((await get('a.x3star.net', '/site/index.html%00', auth())).status, 400);
    assert.equal((await get('a.x3star.net', '/site%5c..%5c.env', auth())).status, 400);
  });
  test('a mounted site serves its folder at /, with clean URLs and its own 404 page', async () => {
    const home = await get('c.x3star.net', '/', auth());
    assert.equal(home.status, 200);
    assert.match(await home.text(), /console\.log\(1\)/);
    assert.equal((await get('c.x3star.net', '/b', auth())).status, 200, '/b serves b.html');
    assert.equal((await get('c.x3star.net', '/b.html', auth())).status, 200);
    const md = await get('c.x3star.net', '/notes.md', auth());
    assert.equal(md.headers.get('content-type'), 'text/markdown; charset=utf-8');
    const missing = await get('c.x3star.net', '/nope', auth());
    assert.equal(missing.status, 404);
    assert.match(await missing.text(), /Page missing/);
    assert.ok(missing.headers.get('content-security-policy'), 'the 404 page carries the CSP too');
  });
  test('a mount confines the site: listed files outside it are unreachable', async () => {
    for (const p of ['/top.json', '/../top.json', '/%2e%2e/top.json', '/site/index.html']) {
      assert.equal((await get('c.x3star.net', p, auth())).status, 404, p);
    }
    assert.equal((await get('a.x3star.net', '/top.json', auth())).status, 200, 'still served on the unmounted site');
  });
  test('without a login, a mounted site still returns 403, never its 404 page', async () => {
    assert.equal((await get('c.x3star.net', '/nope')).status, 403);
  });
  test('only GET and HEAD are allowed', async () => {
    const r = await get('a.x3star.net', '/site/index.html', auth(), 'POST');
    assert.equal(r.status, 405);
    assert.equal(r.headers.get('allow'), 'GET, HEAD');
  });
  test('files are snapshotted at start: changes on disk are not served until restart', async () => {
    writeFileSync(path.join(root, 'site/data.json'), '{"ok":false}');
    assert.deepEqual(await (await get('a.x3star.net', '/site/data.json', auth())).json(), {ok: true});
  });
});

test('buildCsp lists only declared origins', () => {
  const csp = buildCsp({csp: {connect: ['https://a.example'], style: [], font: []}}, []);
  assert.ok(csp.startsWith("default-src 'none'"));
  assert.ok(csp.includes("script-src 'self';"));
  assert.ok(csp.includes("connect-src 'self' https://a.example"));
  assert.ok(csp.includes("media-src 'self';"), 'media defaults to same origin only');
  assert.ok(csp.includes("img-src 'self' data:;"), 'images default to same origin and data: only');
});

test('buildCsp adds declared media and image origins and nothing else', () => {
  const csp = buildCsp({csp: {connect: [], style: [], font: [], media: ['https://dotx3.net'], img: ['https://dotx3.net']}}, []);
  assert.ok(csp.includes("media-src 'self' https://dotx3.net;"), csp);
  assert.ok(csp.includes("img-src 'self' data: https://dotx3.net;"), csp);
  assert.ok(csp.includes("connect-src 'self';"), 'media origins do not widen connect-src');
  assert.ok(csp.startsWith("default-src 'none'"));
});

/** Read-only provider observations. Never authorizes or proves settlement. */
import { createHash, randomUUID } from 'node:crypto';
const ENDPOINT = 'https://explorer.near-intents.org/api/v0/transactions';
const STATUSES = ['FAILED','INCOMPLETE_DEPOSIT','PENDING_DEPOSIT','PROCESSING','REFUNDED','SUCCESS'];
export class ExplorerError extends Error {
  constructor(code, status = 503, retryAfter = 0) {
    super(code); this.code = code; this.status = status; this.retryAfter = retryAfter;
  }
}
const hash = value => createHash('sha256').update(value).digest('hex');
/** Validate only supported public filters; search scope is server-controlled. */
export function queryParams(input, scope) {
  if (!scope || scope.length > 256) throw new ExplorerError('EXPLORER_SCOPE_NOT_CONFIGURED');
  const params = new URLSearchParams({ search: scope, numberOfTransactions: '25', statuses: STATUSES.join(',') });
  const allowed = ['statuses','fromChainId','toChainId','lastDepositAddress','lastDepositMemo'];
  for (const [key, value] of input) {
    if (!allowed.includes(key) || input.getAll(key).length !== 1 || !value || value.length > 256 || /[\x00-\x1f]/.test(value))
      throw new ExplorerError('INVALID_EXPLORER_FILTER', 400);
    if (key === 'statuses' && !STATUSES.includes(value)) throw new ExplorerError('INVALID_EXPLORER_STATUS',400);
    if (key.endsWith('ChainId') && !/^[a-z0-9]{2,20}$/.test(value)) throw new ExplorerError('INVALID_EXPLORER_CHAIN',400);
    params.set(key,value);
  }
  if (params.has('lastDepositMemo') && !params.has('lastDepositAddress')) throw new ExplorerError('INVALID_EXPLORER_CURSOR',400);
  params.sort(); return params;
}
export function normalizeTransactions(data, observedAt) {
  if (!Array.isArray(data) || data.length > 25) throw new ExplorerError('INVALID_EXPLORER_RESPONSE',502);
  const fields = ['depositAddress','depositMemo','recipient','status','createdAt','originAsset','destinationAsset','amountInFormatted','amountOutFormatted'];
  const transactions = data.map(row => {
    if (!row || typeof row !== 'object' || typeof row.depositAddress !== 'string' || !row.depositAddress || !STATUSES.includes(row.status))
      throw new ExplorerError('INVALID_EXPLORER_RESPONSE',502);
    const clean = {};
    for (const field of fields) {
      if (row[field] != null && (typeof row[field] !== 'string' || row[field].length > 2048)) throw new ExplorerError('INVALID_EXPLORER_RESPONSE',502);
      clean[field] = row[field] ?? null;
    }
    return clean;
  });
  const last = transactions.at(-1);
  return { source: 'NEAR Intents Explorer', evidenceState: 'provider-reported', independentlyVerified: false,
    executionAllowed: false, observedAt, transactions,
    nextCursor: transactions.length === 25 ? { lastDepositAddress: last.depositAddress, lastDepositMemo: last.depositMemo } : null };
}
/** All consumers of a partner credential must use the same Redis deployment. */
export function createExplorerClient({ env = process.env, fetchImpl = fetch } = {}) {
  async function redis(command) {
    const response = await fetchImpl(env.X3_EXPLORER_REDIS_URL, { method: 'POST', redirect: 'error',
      headers: { Authorization: `Bearer ${env.X3_EXPLORER_REDIS_TOKEN}`, 'Content-Type':'application/json' },
      body: JSON.stringify(command), signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw new ExplorerError('EXPLORER_COORDINATOR_UNAVAILABLE');
    const data = await response.json();
    if (data.error) throw new ExplorerError('EXPLORER_COORDINATOR_UNAVAILABLE');
    return data.result;
  }
  return { async transactions(input = new URLSearchParams()) {
    try {
      if (!env.NEAR_INTENTS_EXPLORER_JWT || !env.X3_EXPLORER_REDIS_URL || !env.X3_EXPLORER_REDIS_TOKEN)
        throw new ExplorerError('EXPLORER_NOT_CONFIGURED');
      if (!env.X3_EXPLORER_REDIS_URL.startsWith('https://')) throw new ExplorerError('EXPLORER_COORDINATOR_UNAVAILABLE');
      // Decode only to namespace the limiter. The provider verifies authentication.
      let partner;
      try { const payload = JSON.parse(Buffer.from(env.NEAR_INTENTS_EXPLORER_JWT.split('.')[1], 'base64url').toString());
        partner = payload.partner_id;
        if (payload.key_type !== 'explorer' || typeof partner !== 'string' || !partner) throw Error();
      } catch { throw new ExplorerError('EXPLORER_CREDENTIAL_INVALID',503); }
      const query = queryParams(input, env.X3_EXPLORER_SEARCH);
      const prefix = `x3:explorer:${hash(partner)}`;
      const cacheKey = `${prefix}:cache:${hash(query.toString())}`;
      const cached = await redis(['GET', cacheKey]);
      if (cached) return { ...JSON.parse(cached), cached: true };
      const lockKey = `${prefix}:lock`; const owner = randomUUID();
      if (await redis(['SET',lockKey,owner,'NX','PX','30000']) !== 'OK') throw new ExplorerError('EXPLORER_RATE_LIMITED',429,5);
      let cooldown = 5000;
      try {
        const response = await fetchImpl(`${ENDPOINT}?${query}`, { redirect:'error',
          headers:{ Authorization:`Bearer ${env.NEAR_INTENTS_EXPLORER_JWT}`, Accept:'application/json' },
          signal:AbortSignal.timeout(10000) });
        if (response.status === 429) {
          const header = response.headers.get('retry-after');
          const seconds = Number(header);
          cooldown = Math.max(5000, Number.isFinite(seconds) ? seconds * 1000 : (Date.parse(header || '') - Date.now()) || 5000);
          throw new ExplorerError('EXPLORER_RATE_LIMITED',429,Math.ceil(cooldown/1000));
        }
        if ([401,403].includes(response.status)) throw new ExplorerError('EXPLORER_AUTH_REJECTED',503);
        if (!response.ok) throw new ExplorerError('EXPLORER_UPSTREAM_UNAVAILABLE',502);
        const body = await response.text();
        if (body.length > 1000000) throw new ExplorerError('INVALID_EXPLORER_RESPONSE',502);
        const result = normalizeTransactions(JSON.parse(body),new Date().toISOString());
        await redis(['SET',cacheKey,JSON.stringify(result),'EX','30']);
        return {...result,cached:false};
      } finally {
        // Never shorten another worker's lock; retain a cooldown even after errors.
        await redis(['EVAL', "if redis.call('get',KEYS[1]) == ARGV[1] then return redis.call('pexpire',KEYS[1],ARGV[2]) else return 0 end", '1', lockKey, owner, String(cooldown)]);
      }
    } catch (error) {
      if (error instanceof ExplorerError) throw error;
      throw new ExplorerError('EXPLORER_UNAVAILABLE'); // Never echo upstream bodies or tokens.
    }
  }};
}

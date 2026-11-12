/** Local planning only. Does not alter signed quotes or authorize funding. */
export function planDeadline(input: {
  dry: boolean;
  nowMs: number;
  estimateSeconds?: number;
  defaultSeconds: number;
  maximumSeconds: number;
  ownerExpiresMs: number;
}) {
  const { dry, nowMs, estimateSeconds, defaultSeconds, maximumSeconds, ownerExpiresMs } = input;
  if (
    typeof dry !== "boolean" ||
    ![nowMs, defaultSeconds, maximumSeconds, ownerExpiresMs].every(Number.isSafeInteger) ||
    defaultSeconds <= 0 ||
    maximumSeconds < defaultSeconds ||
    nowMs < 0 ||
    ownerExpiresMs <= nowMs
  )
    return { ok: false, reason: "Invalid windows or expired owner authorization", deadline: null };
  if (!dry && (!Number.isSafeInteger(estimateSeconds) || estimateSeconds! <= 0))
    return { ok: false, reason: "A positive execution estimate is required", deadline: null };
  const window = dry
    ? maximumSeconds
    : Math.min(maximumSeconds, Math.max(defaultSeconds, estimateSeconds!));
  const deadlineMs = Math.min(nowMs + window * 1000, ownerExpiresMs);
  if (!Number.isSafeInteger(deadlineMs) || deadlineMs > 8640000000000000)
    return { ok: false, reason: "Deadline out of range", deadline: null };
  const fits = dry || estimateSeconds! * 1000 <= deadlineMs - nowMs;
  return {
    ok: fits,
    reason: dry
      ? "Informational only; never executable"
      : fits
        ? "Estimate fits; all other authorization checks still required"
        : "Estimate exceeds the permitted window; refuse execution",
    deadline: new Date(deadlineMs).toISOString(),
  };
}
export function genericPublicFees(appBps: number, authenticated: boolean, stableRoute: boolean) {
  if (typeof authenticated !== "boolean" || typeof stableRoute !== "boolean")
    throw Error("Fee planning flags must be boolean");
  if (!Number.isSafeInteger(appBps) || appBps < 0 || appBps > 500)
    throw Error("Fee must be an integer from 0 to 500 bps");
  const floor = stableRoute ? 1 : 20;
  const developer = authenticated ? Math.ceil(appBps / 2) : appBps;
  // Public default split rounds both halves up; the signed quote remains authoritative.
  // https://docs.near-intents.org/integration/distribution-channels/1click-api/fee-config
  const provider = authenticated ? Math.max(floor, Math.ceil(appBps / 2)) : 25;
  const total = developer + provider;
  return { developer, provider, total, withinCap: total <= 500 };
}

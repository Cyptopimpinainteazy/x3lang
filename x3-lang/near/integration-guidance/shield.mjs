/** Official provider incidents only. Never infers a security mode. */
export function publicIncidents(data, observedAt) {
  if (data?.status === "operational")
    return {
      status: "operational",
      incidents: [],
      observedAt,
      securityMode: "unknown",
      source: "Official NEAR Intents SHIELD",
    };
  if (data?.status !== "incidents" || !Array.isArray(data.incidents) || data.incidents.length > 500)
    throw Error("Invalid incident payload");
  const incidents = data.incidents.map((row) => {
    if (
      !["chain", "bridge", "token", "address"].includes(row.scopeType) ||
      typeof row.scopeValue !== "string" ||
      row.scopeValue.length > 512
    )
      throw Error("Invalid incident scope");
    return {
      scopeType: row.scopeType,
      scopeValue: row.scopeValue,
      direction: typeof row.direction === "string" ? row.direction.slice(0, 64) : null,
      publicDescription:
        typeof row.publicDescription === "string" ? row.publicDescription.slice(0, 2000) : null,
    };
  });
  return {
    status: "incidents",
    incidents,
    observedAt,
    securityMode: "unknown",
    source: "Official NEAR Intents SHIELD",
  };
}
export function createShieldReader({ env = process.env, fetcher = fetch, clock = Date.now } = {}) {
  let cache = null;
  let until = 0;
  let pending = null;
  return async () => {
    if (!env.NEAR_INTENTS_SHIELD_JWT)
      return {
        status: "unknown",
        reason: "not-configured",
        securityMode: "unknown",
        incidents: [],
        observedAt: null,
      };
    if (cache && clock() < until) return cache;
    if (pending) return pending;
    pending = (async () => {
      try {
        const response = await fetcher("https://shield.chaindefuser.com/incident", {
          method: "GET",
          redirect: "error",
          headers: {
            Authorization: `Bearer ${env.NEAR_INTENTS_SHIELD_JWT}`,
            Accept: "application/json",
          },
          signal: AbortSignal.timeout(8000),
        });
        if (!response.ok) throw Error();
        const result = publicIncidents(await response.json(), new Date(clock()).toISOString());
        cache = result;
        until = clock() + 30000;
        return result;
      } catch {
        return {
          status: "unknown",
          reason: "unavailable",
          securityMode: "unknown",
          incidents: [],
          observedAt: null,
        };
      } finally {
        pending = null;
      }
    })();
    return pending;
  };
}

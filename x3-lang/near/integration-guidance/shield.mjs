/** Official provider incidents only. Never infers a security mode. */
export function publicIncidents(data, observedAt) {
  if (!data || typeof data !== "object" || Array.isArray(data))
    throw Error("Invalid incident payload");
  if (data.status === "operational") {
    // Never discard contradictory incident evidence as a healthy observation.
    if (data.incidents !== undefined && (!Array.isArray(data.incidents) || data.incidents.length))
      throw Error("Contradictory operational payload");
    return {
      status: "operational",
      incidents: [],
      observedAt,
      securityMode: "unknown",
      source: "Official NEAR Intents SHIELD",
    };
  }
  if (data?.status !== "incidents" || !Array.isArray(data.incidents) || data.incidents.length > 500)
    throw Error("Invalid incident payload");
  const incidents = data.incidents.map((row) => {
    if (
      !row ||
      typeof row !== "object" ||
      Array.isArray(row) ||
      !["chain", "bridge", "token", "address"].includes(row.scopeType) ||
      typeof row.scopeValue !== "string" ||
      !row.scopeValue.trim() ||
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
// Bound bytes before parsing or retaining provider content, including private fields.
const MAX_RESPONSE_BYTES = 1024 * 1024;
async function readIncidentPayload(response) {
  const length = response.headers.get("content-length");
  if (length && /^\d+$/.test(length) && Number(length) > MAX_RESPONSE_BYTES) {
    await response.body?.cancel();
    throw Error("Incident response too large");
  }
  if (!response.body) throw Error("Missing incident response");
  const reader = response.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let bytes = 0;
  let text = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > MAX_RESPONSE_BYTES) throw Error("Incident response too large");
      text += decoder.decode(value, { stream: true });
    }
    text += decoder.decode();
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
  return JSON.parse(text);
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
        const result = publicIncidents(await readIncidentPayload(response), new Date(clock()).toISOString());
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

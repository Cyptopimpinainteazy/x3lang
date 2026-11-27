// Browser-held Ed25519 keys (WebCrypto), kept in IndexedDB. The Forge host never sees a
// private key: it gets public keys and signatures over payloads it produced.
//
// - High Command (operator) key: non-extractable. It signs the claim, every agent genesis and
//   every revocation. Losing this browser profile means losing High Command for this Forge.
// - Agent keys: extractable, so the owner can export one for the agent's own runtime. Each
//   signs that agent's mesh messages.

const DB = "x3-agent-forge";
const STORE = "keys";
const OPERATOR = "operator";

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function put(name: string, value: CryptoKeyPair): Promise<void> {
  const db = await open();
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).put(value, name);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

async function get(name: string): Promise<CryptoKeyPair | undefined> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readonly");
    const req = tx.objectStore(STORE).get(name);
    req.onsuccess = () => resolve(req.result as CryptoKeyPair | undefined);
    req.onerror = () => reject(req.error);
  });
}

export const hex = (b: ArrayBuffer | Uint8Array) =>
  Array.from(b instanceof Uint8Array ? b : new Uint8Array(b), (x) => x.toString(16).padStart(2, "0")).join("");
export const unhex = (h: string) => new Uint8Array(h.match(/../g)?.map((x) => parseInt(x, 16)) ?? []);

export async function supported(): Promise<boolean> {
  try {
    await crypto.subtle.generateKey({ name: "Ed25519" }, false, ["sign", "verify"]);
    return true;
  } catch {
    return false;
  }
}

async function generate(extractable: boolean): Promise<CryptoKeyPair> {
  return (await crypto.subtle.generateKey({ name: "Ed25519" }, extractable, ["sign", "verify"])) as CryptoKeyPair;
}

export async function publicHex(pair: CryptoKeyPair): Promise<string> {
  return hex(await crypto.subtle.exportKey("raw", pair.publicKey));
}

export async function sign(pair: CryptoKeyPair, payloadHex: string): Promise<string> {
  return hex(await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, unhex(payloadHex)));
}

export async function operatorKey(): Promise<CryptoKeyPair | undefined> {
  return get(OPERATOR);
}

export async function createOperatorKey(): Promise<CryptoKeyPair> {
  const existing = await get(OPERATOR);
  if (existing) return existing;
  const pair = await generate(false);
  await put(OPERATOR, pair);
  return pair;
}

export async function newAgentKey(): Promise<CryptoKeyPair> {
  return generate(true);
}

export async function saveAgentKey(agentId: string, pair: CryptoKeyPair): Promise<void> {
  await put(`agent:${agentId}`, pair);
}

export async function agentKey(agentId: string): Promise<CryptoKeyPair | undefined> {
  return get(`agent:${agentId}`);
}

export async function exportAgentJwk(pair: CryptoKeyPair): Promise<JsonWebKey> {
  return crypto.subtle.exportKey("jwk", pair.privateKey);
}

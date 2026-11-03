// x3guard.mjs: X3 intents settled through NEAR Intents, checked in the browser.
//
// A dependency-free port of the parts of `x3-lang/near_intents.py` a judge needs to see
// working: parse an X3 `intent`, lower it to a 1Click quote request, verify the quote's
// Ed25519 signature exactly as `@defuse-protocol/one-click-sdk-typescript` 0.1.26
// (`verifyQuoteSignature`, ISC) does, and run the program's guards on the quote.
//
// `x3-lang/tests/test_demo_parity.py` runs this module and the Python adapter on the same
// programs and quotes and requires identical results, so the two cannot drift.
//
// Everything fails closed: a program this cannot honour is refused with the same error code
// the Python adapter uses, and a quote is trusted only after its signature verifies.

export const ONE_CLICK_BASE_URL = "https://1click.chaindefuser.com";
export const ONE_CLICK_MANAGER_PUBLIC_KEY = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";
export const MAX_QUOTE_AGE_S = 300;
export const MAX_CLOCK_SKEW_S = 60;

export class X3Error extends Error {
  constructor(code, message, { line = null, field = null, detail = null } = {}) {
    super(message);
    this.code = code;
    this.line = line;
    this.field = field;
    this.detail = detail;
  }
}

// ---------------------------------------------------------------------------------------
// Registry (x3-lang/registry.py)
// ---------------------------------------------------------------------------------------

const CHAIN_DOMAINS = {
  solana: { aliases: ["sol", "svm"], address: "base58" },
  ethereum: { aliases: ["eth", "evm"], address: "evm" },
  arbitrum: { aliases: ["arb"], address: "evm" },
  polygon: { aliases: ["matic"], address: "evm" },
  bitcoin: { aliases: ["btc"], address: "btc" },
  x3: { aliases: [], address: "x3" },
  near: { aliases: [], address: "near" },
};
const ALIASES = {};
for (const [chain, meta] of Object.entries(CHAIN_DOMAINS)) {
  ALIASES[chain] = chain;
  for (const a of meta.aliases) ALIASES[a] = chain;
}
const ASSETS_BY_CHAIN = {
  solana: ["USDC", "SOL", "WSOL", "USDT", "BONK"],
  ethereum: ["USDC", "USDT", "WETH", "ETH", "WSOL", "DAI"],
  arbitrum: ["USDC", "USDT", "WETH", "ETH", "ARB"],
  polygon: ["USDC", "USDT", "WETH", "MATIC", "DAI"],
  bitcoin: ["BTC", "WBTC"],
  x3: ["SOL", "USDC", "WSOL", "WETH", "BTC", "X3"],
  near: ["NEAR", "wNEAR", "USDC", "USDT"],
};
export const NEAR_INTENTS_VIAS = ["nearintents", "near_intents"];
const KNOWN_BRIDGES = ["x3", "wormhole", "layerzero", "axelar", ...NEAR_INTENTS_VIAS];
const KNOWN_DEXS = ["raydium", "orca", "uniswap", "sushiswap", "curve"];
const SUPPORTED_OPERATIONS = ["swap", "bridge", "lock", "mint", "burn", "release"];
export const REQUIRE_KINDS = [
  "finality", "slippage", "fees", "profit", "invariant", "risk", "nonce", "audit_gate",
  "bridge_liquidity", "canonical_supply", "relayer_quorum", "route_score", "solver_bond",
  "proof_complete", "refund_path", "refund_to", "finality_explicit", "vm_supported",
  "mainnet_safe", "proof",
];

export function normalizeChain(chain) {
  if (chain == null) return null;
  return ALIASES[String(chain).toLowerCase()] ?? null;
}

const NEAR_ACCOUNT_RE = /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/;
export function isValidNearAccount(account) {
  const text = String(account);
  if (/^0x[a-f0-9]{40}$/.test(text)) return true;
  return text.length >= 2 && text.length <= 64 && NEAR_ACCOUNT_RE.test(text);
}

export function isValidReceiver(chain, receiver) {
  if (receiver == null || receiver === "" || receiver === "sender") return true;
  const kind = CHAIN_DOMAINS[normalizeChain(chain)]?.address;
  const r = String(receiver);
  if (kind === "evm") return /^0x[a-fA-F0-9]+$/.test(r);
  if (kind === "base58") return /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(r);
  if (kind === "btc") return /^(bc1|[13])[a-zA-HJ-NP-Z0-9]{25,90}$/.test(r);
  if (kind === "x3") return /^(x3_[a-zA-Z0-9_]{6,}|0x[a-fA-F0-9]{40})$/.test(r);
  if (kind === "near") return isValidNearAccount(r);
  return false;
}

// ---------------------------------------------------------------------------------------
// Parser (x3-lang/cli.py: the `intent` surface)
// ---------------------------------------------------------------------------------------

function cleanLines(source) {
  const out = [];
  source.split(/\r?\n/).forEach((raw, i) => {
    let s = raw.trim();
    if (!s || s.startsWith("//") || s.startsWith("#")) return;
    s = s.replace(/\s+(\/\/|#).*$/, "").trim();
    s = s.replace(/;+$/, "");
    if (s) out.push({ no: i + 1, text: s });
  });
  return out;
}

function assetRef(value, line, field) {
  const m = /^([A-Za-z][A-Za-z0-9_-]*)\.([A-Za-z][A-Za-z0-9_-]*)$/.exec(value ?? "");
  if (!m) throw new X3Error("X3_PARSE_ASSET_REF", `expected chain.asset reference for ${field}`, { line, field });
  return { chain: m[1].toLowerCase(), asset: m[2] };
}

function parseReceiver(tokens, line) {
  const idx = tokens.indexOf("receiver");
  if (idx < 0) return null;
  if (idx + 1 >= tokens.length) throw new X3Error("X3_PARSE_RECEIVER", "receiver requires an address", { line, field: "receiver" });
  const address = tokens[idx + 1].replace(/^"|"$/g, "");
  if (address.startsWith("0x") && !/^0x[0-9a-fA-F]+$/.test(address)) {
    throw new X3Error("X3_PARSE_RECEIVER", `invalid Ethereum address '${address}'`, { line, field: "receiver" });
  }
  return address;
}

function parseEndpoint(ln, keyword) {
  const parts = ln.text.split(/\s+/);
  if (parts.length < 2 || parts[0] !== keyword) throw new X3Error("X3_PARSE_ENDPOINT", `expected ${keyword} chain.asset`, { line: ln.no, field: keyword });
  const ep = assetRef(parts[1], ln.no, keyword);
  const ai = parts.indexOf("amount");
  if (ai >= 0) {
    if (ai + 1 >= parts.length) throw new X3Error("X3_PARSE_AMOUNT", `${keyword}.amount requires a value`, { line: ln.no });
    ep.amount = parts[ai + 1];
  } else ep.amount = null;
  const receiver = parseReceiver(parts, ln.no);
  if (receiver) ep.receiver = receiver;
  return ep;
}

function parseRefund(tokens, line) {
  if (!tokens.length || tokens[0] !== "refund" || tokens.length < 2) {
    throw new X3Error("X3_PARSE_REFUND", "expected refund <chain.asset> [to <receiver>]", { line, field: "refund" });
  }
  let to = "sender";
  if (tokens.length > 2) {
    if (tokens[2] !== "to" || tokens.length < 4) throw new X3Error("X3_PARSE_REFUND", "expected refund <chain.asset> [to <receiver>]", { line, field: "refund" });
    to = tokens[3].replace(/^"|"$/g, "");
  }
  const a = assetRef(tokens[1], line, "refund.asset");
  return { type: "refund", chain: a.chain, asset: a.asset, to };
}

function parseRequire(ln) {
  const t = ln.text.split(/\s+/);
  const kind = (t[1] ?? "").toLowerCase();
  const at = (r) => ({ ...r, line: ln.no });
  if (kind.startsWith("finality.") && t.length >= 4) return at({ kind: "finality", chain: kind.split(".")[1], op: t[2], value: t[3] });
  if (kind === "finality" && t.length >= 5) return at({ kind: "finality", chain: t[2].toLowerCase(), op: t[3], value: t[4] });
  if (kind === "slippage" && t.length >= 4) return at({ kind: "slippage", op: t[2], value: t[3] });
  if (kind === "nonce" && t.length >= 3) return at({ kind: "nonce", value: t.slice(2).join(" ") });
  if (REQUIRE_KINDS.includes(kind)) {
    if (t.length >= 4) return at({ kind, op: t[2], value: t.slice(3).join(" ") });
    if (t.length >= 3) return at({ kind, value: t.slice(2).join(" ") });
  }
  throw new X3Error("X3_PARSE_REQUIRE", `malformed require '${kind}': not a guard kind the language defines`, { line: ln.no, field: "require" });
}

function parseRouteStep(ln) {
  const p = ln.text.split(/\s+/);
  const op = p[0].toLowerCase();
  if (op === "bridge") {
    if (p.length < 5 || p[3] !== "->") throw new X3Error("X3_PARSE_BRIDGE", "expected bridge <via> <from> -> <to>", { line: ln.no, field: "route.bridge" });
    const from_ref = assetRef(p[2], ln.no, "bridge.from");
    const to_ref = assetRef(p[4], ln.no, "bridge.to");
    const step = { type: "bridge", via: p[1].toLowerCase(), from_ref, to_ref, asset: from_ref.asset, line: ln.no };
    for (const key of ["amount", "min_receive"]) {
      const i = p.indexOf(key);
      if (i >= 0) {
        if (i + 1 >= p.length) throw new X3Error("X3_PARSE_BRIDGE", `bridge ${key} requires a value`, { line: ln.no });
        step[key] = p[i + 1];
      }
    }
    const r = parseReceiver(p, ln.no);
    if (r) step.receiver = r;
    return step;
  }
  if (op === "swap") {
    if (p.length < 5 || p[3] !== "->") throw new X3Error("X3_PARSE_SWAP", "expected swap <dex> <from> -> <to>", { line: ln.no, field: "route.swap" });
    return { type: "swap", dex: p[1].toLowerCase(), from_ref: assetRef(p[2], ln.no, "swap.from"), to_ref: assetRef(p[4], ln.no, "swap.to"), line: ln.no };
  }
  if (["lock", "mint", "burn", "release"].includes(op)) return { type: op, ...assetRef(p[1], ln.no, `${op}.asset`), line: ln.no };
  throw new X3Error("X3_PARSE_OPERATION", `unsupported route operation '${op}'`, { line: ln.no, field: "route" });
}

/** Parse one `intent` (declarations above it are skipped, as in cli.py). */
export function parse(source) {
  const lines = cleanLines(source);
  if (!lines.length) throw new X3Error("X3_PARSE_EMPTY", "the program is empty");
  let i = 0;
  while (i < lines.length && !/^intent\s+[A-Za-z_]/.test(lines[i].text)) {
    if (/^error\s+[A-Za-z_][A-Za-z0-9_]*$/.test(lines[i].text)) { i++; continue; }
    if (!/^[A-Za-z_][A-Za-z0-9_-]*(\s+[A-Za-z_][A-Za-z0-9_.-]*)*\s*\{/.test(lines[i].text)) break;
    const header = lines[i];
    let depth = 0;
    while (i < lines.length) {
      depth += (lines[i].text.match(/\{/g) || []).length - (lines[i].text.match(/\}/g) || []).length;
      i++;
      if (depth <= 0) break;
    }
    if (depth > 0) throw new X3Error("X3_PARSE_DECLARATION", `declaration '${header.text}' is never closed`, { line: header.no });
  }
  const m = i < lines.length ? /^intent\s+([A-Za-z_][A-Za-z0-9_-]*)\s*\{?/.exec(lines[i].text) : null;
  if (!m) throw new X3Error("X3_PARSE_NO_INTENT", "this program declares no `intent`", { line: lines[Math.min(i, lines.length - 1)].no });
  const intent = { intent: m[1], from: {}, to: {}, route: [], requires: [], policies: {}, lines: { intent: lines[i].no } };
  for (i = i + 1; i < lines.length; i++) {
    const ln = lines[i];
    const t = ln.text;
    if (t === "}") continue;
    if (t.startsWith("from ")) { intent.from = parseEndpoint(ln, "from"); intent.lines.from = ln.no; }
    else if (t.startsWith("to ")) { intent.to = parseEndpoint(ln, "to"); intent.lines.to = ln.no; }
    else if ((t.startsWith("route") || t.startsWith("path")) && t.includes("{")) {
      for (i = i + 1; i < lines.length && lines[i].text !== "}"; i++) {
        if (lines[i].text.startsWith("fallback")) throw new X3Error("X3_NEAR_ROUTE_SHAPE", "a NEAR Intents program has one route leg and no fallback", { line: lines[i].no });
        intent.route.push(parseRouteStep(lines[i]));
      }
    } else if (t.startsWith("require ")) intent.requires.push(parseRequire(ln));
    else if (t.startsWith("timeout ")) {
      const p = t.split(/\s+/);
      const timeout = { duration: p[1] };
      if (p.length > 2) timeout.action = parseRefund(p.slice(2), ln.no);
      intent.policies.timeout = timeout;
      intent.lines.timeout = ln.no;
    } else if (t.startsWith("under ")) {
      const p = t.split(/\s+/);
      if (p.length !== 3 || p[1] !== "mandate") throw new X3Error("X3_PARSE_MANDATE", "expected `under mandate <owner account>`", { line: ln.no, field: "mandate" });
      if (intent.mandate) throw new X3Error("X3_PARSE_MANDATE", "an intent is under one mandate", { line: ln.no, field: "mandate" });
      intent.mandate = { owner: p[2].replace(/"/g, "") };
      intent.lines.mandate = ln.no;
    } else if (t.startsWith("on_fail ")) {
      const p = t.split(/\s+/).slice(1);
      intent.policies.on_fail = ["rollback", "halt", "quarantine"].includes(p[0]) ? { type: p[0] } : parseRefund(p, ln.no);
      intent.lines.on_fail = ln.no;
    }
  }
  return intent;
}

/** The parts of typechecker.py a NEAR Intents program can hit. Returns error objects. */
export function typecheck(intent) {
  const errors = [];
  const err = (code, message, field, line) => errors.push({ code, message, field, line });
  const checkAsset = (chain, asset, field, line) => {
    const norm = normalizeChain(chain);
    if (!norm) { err("X3_INVALID_CHAIN", `unsupported chain '${chain}'`, `${field}.chain`, line); return null; }
    if (!ASSETS_BY_CHAIN[norm].includes(asset)) err("X3_INVALID_ASSET", `asset '${asset}' not registered on ${norm}`, `${field}.asset`, line);
    return norm;
  };
  for (const key of ["from", "to"]) {
    const ep = intent[key];
    if (!ep || !ep.chain) { err("X3_MISSING_ENDPOINT", `missing ${key} endpoint`, key, intent.lines.intent); continue; }
    const chain = checkAsset(ep.chain, ep.asset, key, intent.lines[key]);
    if (chain && !isValidReceiver(chain, ep.receiver)) err("X3_INVALID_RECEIVER", `invalid receiver for ${chain}`, `${key}.receiver`, intent.lines[key]);
  }
  if (!intent.route.length) err("X3_EMPTY_ROUTE", "route must contain at least one operation", "route", intent.lines.intent);
  intent.route.forEach((step, i) => {
    if (!SUPPORTED_OPERATIONS.includes(step.type)) err("X3_INVALID_OPERATION", `unsupported operation '${step.type}'`, `route[${i}]`, step.line);
    if (step.type === "bridge" && !KNOWN_BRIDGES.includes(step.via)) err("X3_INVALID_BRIDGE", "unknown bridge adapter", `route[${i}].via`, step.line);
    if (step.type === "swap" && !KNOWN_DEXS.includes(step.dex)) err("X3_INVALID_DEX", "unknown DEX", `route[${i}].dex`, step.line);
    if (step.from_ref) checkAsset(step.from_ref.chain, step.from_ref.asset, `route[${i}].from`, step.line);
    if (step.to_ref) {
      const c = checkAsset(step.to_ref.chain, step.to_ref.asset, `route[${i}].to`, step.line);
      if (c && step.type === "bridge" && !isValidReceiver(c, step.receiver ?? intent.to?.receiver)) err("X3_INVALID_RECEIVER", `invalid receiver for ${c}`, `route[${i}].receiver`, step.line);
    }
  });
  return errors;
}

// ---------------------------------------------------------------------------------------
// Exact decimals (BigInt), so bps rounding matches Python's Decimal
// ---------------------------------------------------------------------------------------

/** "12.345" -> { n: 12345n, scale: 3 }. Refuses anything that is not a plain decimal. */
function decimal(text, field) {
  const s = String(text).trim();
  const m = /^(\d+)(?:\.(\d+))?$/.exec(s);
  if (!m) throw new X3Error("X3_NEAR_NUMBER", `${field} is not a number: '${text}'`, { field });
  const frac = m[2] ?? "";
  return { n: BigInt(m[1] + frac), scale: frac.length };
}

const pow10 = (k) => 10n ** BigInt(k);

export function baseUnits(value, field) {
  const text = String(value).trim().replace(/_/g, "");
  if (!/^\d+$/.test(text)) throw new X3Error("X3_NEAR_AMOUNT", `${field} must be an integer in base units, got '${value}'`, { field });
  const n = BigInt(text);
  if (n <= 0n) throw new X3Error("X3_NEAR_AMOUNT", `${field} must be positive`, { field });
  return n;
}

/** `0.5%`, `50 bps`, or a bare fraction -> { n, d } basis points as an exact fraction. */
function toBps(value, field) {
  const text = String(value).trim().toLowerCase().replace(/\s+/g, "");
  let d;
  let mul;
  if (text.endsWith("bps")) { d = decimal(text.slice(0, -3), field); mul = 1n; }
  else if (text.endsWith("%")) { d = decimal(text.slice(0, -1), field); mul = 100n; }
  else {
    d = decimal(text, field);
    if (d.n >= pow10(d.scale)) throw new X3Error("X3_NEAR_BOUND_UNIT", `${field} '${value}' has no unit; write it as a percent (0.5%) or in bps (50 bps)`, { field });
    mul = 10000n;
  }
  return { n: d.n * mul, d: pow10(d.scale) };
}

function upperBoundBps(req, field) {
  if (req.op !== "<=" && req.op !== "<") throw new X3Error("X3_NEAR_GUARD_OPERATOR", `${field} must be an upper bound (<= or <), got '${req.op}'`, { field, line: req.line });
  const { n, d } = toBps(req.value, field);
  let whole = n / d; // floor for non-negative values
  if (req.op === "<" && n % d === 0n) whole -= 1n;
  if (whole < 0n) throw new X3Error("X3_NEAR_BOUND_UNIT", `${field} allows no value at all`, { field, line: req.line });
  return Number(whole);
}

// ---------------------------------------------------------------------------------------
// Lowering (near_intents.lower)
// ---------------------------------------------------------------------------------------

export const CHAIN_TO_BLOCKCHAIN = { near: "near", ethereum: "eth", solana: "sol", arbitrum: "arb", polygon: "pol", bitcoin: "btc" };
const SYMBOL_ALIASES = { "near/NEAR": "wNEAR" };
const ENFORCED_GUARDS = ["slippage", "fees", "nonce"];

export function formatTimestamp(ms) {
  return new Date(ms).toISOString(); // YYYY-MM-DDTHH:MM:SS.mmmZ, as Python's format_timestamp
}

export function parseTimestampMs(text) {
  const m = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:\d{2})$/.exec(String(text));
  if (!m) throw new X3Error("X3_NEAR_TIMESTAMP", `unreadable timestamp '${text}'`);
  const frac = (m[2] ?? ".0").slice(1, 4).padEnd(3, "0");
  const ms = Date.parse(`${m[1]}.${frac}${m[3]}`);
  if (Number.isNaN(ms)) throw new X3Error("X3_NEAR_TIMESTAMP", `unreadable timestamp '${text}'`);
  return ms;
}

function parseDurationMs(text) {
  const m = /^(\d+)(ms|s|m|h)$/.exec(String(text).trim());
  if (!m) throw new X3Error("X3_NEAR_TIMEOUT_UNIT", `timeout '${text}' is not a wall-clock duration (<n>ms|s|m|h)`, { field: "policies.timeout.duration" });
  const ms = Number(m[1]) * { ms: 1, s: 1000, m: 60000, h: 3600000 }[m[2]];
  if (ms <= 0) throw new X3Error("X3_NEAR_TIMEOUT_UNIT", "timeout must be positive");
  return ms;
}

function endpoint(intent, key) {
  const ep = intent[key];
  if (!ep || !ep.chain || !ep.asset) throw new X3Error("X3_NEAR_ENDPOINT", `intent has no \`${key}\` chain.asset`, { field: key, line: intent.lines.intent });
  const chain = normalizeChain(ep.chain);
  if (!CHAIN_TO_BLOCKCHAIN[chain]) {
    throw new X3Error("X3_NEAR_CHAIN_UNSUPPORTED", `${key} chain '${ep.chain}' has no NEAR Intents mapping (supported: ${Object.keys(CHAIN_TO_BLOCKCHAIN).sort().join(", ")})`, { field: `${key}.chain`, line: intent.lines[key] });
  }
  return { ...ep, chain };
}

export function lower(intent, nowMs = Date.now()) {
  const route = intent.route;
  const L = intent.lines;
  if (route.length !== 1 || route[0].type !== "bridge" || !NEAR_INTENTS_VIAS.includes(route[0].via)) {
    throw new X3Error("X3_NEAR_ROUTE_SHAPE", "a NEAR Intents program has exactly one route leg, `bridge NearIntents <from> -> <to>`: 1Click settles one swap, and composing it with other legs would allow partial execution with no rollback", { field: "route", line: route[0]?.line ?? L.intent });
  }
  const leg = route[0];
  const source = endpoint(intent, "from");
  const dest = endpoint(intent, "to");
  for (const [refKey, ep, label] of [["from_ref", source, "from"], ["to_ref", dest, "to"]]) {
    const ref = leg[refKey] ?? {};
    if (normalizeChain(ref.chain) !== ep.chain || ref.asset !== ep.asset) {
      throw new X3Error("X3_NEAR_ROUTE_ENDPOINTS", `the NearIntents leg's ${label} side (${ref.chain}.${ref.asset}) is not the intent's \`${label}\` (${ep.chain}.${ep.asset})`, { field: `route[0].${refKey}`, line: leg.line });
    }
  }
  if (source.amount == null) throw new X3Error("X3_NEAR_AMOUNT", "`from` must state an amount", { field: "from.amount", line: L.from });
  const amount = baseUnits(source.amount, "from.amount");
  if (leg.amount != null && baseUnits(leg.amount, "route[0].amount") !== amount) {
    throw new X3Error("X3_NEAR_AMOUNT", "the leg's amount differs from the intent's `from` amount; the leg spends all of it", { field: "route[0].amount", line: leg.line });
  }
  const minReceive = leg.min_receive != null ? baseUnits(leg.min_receive, "route[0].min_receive") : null;
  const recipient = leg.receiver ?? dest.receiver;
  if (!recipient || recipient === "sender") throw new X3Error("X3_NEAR_RECIPIENT", "`to` must name a receiver address", { field: "to.receiver", line: L.to });
  if (!isValidReceiver(dest.chain, recipient)) throw new X3Error("X3_NEAR_RECIPIENT", `'${recipient}' is not a ${dest.chain} address`, { field: "to.receiver", line: L.to });

  const requires = intent.requires;
  const unenforceable = [...new Set(requires.map((r) => r.kind).filter((k) => !ENFORCED_GUARDS.includes(k)))].sort();
  if (unenforceable.length) {
    const first = requires.find((r) => unenforceable.includes(r.kind));
    throw new X3Error("X3_NEAR_GUARD_UNENFORCEABLE", `NEAR Intents gives this adapter no way to enforce: ${unenforceable.join(", ")}. It enforces ${ENFORCED_GUARDS.slice().sort().join(", ")}`, { field: "requires", line: first.line });
  }
  const slippage = requires.filter((r) => r.kind === "slippage").map((r) => upperBoundBps(r, "require slippage"));
  if (!slippage.length) throw new X3Error("X3_NEAR_SLIPPAGE_REQUIRED", "a NEAR Intents swap needs `require slippage <= <bound>`; 1Click requires a tolerance and this adapter will not pick one for you", { field: "requires", line: L.intent });
  const feeCaps = requires.filter((r) => r.kind === "fees").map((r) => upperBoundBps(r, "require fees"));
  const nonces = [];
  for (const r of requires.filter((x) => x.kind === "nonce")) {
    let words = String(r.value).split(/\s+/).filter(Boolean);
    if (words[0] === "unused") words = words.slice(1);
    if (words.length !== 1) throw new X3Error("X3_NEAR_NONCE", "expected `require nonce unused <id>`", { line: r.line });
    nonces.push(words[0]);
  }
  if (new Set(nonces).size > 1) throw new X3Error("X3_NEAR_NONCE", "an intent carries one nonce");

  const timeout = intent.policies.timeout;
  if (!timeout || !timeout.duration) throw new X3Error("X3_NEAR_TIMEOUT_REQUIRED", "a NEAR Intents swap needs `timeout <duration> refund <from asset>`: the deadline is when 1Click starts refunding", { field: "policies.timeout", line: L.intent });
  let windowMs;
  try { windowMs = parseDurationMs(timeout.duration); } catch (e) { e.line = L.timeout; throw e; }
  const refund = timeout.action ?? { type: "refund", chain: source.chain, asset: source.asset, to: "sender" };
  const onFail = intent.policies.on_fail;
  if (onFail && (onFail.type === "halt" || onFail.type === "quarantine")) {
    throw new X3Error("X3_NEAR_ON_FAIL_UNSUPPORTED", `on_fail ${onFail.type} cannot be expressed: 1Click refunds a failed swap to the refund address`, { field: "policies.on_fail", line: L.on_fail });
  }
  const actions = onFail && onFail.type === "refund" ? [[refund, L.timeout], [onFail, L.on_fail]] : [[refund, L.timeout]];
  let refundTo = null;
  for (const [action, line] of actions) {
    if (normalizeChain(action.chain) !== source.chain || action.asset !== source.asset) {
      throw new X3Error("X3_NEAR_REFUND_ASSET", `1Click refunds the deposited asset (${source.chain}.${source.asset}) on the origin chain, not ${action.chain}.${action.asset}`, { field: "policies", line });
    }
    const target = (action.to ?? "sender") === "sender" ? source.receiver : action.to;
    if (!target) throw new X3Error("X3_NEAR_REFUND_TO", "the refund goes to `sender`, and `from` names no receiver to refund", { field: "from.receiver", line: L.from });
    if (!isValidReceiver(source.chain, target)) throw new X3Error("X3_NEAR_REFUND_TO", `'${target}' is not a ${source.chain} address`, { field: "policies", line });
    if (refundTo !== null && refundTo !== target) throw new X3Error("X3_NEAR_REFUND_TO", "timeout and on_fail refund to different addresses; 1Click takes one", { field: "policies", line });
    refundTo = target;
  }
  const out = {
    intent: intent.intent,
    origin: { chain: source.chain, asset: source.asset },
    destination: { chain: dest.chain, asset: dest.asset },
    amount: amount.toString(),
    min_receive: minReceive === null ? null : minReceive.toString(),
    recipient,
    refund_to: refundTo,
    slippage_bps: Math.min(...slippage),
    fee_cap_bps: feeCaps.length ? Math.min(...feeCaps) : null,
    nonce: nonces[0] ?? null,
    deadline: formatTimestamp(nowMs + windowMs),
  };
  if (intent.mandate) {
    const owner = intent.mandate.owner;
    if (!owner || !isValidNearAccount(owner)) throw new X3Error("X3_MANDATE_OWNER", `\`under mandate\` names '${owner}', which is not a NEAR account`, { field: "mandate.owner", line: L.mandate });
    if (!out.nonce) throw new X3Error("X3_NEAR_NONCE", "an intent under a mandate needs `require nonce unused <id>`: x3-guard records it under the owner by its nonce", { field: "requires.nonce", line: L.mandate });
    out.mandate_owner = owner;
  }
  return out;
}

export function resolveAsset(tokens, chain, asset) {
  const blockchain = CHAIN_TO_BLOCKCHAIN[chain];
  const symbol = SYMBOL_ALIASES[`${chain}/${asset}`] ?? asset;
  const matches = tokens.filter((t) => t.blockchain === blockchain && String(t.symbol ?? "").toUpperCase() === symbol.toUpperCase());
  if (!matches.length) throw new X3Error("X3_NEAR_ASSET_UNSUPPORTED", `1Click lists no ${symbol} on ${blockchain}`, { field: `${chain}.${asset}` });
  if (matches.length > 1) throw new X3Error("X3_NEAR_ASSET_AMBIGUOUS", `1Click lists ${matches.length} tokens with symbol ${symbol} on ${blockchain}; refusing to guess`, { field: `${chain}.${asset}`, detail: matches.map((m) => m.assetId) });
  return matches[0];
}

export function buildQuoteRequest(leg, tokens, { dry = true } = {}) {
  const origin = resolveAsset(tokens, leg.origin.chain, leg.origin.asset);
  const destination = resolveAsset(tokens, leg.destination.chain, leg.destination.asset);
  return {
    dry,
    swapType: "EXACT_INPUT",
    slippageTolerance: leg.slippage_bps,
    originAsset: origin.assetId,
    depositType: "ORIGIN_CHAIN",
    destinationAsset: destination.assetId,
    amount: leg.amount,
    refundTo: leg.refund_to,
    refundType: "ORIGIN_CHAIN",
    recipient: leg.recipient,
    recipientType: "DESTINATION_CHAIN",
    deadline: leg.deadline,
  };
}

// ---------------------------------------------------------------------------------------
// 1Click quote signatures (SDK 0.1.26 `quote-signature.ts`, reproduced field for field)
// ---------------------------------------------------------------------------------------

/** json-stable-stringify 1.3.0 with default options. */
export function stableStringify(value) {
  if (value && typeof value.toJSON === "function") value = value.toJSON();
  if (value === undefined) return undefined;
  if (typeof value !== "object" || value === null) return JSON.stringify(value);
  if (Array.isArray(value)) return "[" + value.map((v) => stableStringify(v) ?? "null").join(",") + "]";
  const parts = [];
  for (const k of Object.keys(value).sort()) {
    const v = stableStringify(value[k]);
    if (!v) continue;
    parts.push(JSON.stringify(k) + ":" + v);
  }
  return "{" + parts.join(",") + "}";
}

function signedRequest(r) {
  return {
    dry: r.dry, swapType: r.swapType, slippageTolerance: r.slippageTolerance, originAsset: r.originAsset,
    depositType: r.depositType, destinationAsset: r.destinationAsset, amount: r.amount, refundTo: r.refundTo,
    refundType: r.refundType, recipient: r.recipient, recipientType: r.recipientType, deadline: r.deadline,
    quoteWaitingTimeMs: r.quoteWaitingTimeMs ? r.quoteWaitingTimeMs : undefined,
    referral: r.referral ? r.referral : undefined,
    virtualChainRecipient: r.virtualChainRecipient ? r.virtualChainRecipient : undefined,
    virtualChainRefundRecipient: r.virtualChainRefundRecipient ? r.virtualChainRefundRecipient : undefined,
    customRecipientMsg: r.customRecipientMsg ? r.customRecipientMsg : undefined,
  };
}

function signedQuote(response) {
  const q = response.quote;
  const base = {
    amountIn: q.amountIn, amountInFormatted: q.amountInFormatted, amountInUsd: q.amountInUsd, minAmountIn: q.minAmountIn,
    amountOut: q.amountOut, amountOutFormatted: q.amountOutFormatted, amountOutUsd: q.amountOutUsd, minAmountOut: q.minAmountOut,
  };
  if (response.quoteRequest.dry) return base;
  return {
    ...base,
    depositAddress: q.depositAddress || undefined, depositMemo: q.depositMemo || undefined, deadline: q.deadline || undefined,
    timeWhenInactive: q.timeWhenInactive || undefined, timeEstimate: q.timeEstimate || undefined,
    refundFee: q.refundFee || undefined, withdrawFee: q.withdrawFee || undefined,
  };
}

export function signedQuotePayload(response) {
  if (!response || typeof response !== "object" || typeof response.quote !== "object" || response.quote === null ||
      typeof response.quoteRequest !== "object" || response.quoteRequest === null || typeof response.timestamp !== "string") {
    throw new X3Error("X3_NEAR_SIGNATURE_FORMAT", "the response lacks quote, quoteRequest or timestamp");
  }
  return stableStringify({ ...signedRequest(response.quoteRequest), ...signedQuote(response), timestamp: response.timestamp });
}

const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

export function b58encode(bytes) {
  let n = 0n;
  for (const b of bytes) n = (n << 8n) | BigInt(b);
  let out = "";
  while (n > 0n) { out = B58[Number(n % 58n)] + out; n /= 58n; }
  for (const b of bytes) { if (b !== 0) break; out = "1" + out; }
  return out;
}

export function b58decode(text) {
  if (typeof text !== "string" || !text) throw new X3Error("X3_NEAR_SIGNATURE_FORMAT", "base58 input must be a non-empty string");
  let n = 0n;
  for (const c of text) {
    const i = B58.indexOf(c);
    if (i < 0) throw new X3Error("X3_NEAR_SIGNATURE_FORMAT", `invalid base58 character '${c}'`);
    n = n * 58n + BigInt(i);
  }
  const bytes = [];
  while (n > 0n) { bytes.unshift(Number(n & 0xffn)); n >>= 8n; }
  for (const c of text) { if (c !== "1") break; bytes.unshift(0); }
  return new Uint8Array(bytes);
}

function decodeEd25519(value, length) {
  if (typeof value !== "string" || !value.startsWith("ed25519:")) throw new X3Error("X3_NEAR_SIGNATURE_FORMAT", "expected an ed25519:<base58> value");
  const raw = b58decode(value.slice(8));
  if (raw.length !== length) throw new X3Error("X3_NEAR_SIGNATURE_FORMAT", `expected ${length} bytes, decoded ${raw.length}`);
  return raw;
}

export async function quoteHash(response) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(signedQuotePayload(response)));
  return b58encode(new Uint8Array(digest));
}

/** { valid, reason, hash }. Never throws: anything it cannot check is invalid. */
export async function checkQuoteSignature(response, managerPublicKey = ONE_CLICK_MANAGER_PUBLIC_KEY) {
  let key;
  try {
    const pub = decodeEd25519(managerPublicKey, 32);
    const sig = decodeEd25519(response?.signature, 64);
    const hash = await quoteHash(response);
    try {
      key = await crypto.subtle.importKey("raw", pub, { name: "Ed25519" }, false, ["verify"]);
    } catch {
      return { valid: false, reason: "this browser cannot verify Ed25519 signatures; use a current Chrome, Firefox or Safari", hash: null };
    }
    const ok = await crypto.subtle.verify({ name: "Ed25519" }, key, sig, new TextEncoder().encode(hash));
    return ok ? { valid: true, reason: "verified", hash } : { valid: false, reason: "signature does not verify under the 1Click manager key", hash: null };
  } catch (e) {
    return { valid: false, reason: `unsupported quote format: ${e.message}`, hash: null };
  }
}

// ---------------------------------------------------------------------------------------
// Quote guards (near_intents.verify_quote)
// ---------------------------------------------------------------------------------------

const ECHO_FIELDS = ["swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset", "amount",
  "refundTo", "refundType", "recipient", "recipientType", "deadline"];

function same(field, sent, echoed) {
  if (field === "deadline") {
    try { return parseTimestampMs(sent) === parseTimestampMs(echoed); } catch { return false; }
  }
  if (field === "amount" || field === "slippageTolerance") {
    try { return String(BigInt(String(sent))) === String(BigInt(String(echoed))); } catch { return false; }
  }
  return sent === echoed;
}

const intOf = (v) => { if (!/^\d+$/.test(String(v))) throw new Error(); return BigInt(String(v)); };

/** Scale a decimal USD string to 1e-18 units, exactly. */
function usd(text) {
  const d = decimal(text, "usd");
  if (d.scale > 18) throw new Error("too precise");
  return d.n * pow10(18 - d.scale);
}

function bps2(numerator, denominator) {
  // numerator/denominator in bps, as a 2-decimal display string, rounded half-even like Decimal.quantize.
  const scaled = numerator * 10000n * 100n;
  const q = scaled / denominator;
  const r = scaled % denominator;
  let v = q;
  if (r * 2n > denominator || (r * 2n === denominator && q % 2n === 1n)) v += 1n;
  const s = v.toString().padStart(3, "0");
  return `${s.slice(0, -2)}.${s.slice(-2)}`;
}

/**
 * Every guard, in the adapter's order. Resolves to a list of { guard, ok, ... }.
 * The signature is checked first; if it fails, nothing else in the response is read.
 */
export async function verifyQuote(leg, request, response, { nowMs = Date.now(), managerPublicKey = ONE_CLICK_MANAGER_PUBLIC_KEY } = {}) {
  const results = [];
  const record = (guard, ok, detail = {}) => results.push({ guard, ok: Boolean(ok), ...detail });

  const sig = await checkQuoteSignature(response, managerPublicKey);
  record("quote_signature", sig.valid, { reason: sig.reason, quote_hash: sig.hash, manager_public_key: managerPublicKey });
  if (!sig.valid) return { results, verdict: "invalid_signature" };

  const age = (nowMs - parseTimestampMs(response.timestamp)) / 1000;
  record("quote_freshness", age >= -MAX_CLOCK_SKEW_S && age <= MAX_QUOTE_AGE_S, { age_s: Math.trunc(age), max_age_s: MAX_QUOTE_AGE_S, issued_at: response.timestamp });

  const echoed = response.quoteRequest ?? {};
  const mismatched = ECHO_FIELDS.filter((f) => !same(f, request[f], echoed[f]));
  record("quote_request_echo", mismatched.length === 0, { mismatched });

  const q = response.quote;
  let amountIn, amountOut, minOut;
  try { amountIn = intOf(q.amountIn); amountOut = intOf(q.amountOut); minOut = intOf(q.minAmountOut); } catch {
    throw new X3Error("X3_NEAR_QUOTE_MALFORMED", "quote lacks integer amountIn/amountOut/minAmountOut");
  }
  const amount = BigInt(leg.amount);
  record("amount_in", amountIn === amount, { expected: leg.amount, quoted: amountIn.toString() });
  if (leg.min_receive != null) record("min_receive", minOut >= BigInt(leg.min_receive), { floor: leg.min_receive, guaranteed: minOut.toString() });
  if (amountOut <= 0n || minOut > amountOut) {
    record("slippage", false, { reason: "minAmountOut is above amountOut or amountOut is zero" });
  } else {
    const spread = amountOut - minOut;
    record("slippage", spread * 10000n <= BigInt(leg.slippage_bps + 1) * amountOut, { bound_bps: leg.slippage_bps, quoted_spread_bps: bps2(spread, amountOut) });
  }
  if (leg.fee_cap_bps != null) {
    const cap = BigInt(leg.fee_cap_bps);
    let usdIn = 0n, usdOut = 0n;
    try { usdIn = usd(q.amountInUsd); usdOut = usd(q.amountOutUsd); } catch { usdIn = 0n; }
    if (usdIn <= 0n) record("fees", false, { reason: "quote has no USD price for the input, so the cost cannot be measured" });
    else {
      const cost = usdIn > usdOut ? usdIn - usdOut : 0n;
      record("fees", cost * 10000n <= cap * usdIn, { cap_bps: leg.fee_cap_bps, effective_cost_bps: bps2(cost, usdIn) });
    }
    if (q.refundFee == null) record("refund_fee", false, { reason: "quote states no refundFee, so the refund path's cost is unknown" });
    else {
      let fee;
      try { fee = intOf(q.refundFee); } catch { throw new X3Error("X3_NEAR_QUOTE_MALFORMED", `refundFee is not an integer: '${q.refundFee}'`); }
      record("refund_fee", fee * 10000n <= cap * amount, { cap_bps: leg.fee_cap_bps, refund_fee: fee.toString(), refund_fee_bps: bps2(fee, amount), signed: !request.dry });
    }
  }
  if (!request.dry) {
    record("deposit_address", Boolean(q.depositAddress), { deposit_address: q.depositAddress ?? null });
    if (!q.deadline) record("deadline", false, { reason: "live quote carries no deadline" });
    else {
      const qd = parseTimestampMs(q.deadline);
      const remaining = (qd - nowMs) / 1000;
      const within = qd <= parseTimestampMs(leg.deadline);
      record("deadline", remaining > 0 && remaining >= Number(q.timeEstimate ?? 0) && within, { seconds_remaining: Math.trunc(remaining), within_program_timeout: within });
    }
  }
  const failed = results.filter((r) => !r.ok).map((r) => r.guard);
  return { results, verdict: failed.length ? "refused" : request.dry ? "dry_pass" : "eligible", failed };
}

// ---------------------------------------------------------------------------------------
// Network (browser)
// ---------------------------------------------------------------------------------------

export async function fetchTokens(baseUrl = ONE_CLICK_BASE_URL) {
  const res = await fetch(`${baseUrl}/v0/tokens`, { headers: { Accept: "application/json" } });
  if (!res.ok) throw new X3Error("X3_NEAR_HTTP", `GET /v0/tokens returned ${res.status}`);
  const data = await res.json();
  if (!Array.isArray(data)) throw new X3Error("X3_NEAR_HTTP", "/v0/tokens did not return a list");
  return data;
}

export async function requestQuote(request, baseUrl = ONE_CLICK_BASE_URL) {
  const res = await fetch(`${baseUrl}/v0/quote`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify(request),
  });
  let data = null;
  try { data = await res.json(); } catch { /* reported below */ }
  if (!res.ok) {
    throw new X3Error("X3_NEAR_HTTP", `1Click returned ${res.status}: ${data?.message ?? "no message"}`, { detail: { status: res.status, correlationId: data?.correlationId ?? null } });
  }
  if (!data) throw new X3Error("X3_NEAR_HTTP", "1Click returned no JSON");
  return data;
}

export const NEAR_RPC_URL = "https://free.rpc.fastnear.com";

/** true / false, or null when the RPC cannot be reached from this page. */
export async function nearAccountExists(account, rpcUrl = NEAR_RPC_URL) {
  if (/^([a-f0-9]{64}|0x[a-f0-9]{40})$/.test(account)) return true;
  try {
    const res = await fetch(rpcUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: "x3", method: "query", params: { request_type: "view_account", finality: "final", account_id: account } }),
    });
    const data = await res.json();
    if (data?.result && "amount" in data.result) return true;
    if (data?.error?.cause?.name === "UNKNOWN_ACCOUNT") return false;
    return null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------------------
// Agent mandates (near_mandate.py; the contract's `check_mandate` in x3-guard/src/mandate.rs)
// ---------------------------------------------------------------------------------------

export const MANDATE_WINDOW_MS = 86_400_000;
export const MAX_PAYMENTS_PER_WINDOW = 64;
const MANDATE_MAX_LIST = 16;
const MANDATE_MAX_APPROVERS = 8;
const MANDATE_KEYS = ["owner", "agent", "spend", "per_payment", "per_24h", "refund_to", "expires"];
const mandateList = (text) => text.split(",").map((x) => x.trim().replace(/"/g, "")).filter(Boolean);
const fieldName = (f) => (f.startsWith("allow_") ? f.replace("_", " ") : f);

/** One `mandate` declaration. Strict: a line it does not know is an error. */
export function parseMandate(source) {
  const lines = cleanLines(source);
  if (!lines.length) throw new X3Error("X3_MANDATE_PARSE", "the file is empty");
  const head = /^mandate\s+([A-Za-z_][A-Za-z0-9_-]*)\s*\{$/.exec(lines[0].text);
  if (!head) throw new X3Error("X3_MANDATE_PARSE", "expected `mandate <name> {` as the first line", { line: lines[0].no, field: "mandate" });
  const out = { name: head[1], requires: [], line_of: {} };
  let closed = false;
  for (const ln of lines.slice(1)) {
    const t = ln.text;
    if (closed) throw new X3Error("X3_MANDATE_PARSE", `unexpected '${t}' after the mandate's closing brace`, { line: ln.no, field: "mandate" });
    if (t === "}") { closed = true; continue; }
    const words = t.split(/\s+/);
    let field;
    let value;
    if (words[0] === "require") {
      let req;
      try { req = parseRequire(ln); } catch (e) { throw new X3Error("X3_MANDATE_PARSE", e.message, { line: ln.no, field: "require" }); }
      if (req.kind !== "slippage" && req.kind !== "fees") throw new X3Error("X3_MANDATE_PARSE", `a mandate bounds slippage and fees; \`require ${req.kind}\` belongs in the agent's intent`, { line: ln.no, field: "require" });
      out.requires.push(req);
      continue;
    }
    if (words[0] === "allow" && words.length >= 3 && (words[1] === "recipients" || words[1] === "assets")) {
      field = `allow_${words[1]}`;
      value = mandateList(t.split(/\s+/).slice(2).join(" "));
    } else if (words[0] === "approve") {
      const m = /^approve\s+above\s+(\S+)\s+by\s+(\d+)\s+of\s+(.+)$/.exec(t);
      if (!m) throw new X3Error("X3_MANDATE_PARSE", "expected `approve above <amount> by <n> of <account>, …`", { line: ln.no, field: "approve" });
      field = "approve";
      value = { above: m[1], threshold: Number(m[2]), approvers: mandateList(m[3]) };
    } else if (MANDATE_KEYS.includes(words[0]) && words.length === 2) {
      field = words[0];
      value = words[1].replace(/"/g, "");
    } else {
      throw new X3Error("X3_MANDATE_PARSE", `'${t}' is not a mandate clause`, { line: ln.no, field: "mandate" });
    }
    if (field in out) throw new X3Error("X3_MANDATE_PARSE", `\`${fieldName(field)}\` appears twice`, { line: ln.no, field });
    out[field] = value;
    out.line_of[field] = ln.no;
  }
  if (!closed) throw new X3Error("X3_MANDATE_PARSE", "the mandate is never closed", { line: lines[lines.length - 1].no, field: "mandate" });
  for (const req of ["owner", "agent", "spend", "per_payment", "per_24h", "allow_recipients", "allow_assets", "refund_to", "expires"]) {
    if (!(req in out)) throw new X3Error("X3_MANDATE_PARSE", `a mandate must state \`${fieldName(req)}\``, { field: req });
  }
  return out;
}

function mandateChainAsset(ref, field, line) {
  const m = /^([A-Za-z][A-Za-z0-9_-]*)\.([A-Za-z][A-Za-z0-9_-]*)$/.exec(ref);
  if (!m) throw new X3Error("X3_MANDATE_ASSET", `'${ref}' is not a chain.asset reference`, { field, line });
  const chain = normalizeChain(m[1]);
  if (!CHAIN_TO_BLOCKCHAIN[chain]) throw new X3Error("X3_MANDATE_ASSET", `'${ref}': chain has no NEAR Intents mapping`, { field, line });
  return { chain, asset: m[2] };
}

function mandateAccount(value, field, line) {
  if (!isValidNearAccount(value)) throw new X3Error("X3_MANDATE_ACCOUNT", `'${value}' is not a NEAR account`, { field, line });
  return value;
}

export function parseExpiryMs(text, line = null) {
  const m = /^(\d+)(ms|s|m|h|d)$/.exec(String(text).trim());
  if (!m || Number(m[1]) === 0) throw new X3Error("X3_MANDATE_EXPIRES", `expires '${text}' must be a positive <n>ms|s|m|h|d duration`, { field: "expires", line });
  return Number(m[1]) * { ms: 1, s: 1000, m: 60000, h: 3600000, d: 86400000 }[m[2]];
}

const distinct = (xs) => new Set(xs).size === xs.length;

/** The `create_mandate` call: who sends it, and its `agent` and `terms`. */
export function lowerMandate(parsed, tokens, nowMs, sourceSha256) {
  const at = parsed.line_of;
  const owner = mandateAccount(parsed.owner, "owner", at.owner);
  const agent = mandateAccount(parsed.agent, "agent", at.agent);
  if (owner === agent) throw new X3Error("X3_MANDATE_ACCOUNT", "the agent must be a different account from the owner", { field: "agent", line: at.agent });
  const spend = mandateChainAsset(parsed.spend, "spend", at.spend);
  const spendId = resolveAsset(tokens, spend.chain, spend.asset).assetId;
  const destinations = parsed.allow_assets.map((r) => mandateChainAsset(r, "allow assets", at.allow_assets));
  const destinationIds = destinations.map((d) => resolveAsset(tokens, d.chain, d.asset).assetId);
  const perPayment = baseUnits(parsed.per_payment, "per_payment");
  const per24h = baseUnits(parsed.per_24h, "per_24h");
  if (perPayment > per24h) throw new X3Error("X3_MANDATE_LIMIT", "per_payment must not exceed per_24h", { field: "per_payment", line: at.per_payment });
  const recipients = parsed.allow_recipients;
  const chains = [...new Set(destinations.map((d) => d.chain))].sort();
  for (const r of recipients) {
    if (r === "sender" || !chains.some((c) => isValidReceiver(c, r))) throw new X3Error("X3_MANDATE_RECIPIENT", `'${r}' is not an address on ${chains.join(", ")}`, { field: "allow recipients", line: at.allow_recipients });
  }
  const refundTo = parsed.refund_to;
  if (refundTo === "sender" || !isValidReceiver(spend.chain, refundTo)) throw new X3Error("X3_MANDATE_REFUND_TO", `'${refundTo}' is not a ${spend.chain} address`, { field: "refund_to", line: at.refund_to });
  for (const [field, items] of [["allow recipients", recipients], ["allow assets", destinationIds]]) {
    if (!items.length || items.length > MANDATE_MAX_LIST || !distinct(items)) throw new X3Error("X3_MANDATE_LIST", `${field} must list 1..${MANDATE_MAX_LIST} distinct entries`, { field });
  }
  const bounds = {};
  for (const kind of ["slippage", "fees"]) {
    const found = parsed.requires.filter((r) => r.kind === kind).map((r) => upperBoundBps(r, `require ${kind}`));
    if (!found.length) throw new X3Error("X3_MANDATE_BOUND", `a mandate must bound \`require ${kind} <= …\`: the agent would otherwise choose it`, { field: "require" });
    bounds[kind] = Math.min(...found);
  }
  let approvalAbove = null;
  let approvers = [];
  let threshold = 0;
  if (parsed.approve) {
    const a = parsed.approve;
    approvalAbove = baseUnits(a.above, "approve above");
    approvers = a.approvers.map((x) => mandateAccount(x, "approve", at.approve));
    threshold = a.threshold;
    const bad = (msg) => new X3Error("X3_MANDATE_APPROVAL", msg, { field: "approve", line: at.approve });
    if (approvalAbove >= perPayment) throw bad("`approve above` must be below per_payment, or no payment could need it");
    if (!approvers.length || approvers.length > MANDATE_MAX_APPROVERS || !distinct(approvers)) throw bad(`approvers must be 1..${MANDATE_MAX_APPROVERS} distinct accounts`);
    if (approvers.includes(agent)) throw bad("the agent cannot approve its own payments");
    if (threshold < 1 || threshold > approvers.length) throw bad("the threshold must be 1..the number of approvers");
  }
  const expiresAt = nowMs + parseExpiryMs(parsed.expires, at.expires);
  return {
    name: parsed.name,
    owner,
    agent,
    terms: {
      program_sha256: sourceSha256,
      asset: spendId,
      per_payment_cap: perPayment.toString(),
      daily_cap: per24h.toString(),
      destination_assets: destinationIds,
      recipients,
      refund_to: refundTo,
      max_slippage_bps: bounds.slippage,
      max_fee_bps: bounds.fees,
      approval_above: approvalAbove === null ? null : approvalAbove.toString(),
      approvers,
      approval_threshold: threshold,
      expires_at_ms: expiresAt,
    },
    expires: formatTimestamp(expiresAt),
  };
}

export async function sha256Hex(text) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** x3-guard's `Policy` for a lowered leg and its quote request. */
export function policyFor(leg, request, programSha256) {
  return {
    program_sha256: programSha256,
    origin_asset: request.originAsset,
    destination_asset: request.destinationAsset,
    amount: String(leg.amount),
    min_receive: leg.min_receive == null ? null : String(leg.min_receive),
    slippage_bps: leg.slippage_bps,
    fee_cap_bps: leg.fee_cap_bps,
    recipient: leg.recipient,
    refund_to: leg.refund_to,
    deadline_ms: parseTimestampMs(leg.deadline),
  };
}

function spentInWindow(spends, nowMs) {
  const since = Math.max(nowMs - MANDATE_WINDOW_MS, 0);
  const inside = spends.filter((s) => Number(s.at_ms) > since);
  return [inside.reduce((sum, s) => sum + BigInt(s.amount), 0n), inside.length];
}

/** The contract's `check_mandate`, check for check, in the same order. */
export function checkMandate(mandate, policy, nowMs) {
  const t = mandate.terms;
  const out = [];
  const record = (guard, ok, detail) => out.push({ guard, ok: Boolean(ok), detail });
  const revoked = Boolean(mandate.revoked);
  record("mandate_active", !revoked && nowMs < Number(t.expires_at_ms), `version ${mandate.version ?? "None"}, revoked ${revoked}, expires ${t.expires_at_ms}, now ${nowMs}`);
  record("mandate_asset", policy.origin_asset === t.asset, `spends ${policy.origin_asset}, mandate allows ${t.asset}`);
  record("mandate_destination_asset", t.destination_assets.includes(policy.destination_asset), `${policy.destination_asset}, allowed ${t.destination_assets.join(", ")}`);
  record("mandate_recipient", t.recipients.includes(policy.recipient), `${policy.recipient}, allowed ${t.recipients.join(", ")}`);
  record("mandate_refund_to", policy.refund_to === t.refund_to, `refunds to ${policy.refund_to}, mandate pins ${t.refund_to}`);
  const amount = BigInt(policy.amount);
  record("mandate_per_payment", amount <= BigInt(t.per_payment_cap), `${amount} of at most ${t.per_payment_cap}`);
  const [spent, count] = spentInWindow(mandate.spends ?? [], nowMs);
  record("mandate_daily", spent + amount <= BigInt(t.daily_cap) && count < MAX_PAYMENTS_PER_WINDOW, `${spent} spent in ${count} payments over 24h, plus ${amount}, cap ${t.daily_cap}`);
  record("mandate_slippage", policy.slippage_bps <= t.max_slippage_bps, `${policy.slippage_bps} bps of at most ${t.max_slippage_bps}`);
  record("mandate_fees", policy.fee_cap_bps != null && policy.fee_cap_bps <= t.max_fee_bps, `fee cap ${policy.fee_cap_bps} bps, mandate requires at most ${t.max_fee_bps}`);
  record("mandate_deadline", policy.deadline_ms <= Number(t.expires_at_ms), `intent deadline ${policy.deadline_ms}, mandate expires ${t.expires_at_ms}`);
  return out;
}

export function approvalsNeeded(mandate, amount) {
  const above = mandate.terms.approval_above;
  return above != null && BigInt(amount) > BigInt(above) ? mandate.terms.approval_threshold : 0;
}

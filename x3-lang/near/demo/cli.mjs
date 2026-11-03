#!/usr/bin/env node
// Runs x3guard.mjs on files, for `x3-lang/tests/test_demo_parity.py`. Output is JSON on stdout.
//
//   node cli.mjs lower  <program.x3> <tokens.json> <now-ms>
//   node cli.mjs verify <response.json> [manager-key]
//   node cli.mjs guards <case.json>      ({ leg, request, response, now_ms, manager_public_key })
import { readFileSync } from "node:fs";
import * as x3 from "./x3guard.mjs";

const [cmd, ...args] = process.argv.slice(2);
const read = (p) => readFileSync(p, "utf8");
const out = (v) => process.stdout.write(JSON.stringify(v) + "\n");

try {
  if (cmd === "lower") {
    const [program, tokens, now] = args;
    const intent = x3.parse(read(program));
    const errors = x3.typecheck(intent);
    if (errors.length) out({ error: { code: "X3_NEAR_TYPECHECK", detail: errors.map((e) => e.code) } });
    else {
      const leg = x3.lower(intent, Number(now));
      out({ leg, request: x3.buildQuoteRequest(leg, JSON.parse(read(tokens)), { dry: true }) });
    }
  } else if (cmd === "verify") {
    const response = JSON.parse(read(args[0]));
    const check = await x3.checkQuoteSignature(response, args[1] ?? x3.ONE_CLICK_MANAGER_PUBLIC_KEY);
    out({ ...check, payload: (() => { try { return x3.signedQuotePayload(response); } catch { return null; } })() });
  } else if (cmd === "guards") {
    const c = JSON.parse(read(args[0]));
    out(await x3.verifyQuote(c.leg, c.request, c.response, { nowMs: c.now_ms, managerPublicKey: c.manager_public_key }));
  } else {
    throw new Error(`unknown command ${cmd}`);
  }
} catch (e) {
  out({ error: { code: e.code ?? "X3_DEMO_ERROR", message: e.message, line: e.line ?? null } });
}

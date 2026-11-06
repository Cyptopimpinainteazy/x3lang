// node --test x3-lang/near/demo/test
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import * as judge from "../judge/judge.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..", "..");
const DATA = JSON.parse(readFileSync(join(here, "..", "judge", "scenarios.json"), "utf8"));
const read = (file) => readFileSync(join(repo, file), "utf8");

for (const s of DATA.scenarios) {
  test(`${s.id}: the browser reaches Python's decision through the same guards`, async () => {
    const result = await judge.evaluate(DATA, s);
    assert.equal(result.decision, s.python.decision);
    assert.deepEqual(result.denied_guards, s.python.denied_guards);
    assert.equal(result.approvals_needed, s.python.approvals_needed);
    assert.ok(judge.agrees(s, result));
  });

  test(`${s.id}: the program is the checked-in example plus its stated edits`, () => {
    let source = read(s.intent.file);
    for (const e of s.intent.edits) source = source.replace(e.from, e.to);
    assert.equal(source, s.intent.source);
  });
}

test("the mandate and token list are the checked-in files", () => {
  assert.equal(DATA.mandate.source, read(DATA.mandate.file));
  assert.deepEqual(DATA.tokens.records, JSON.parse(read(DATA.tokens.file)));
});

test("every forbidden action in group B is refused, by the contract when the checker can't see it", async () => {
  for (const s of DATA.scenarios.filter((x) => x.group === "B")) {
    const shown = judge.shownDecision(s, await judge.evaluate(DATA, s));
    assert.ok(["denied_by_policy", "contract_only"].includes(shown), `${s.id} shows ${shown}`);
    if (shown === "contract_only") assert.ok(s.contract_tests.length > 0, `${s.id} cites no contract test`);
  }
});

test("a contract-only case never shows as passed, even though the local check passes it", async () => {
  const s = DATA.scenarios.find((x) => x.id === "nonce-reuse");
  const result = await judge.evaluate(DATA, s);
  assert.equal(result.decision, "preflight_passed");
  assert.equal(judge.shownDecision(s, result), "contract_only");
});

test("agreement fails when Python and the browser differ on one guard", async () => {
  const s = DATA.scenarios.find((x) => x.id === "authorized");
  const result = await judge.evaluate(DATA, s);
  const tampered = structuredClone(s);
  tampered.python.checks[3].ok = false;
  assert.equal(judge.agrees(tampered, result), false);
});

test("a mandate owned by someone else is refused before any term is checked", async () => {
  const s = structuredClone(DATA.scenarios[0]);
  s.intent.source = s.intent.source.replace(/^(\s*)under mandate x3-owner\.near$/m, "$1under mandate someone-else.near");
  assert.match(s.intent.source, /^\s*under mandate someone-else\.near$/m);
  const result = await judge.evaluate(DATA, s);
  assert.equal(result.decision, "denied_by_policy");
  assert.deepEqual(result.denied_guards, ["mandate_owner"]);
  assert.deepEqual(result.checks, []);
});

test("prompt-injection-recipient: the browser refuses the injected recipient and agrees with Python", async () => {
  const s = DATA.scenarios.find((x) => x.id === "prompt-injection-recipient");
  assert.ok(s, "prompt-injection-recipient scenario is missing from scenarios.json");
  assert.match(s.story.toLowerCase(), /prompt/);
  const result = await judge.evaluate(DATA, s);
  assert.equal(result.decision, "denied_by_policy");
  assert.deepEqual(result.denied_guards, ["mandate_recipient"]);
  assert.ok(judge.agrees(s, result));
});

test("prompt-injection-recipient and wrong-recipient evaluate the same checked-in DENIED payload", () => {
  const wrong = DATA.scenarios.find((x) => x.id === "wrong-recipient");
  const injected = DATA.scenarios.find((x) => x.id === "prompt-injection-recipient");
  assert.ok(wrong, "wrong-recipient scenario is missing from scenarios.json");
  assert.ok(injected, "prompt-injection-recipient scenario is missing from scenarios.json");
  assert.equal(wrong.intent.file, "x3-lang/examples/near_intents_agent_denied.x3");
  assert.equal(injected.intent.file, "x3-lang/examples/near_intents_agent_denied.x3");
  assert.equal(injected.intent.source, wrong.intent.source);
  assert.equal(injected.intent.sha256, wrong.intent.sha256);
  assert.equal(injected.python.decision, wrong.python.decision);
  assert.deepEqual(injected.python.denied_guards, wrong.python.denied_guards);
});

test("only the local policy level is claimed as proven", () => {
  assert.deepEqual(judge.EVIDENCE_LADDER.filter((l) => l.proven).map((l) => l.level), ["local_policy"]);
});

test("judge evidence panel labels valid test-key signed-quote refusal as local, not on-chain", () => {
  const html = read("x3-lang/near/demo/judge.html");
  assert.match(html, /id === "guard_signed_refusal"/);
  assert.match(html, /TEST-ONLY key/);
  assert.match(html, /not a live 1Click signature or on-chain receipt/);
  assert.match(html, /NOT PROVEN BY THIS RECORD/);
});

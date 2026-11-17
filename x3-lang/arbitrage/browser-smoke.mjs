// Optional real-browser gate. Reuses an existing Playwright/Chromium installation.
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.X3_PLAYWRIGHT_MODULE || "playwright");
const temporary = await mkdtemp(path.join(tmpdir(), "x3-arbitrage-browser-"));
const artifacts =
  process.env.X3_ARTIFACT_DIR || path.join(temporary, "screenshots");
await mkdir(artifacts, { recursive: true });
const server = spawn(
  process.env.X3_PYTHON || "python3",
  [
    "-m",
    "arbitrage",
    "serve",
    "--mode",
    "demo",
    "--port",
    "0",
    "--database",
    path.join(temporary, "paper.sqlite"),
  ],
  {
    env: { ...process.env, PYTHONPATH: "x3-lang" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let browser;
try {
  const base = await new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error("Scanner startup timed out")),
      10000,
    );
    server.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    server.once("exit", () => {
      clearTimeout(timeout);
      reject(new Error("Scanner exited during startup"));
    });
    server.stdout.on("data", (chunk) => {
      const match = chunk.toString().match(/http:\/\/127\.0\.0\.1:\d+/);
      if (match) {
        clearTimeout(timeout);
        resolve(match[0]);
      }
    });
  });
  browser = await chromium.launch({
    executablePath: process.env.X3_CHROMIUM || undefined,
    headless: true,
  });
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1100 },
  });
  const errors = [];
  let expectedCompileErrors = 0;
  const extraChecks = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") {
      if (
        message.location().url.endsWith("/api/strategy/compile") &&
        message.text().includes("400 (Bad Request)")
      )
        expectedCompileErrors += 1;
      else errors.push(message.text());
    }
  });
  await page.goto(base);
  await page.getByText("3 of 3 routes", { exact: true }).waitFor();
  assert.equal(await page.locator("#routes tr").count(), 3);
  if (process.env.X3_POOL_SMOKE === "1") {
    await page.locator("#load-pools").click();
    await page.waitForFunction(() => !document.getElementById("load-pools").disabled, {timeout: 60000});
    assert.ok(await page.locator(".pool-card").count() > 0, "real public pools must load");
    assert.match(await page.locator("#pool-status").textContent(), /discovery only/);
    const poolCount = await page.locator(".pool-card").count();
    const poolLinks = await page.locator(".pool-card a").evaluateAll(links => links.map(link => link.href));
    assert.ok(poolLinks.every(link => ["dexscreener.com", "app.rhea.finance"].includes(new URL(link).hostname)));
    await page.locator("#pool-chain").selectOption("solana");
    assert.ok(await page.locator(".pool-card").count() > 0);
    assert.match(await page.locator("#pool-list").textContent(), /Solana/);
    await page.locator("#pool-chain").selectOption("");
    await page.locator("#pool-search").fill("this-token-does-not-exist-x3");
    assert.equal(await page.locator(".pool-card").count(), 0);
    await page.locator("#pool-search").fill("");
    assert.equal(await page.locator(".pool-card").count(), poolCount);
    await page.screenshot({path: path.join(artifacts, "liquidity-pools.png"), fullPage: true});
    extraChecks.push(`real read-only pool discovery (${poolCount} pools), filtering and safe source links`);
  }
  if (process.env.X3_CATALOGUE_SMOKE === "1") {
    await page.locator("#load-tokens").click();
    await page.waitForFunction(() =>
      /\d+ provider-listed assets/.test(
        document.getElementById("token-summary").textContent,
      ),
    );
    assert.ok((await page.locator(".token-card").count()) >= 12);
    await page.locator("#token-chain").selectOption("bsc");
    await page.locator("#token-search").fill("USDC");
    assert.match(
      await page.locator("#token-list").textContent(),
      /18 decimals/,
    );
    const catalogue = await page.evaluate(
      async () => (await (await fetch("/api/state")).json()).catalogue,
    );
    const startToken = catalogue.assets.find(
      (asset) => asset.network === "bsc" && asset.symbol === "USDC",
    );
    await page
      .getByRole("button", { name: "Start with USDC on BSC", exact: true })
      .click();
    assert.equal(
      await page.locator("#route-origin").inputValue(),
      startToken.asset_id,
    );
    await page.locator("#token-chain").selectOption("near");
    await page.locator("#token-search").fill("USDT");
    const viaToken = catalogue.assets.find(
      (asset) => asset.network === "near" && asset.symbol === "USDT",
    );
    await page
      .getByRole("button", { name: "Scan via USDT on NEAR", exact: true })
      .click();
    assert.equal(
      await page.locator("#route-destination").inputValue(),
      viaToken.asset_id,
    );
    assert.match(
      await page.locator("#selected-pair").textContent(),
      /USDC on BSC.*USDT on NEAR/,
    );
    await page.locator("#clear-selected").click();
    extraChecks.push(
      "exact-ID token-card clicks populate the opportunity scanner pair",
    );
    await page.locator("#token-search").fill("");
    await page.locator("#token-chain").selectOption("");
    extraChecks.push(
      "real public token catalogue, chain/search filters and BSC USDC18",
    );
  }
  await page.screenshot({
    path: path.join(artifacts, "dashboard-desktop.png"),
    fullPage: true,
  });
  await page.locator("#eligible").check();
  assert.equal(await page.locator("#routes tr").count(), 1);
  await page.locator("#routes button").first().click();
  await page.locator("#paper").click();
  await page
    .getByText("projected — simulation only; no transactions submitted.", {
      exact: true,
    })
    .waitFor();
  await page.locator("#auto-paper").click();
  await page
    .getByText("COMPLETED · 2/2 legs · no real funds moved.", { exact: true })
    .waitFor();
  assert.match(
    await page.locator("#auto-source").textContent(),
    /under mandate paper-sol-owner.near/,
  );
  await page
    .locator("#refund")
    .fill("4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD");
  await page.locator("#recipient").fill("alice.near");
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download .x3 program" }).click();
  const file = await download;
  assert.match(file.suggestedFilename(), /\.x3$/);
  await page.screenshot({
    path: path.join(artifacts, "route-inspector.png"),
    fullPage: true,
  });
  await page.locator("#close-detail").click();
  await page
    .getByRole("button", { name: ".x3 policy", exact: true })
    .first()
    .click();
  await page.waitForFunction(() =>
    document
      .getElementById("strategy-mandate-0")
      .value.includes("mandate automatic_paper_leg_0"),
  );
  const originalMandate = await page
    .locator("#strategy-mandate-0")
    .inputValue();
  assert.equal(await page.locator("#run-strategy").isDisabled(), true);
  await page
    .locator("#strategy-mandate-0")
    .fill(originalMandate.replace("per_payment 200000000", "per_payment 1"));
  const refusedCompile = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/strategy/compile") &&
      response.status() === 400,
  );
  await page.locator("#compile-strategy").click();
  assert.equal((await refusedCompile).status(), 400);
  await page.waitForFunction(() =>
    document
      .getElementById("strategy-result")
      .textContent.includes("mandate_per_payment"),
  );
  assert.equal(await page.locator("#run-strategy").isDisabled(), true);
  await page.locator("#strategy-mandate-0").fill(originalMandate);
  await page.locator("#compile-strategy").click();
  await page
    .getByText(
      "Compiled and validated both .x3 legs. Ready for paper execution; no wallet authorization.",
      { exact: true },
    )
    .waitFor();
  await page.locator("#strategy-profit").fill("0.75");
  assert.equal(await page.locator("#run-strategy").isDisabled(), true);
  await page.locator("#compile-strategy").click();
  await page
    .getByText(
      "Compiled and validated both .x3 legs. Ready for paper execution; no wallet authorization.",
      { exact: true },
    )
    .waitFor();
  let releaseCompilation, compilationReady;
  const compilationGate = new Promise((resolve) => {
    releaseCompilation = resolve;
  });
  const compilationSeen = new Promise((resolve) => {
    compilationReady = resolve;
  });
  await page.route("**/api/strategy/compile", async (route) => {
    const response = await route.fetch();
    compilationReady();
    await compilationGate;
    await route.fulfill({ response });
  });
  await page.locator("#compile-strategy").click();
  await compilationSeen;
  await page
    .locator("#strategy-mandate-0")
    .fill(originalMandate.replace("per_payment 200000000", "per_payment 1"));
  releaseCompilation();
  await page
    .getByText(
      "Draft changed during compilation. Compile the current source again.",
      { exact: true },
    )
    .waitFor();
  assert.equal(await page.locator("#run-strategy").isDisabled(), true);
  await page.unroute("**/api/strategy/compile");
  await page.locator("#strategy-mandate-0").fill(originalMandate);
  await page.locator("#compile-strategy").click();
  await page
    .getByText(
      "Compiled and validated both .x3 legs. Ready for paper execution; no wallet authorization.",
      { exact: true },
    )
    .waitFor();
  await page.locator("#run-strategy").click();
  await page
    .getByText(
      "COMPLETED · 2/2 legs under your .x3 mandates · no real funds moved.",
      { exact: true },
    )
    .waitFor();
  await page.screenshot({
    path: path.join(artifacts, "x3-workbench.png"),
    fullPage: true,
  });
  await page.locator("#close-strategy").click();
  extraChecks.push(
    "editable .x3 mandate cap refusal, compile invalidation, stale-response rejection and full paper execution",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: path.join(artifacts, "dashboard-mobile.png"),
    fullPage: true,
  });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await page
    .getByRole("button", { name: ".x3 policy", exact: true })
    .first()
    .click();
  await page.waitForFunction(() =>
    document
      .getElementById("strategy-mandate-0")
      .value.includes("mandate automatic_paper_leg_0"),
  );
  const mobileEditor = await page.locator("#strategy-dialog").boundingBox();
  assert.ok(mobileEditor.x >= 0 && mobileEditor.width <= 390);
  await page.screenshot({
    path: path.join(artifacts, "x3-workbench-mobile.png"),
    fullPage: true,
  });
  await page.locator("#close-strategy").click();
  assert.deepEqual(errors, []);
  assert.equal(
    expectedCompileErrors,
    1,
    "only the intentional low-cap compiler refusal may log HTTP400",
  );
  console.log(
    JSON.stringify({
      browser: "chromium",
      checks: [
        "desktop route table",
        "eligibility filter",
        "route inspector",
        "SQLite paper action",
        "full automatic two-leg .x3-mandated paper cycle",
        "compiled x3 download",
        "390px responsive layout",
        "no unexpected page or console errors",
        ...extraChecks,
      ],
      result: "pass",
    }),
  );
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
  await rm(temporary, { recursive: true, force: true });
}

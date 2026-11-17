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
  const extraChecks = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(base);
  await page.getByText("3 of 3 routes", { exact: true }).waitFor();
  assert.equal(await page.locator("#routes tr").count(), 3);
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
  assert.deepEqual(errors, []);
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
        "no page or console errors",
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

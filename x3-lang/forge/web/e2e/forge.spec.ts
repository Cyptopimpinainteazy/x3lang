import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";

test("forge → prove → birth → export/import → revoke", async ({ page }) => {
  page.on("pageerror", (e) => console.log("pageerror:", e.message));
  await page.goto("/");
  await expect(page.getByText("X3 Agent Forge").first()).toBeVisible();

  // Claim High Command with a browser-held key.
  await page.getByRole("button", { name: /Birth/ }).first().click();
  await page.getByLabel("Setup token").fill("e2e-setup-token");
  await page.getByRole("button", { name: "Claim" }).click();
  await expect(page.getByText("High Command · this browser")).toBeVisible();

  // Origin: the repository's example accounts.
  await page.getByRole("button", { name: /^1\s*Origin/ }).click();
  await page.getByRole("button", { name: "Load example" }).click();
  // No connection yet: the build is refused, and says why.
  await expect(page.getByText(/plug in at least one connection/).first()).toBeVisible();
  await expect(page.getByText("X3_FORGE_NAME")).toHaveCount(0);

  await page.getByRole("button", { name: /Skill Tree/ }).first().click();
  await page.getByRole("button", { name: /Armory Access/ }).click();
  await expect(page.getByLabel(/Max per payment/)).toBeVisible();
  await page.screenshot({ path: "e2e-output/skills.png", fullPage: true });

  // Comms Rig: plug into the Forge mesh.
  await page.getByRole("button", { name: /Comms Rig/ }).first().click();
  const mesh = page.locator("div.rounded-lg", { hasText: "Forge agent mesh" }).last();
  await mesh.getByLabel(/Peer agent ids/).fill("gantry.x3-owner.near");
  await mesh.getByRole("button", { name: "status", exact: true }).click();
  await mesh.getByLabel("Messages per minute").fill("5");
  await mesh.getByRole("button", { name: "Plug in" }).click();
  await expect(mesh.getByText("connected")).toBeVisible();
  await expect(page.getByText("compiled", { exact: true })).toBeVisible();

  // Compile chamber shows the real mandate.
  await page.getByRole("button", { name: /Compile Chamber/ }).click();
  await expect(page.getByText("mandate forge_vexa {")).toBeVisible();
  await expect(page.getByText("approve above 30000000 by 1 of x3-owner.near")).toBeVisible();

  // Proving Ground.
  await page.getByRole("button", { name: /Proving Ground/ }).first().click();
  await page.getByRole("button", { name: "Run attacks" }).click();
  await expect(page.getByText("All probes held")).toBeVisible();
  await expect(page.getByText("mandate_recipient").first()).toBeVisible();
  await page.screenshot({ path: "e2e-output/proving.png", fullPage: true });

  // Birth.
  await page.getByRole("button", { name: /Birth/ }).first().click();
  await page.getByRole("button", { name: "Prepare birth" }).click();
  await page.getByRole("button", { name: /Sign with High Command/ }).click();
  await page.waitForURL("**/agents/vexa.x3-owner.near");
  await expect(page.getByText("active", { exact: true }).first()).toBeVisible();
  await page.waitForTimeout(1500);
  await page.screenshot({ path: "e2e-output/profile.png", fullPage: true });

  // Exports.
  const [card] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "card.png" }).click()]);
  await card.saveAs("e2e-output/card.png");
  const [glb] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "creature.glb" }).click()]);
  await glb.saveAs("e2e-output/creature.glb");
  expect(readFileSync("e2e-output/creature.glb").subarray(0, 4).toString()).toBe("glTF");
  const [dl] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "manifest.json" }).click()]);
  await dl.saveAs("e2e-output/manifest.json");

  // Import reproduces the same character.
  await page.goto("/import");
  await page.locator("input[type=file]").setInputFiles("e2e-output/manifest.json");
  await expect(page.getByText("identical")).toBeVisible();

  // Revoke → Scrapyard.
  await page.goto("/agents/vexa.x3-owner.near");
  await page.getByLabel("Reason").fill("end of demo");
  await page.getByRole("button", { name: "Revoke", exact: true }).click();
  await expect(page.getByText("Scrapyard").first()).toBeVisible();
  await expect(page.getByText("revoke_mandate").first()).toBeVisible();
  await page.waitForTimeout(1200);
  await page.screenshot({ path: "e2e-output/scrapyard.png", fullPage: true });
  await page.goto("/roster");
  await expect(page.getByText("Scrapyard · 1")).toBeVisible();
  await page.screenshot({ path: "e2e-output/roster.png", fullPage: true });
});

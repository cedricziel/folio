import { createHmac } from "node:crypto";
import { expect, test } from "@playwright/test";
import { TourInstance, tour } from "./tour";

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

function currentCode(secret: string) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const bits = [...secret].map((c) => alphabet.indexOf(c).toString(2).padStart(5, "0")).join("");
  const key = Buffer.from(bits.match(/.{8}/g)!.map((byte) => parseInt(byte, 2)));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30_000)));
  const digest = createHmac("sha1", key).update(counter).digest();
  const offset = digest[digest.length - 1] & 15;
  return ((digest.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).toString().padStart(6, "0");
}

test("acknowledged recovery codes leave no Turbo snapshot", async ({ browser }) => {
  await instance.ready(browser, { seeded: false });
  const context = await browser.newContext({ baseURL: instance.baseURL });
  const page = await context.newPage();
  try {
    await tour.login(page);
    await page.goto("/settings/security");
    await page.getByRole("button", { name: "Set up authenticator app", exact: true }).click();
    const secret = (await page.locator(".secret-key").innerText()).replace(/\s/g, "");
    await page.getByLabel("Code", { exact: true }).fill(currentCode(secret));
    const response = await Promise.all([
      page.waitForResponse((r) => r.url().endsWith("/settings/security/totp/confirm") && r.request().method() === "POST"),
      page.getByRole("button", { name: "Turn on", exact: true }).click(),
    ]).then(([response]) => response);
    expect(response.headers()["cache-control"]).toBe("no-store");
    await expect(page.locator(".recovery-codes li")).toHaveCount(10);
    const code = await page.locator(".recovery-codes li").first().innerText();
    await page.getByRole("link", { name: "I have saved them", exact: true }).click();
    await expect(page.locator(".recovery-codes")).toHaveCount(0);
    const retained = await page.evaluate(async () => {
      const module = await import("/static/vendor/turbo.js");
      const snapshot = module.session.view.snapshotCache.get(new URL("/settings/security/totp/confirm", location.origin));
      return !!snapshot?.element.querySelector(".recovery-codes");
    });
    expect(retained).toBe(false);
    await page.getByRole("link", { name: "Back to settings", exact: true }).click();
    await expect(page).toHaveURL(instance.baseURL + "/settings");
    await page.goBack();
    await expect(page).toHaveURL(instance.baseURL + "/settings/security");
    await expect(page.getByRole("heading", { name: "Account and security", exact: true })).toBeVisible();
    await expect(page.locator("body")).not.toContainText(code);
    await page.goForward();
    await expect(page).toHaveURL(instance.baseURL + "/settings");
    await expect(page.locator("body")).not.toContainText(code);
    await page.goto("/settings/security");
    await expect(page.getByRole("heading", { name: "Account and security", exact: true })).toBeVisible();
    await page.getByRole("link", { name: "Back to settings", exact: true }).focus();
    await expect(page.getByRole("link", { name: "Back to settings", exact: true })).toBeFocused();
  } finally {
    await context.close();
  }
});

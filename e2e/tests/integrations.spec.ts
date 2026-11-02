import { expect, test } from "@playwright/test";
import { TourInstance, tour } from "./tour";

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

for (const width of [1280, 390]) {
  test(`integration shortcuts work at ${width}px`, async ({ browser }) => {
    await instance.ready(browser, { seeded: false });
    const page = await browser.newPage({ baseURL: instance.baseURL, viewport: { width, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
    try {
      await tour.login(page);
      if (width < 600) await page.getByRole("button", { name: "Open menu", exact: true }).click();
      await expect(page.getByRole("link", { name: "API docs", exact: true })).toHaveAttribute("href", "/api/docs");
      await page.getByRole("link", { name: "MCP server", exact: true }).click();
      await expect(page).toHaveURL(instance.baseURL + "/settings/integrations");
      await expect(page.getByRole("heading", { name: "Integrations", exact: true })).toBeVisible();
      await expect(page.getByLabel("MCP server URL")).toHaveValue(instance.baseURL + "/mcp");
      await page.getByRole("button", { name: "Copy URL", exact: true }).click();
      expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(instance.baseURL + "/mcp");
    } finally { await page.close(); }
  });
}

test("create, copy, reload and revoke a personal token", async ({ browser }) => {
  await instance.ready(browser, { seeded: false });
  const context = await browser.newContext({ baseURL: instance.baseURL, permissions: ["clipboard-read", "clipboard-write"] });
  const page = await context.newPage();
  try {
    await tour.login(page);
    await page.goto("/settings/integrations");
    await expect(page.getByLabel("Read documents", { exact: true })).toBeChecked();
    await expect(page.getByLabel("Write documents", { exact: true })).not.toBeChecked();
    await expect(page.getByLabel("Expires in days")).toHaveValue("30");
    const name = `integration-${Date.now()}`;
    await page.getByLabel("Token name").fill(name);
    await page.getByRole("button", { name: "Create token", exact: true }).click();
    await expect(page.getByLabel("New token", { exact: true })).toBeVisible();
    const secret = await page.getByLabel("New token", { exact: true }).inputValue();
    expect(secret).toMatch(/^folio_pat_/);
    await page.getByRole("button", { name: "Copy token", exact: true }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(secret);
    const row = page.getByRole("listitem").filter({ has: page.getByText(name, { exact: true }) });
    await expect(row).toContainText("documents:read");
    await page.reload();
    await expect(page.getByLabel("New token", { exact: true })).toBeHidden();
    expect(await page.content()).not.toContain(secret);
    expect(await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage } }))).not.toContain(secret);
    await row.getByRole("button", { name: "Revoke", exact: true }).click();
    await expect(row).toContainText("Revoked");
    const response = await page.request.get("/api/documents", { headers: { Authorization: `Bearer ${secret}` } });
    expect(response.status()).toBe(401);
  } finally { await context.close(); }
});

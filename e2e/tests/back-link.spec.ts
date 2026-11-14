import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

test.describe("document back link", () => {
  test.describe.configure({ mode: "serial" });
  const title = uniqueTitle("back-link");

  test("returns to the all-documents list it was opened from", async ({ page }) => {
    await uploadViaUi(page, title);
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();

    const back = page.getByRole("link", { name: /Back to All documents/ });
    await expect(back).toBeVisible();
    await back.click();

    await expect(page).toHaveURL(/\/\?view=all$/);
    await expect(page.getByRole("link", { name: new RegExp(title) })).toBeVisible();
    await expect(page.getByRole("heading", { name: "All documents", level: 1 })).toBeVisible();
  });

  test("falls back to the inbox when opened directly", async ({ page }) => {
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/\/documents\//);
    const direct = page.url().split("?")[0];
    await page.goto(direct);
    const back = page.getByRole("link", { name: /Back to Inbox/ });
    await expect(back).toBeVisible();
    await back.click();
    await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
  });

  test("ignores an external return target", async ({ page }) => {
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/\/documents\//);
    const direct = page.url().split("?")[0];
    for (const bad of ["https://evil.example/", "//evil.example/", "javascript:alert(1)"]) {
      await page.goto(`${direct}?return_to=${encodeURIComponent(bad)}`);
      const href = await page.getByRole("link", { name: /Back to/ }).getAttribute("href");
      expect(href).not.toContain("evil");
      expect(href).not.toContain("javascript");
    }
  });
});

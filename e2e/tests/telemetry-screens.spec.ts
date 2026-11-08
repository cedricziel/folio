import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

const screen = (page: import("@playwright/test").Page) =>
  page.locator('meta[name="folio-screen"]').getAttribute("content");

test("real pages declare the telemetry screen they show", async ({ page }) => {
  const title = uniqueTitle("screen");
  await uploadViaUi(page, title);
  expect(await screen(page)).toBe("inbox");

  await page.goto("/?view=all");
  expect(await screen(page)).toBe("documents");

  await page.getByRole("link", { name: new RegExp(title) }).click();
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
  expect(await screen(page)).toBe("document.detail");

  await page.goto("/trash");
  expect(await screen(page)).toBe("trash");

  await page.goto("/settings");
  expect(await screen(page)).toBe("settings");
});

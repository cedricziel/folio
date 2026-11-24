import AxeBuilder from "@axe-core/playwright";
import { expect, test, uniqueTitle } from "./fixtures";

test("adds, reorders, saves and removes workflow triggers", async ({ page }, testInfo) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("Multiple triggers"));
  const primary = page.getByRole("combobox", { name: "When", exact: true });
  const add = page.getByRole("button", { name: "Add trigger", exact: true });
  const second = page.getByRole("combobox", { name: "Additional trigger 1", exact: true });
  const third = page.getByRole("combobox", { name: "Additional trigger 2", exact: true });
  await add.click();
  await expect(second).toBeFocused();
  await expect(second.locator("option[value='document.uploaded']")).toBeDisabled();
  await expect(second.locator("option[value='workflow.scheduled']")).toBeDisabled();
  await expect(primary.locator("option[value='workflow.scheduled']")).toBeDisabled();
  await second.selectOption("document.updated");
  await add.click();
  await third.selectOption("document.processed");
  await page.getByRole("button", { name: "Move up, additional trigger 2", exact: true }).click();
  await page.getByRole("button", { name: "Move up, additional trigger 1", exact: true }).click();
  await expect(primary).toHaveValue("document.processed");
  await expect(second).toHaveValue("document.uploaded");
  await expect(third).toHaveValue("document.updated");
  await page.getByRole("combobox", { name: "Action to add" }).selectOption("webhook");
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
  const save = page.getByRole("button", { name: "Save workflow", exact: true });
  await save.click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  const id = new URL(page.url()).pathname.split("/").pop();
  const selections = async () => {
    const response = await page.request.get(`/api/workflows/${id}`);
    expect(response.ok()).toBeTruthy();
    return (await response.json()).triggers;
  };
  expect(await selections()).toEqual(["document.processed", "document.uploaded", "document.updated"]);
  await expect(second).toHaveValue("document.uploaded");
  await expect(third).toHaveValue("document.updated");

  for (const { width, height } of [{ width: 390, height: 844 }, { width: 1440, height: 1000 }]) {
    await page.setViewportSize({ width, height });
    for (const theme of ["light", "dark"] as const) {
      await page.emulateMedia({ colorScheme: theme });
      await page.reload();
      await expect(third).toHaveValue("document.updated");
      const audit = await new AxeBuilder({ page }).include("main").analyze();
      expect(audit.violations.filter((item) => ["serious", "critical"].includes(item.impact ?? ""))).toEqual([]);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
      const screenshot = testInfo.outputPath(`multiple-triggers-${width}-${theme}.png`);
      await page.screenshot({ path: screenshot, fullPage: true });
      await testInfo.attach(`multiple-triggers-${width}-${theme}`, { path: screenshot, contentType: "image/png" });
    }
  }

  await page.getByRole("button", { name: "Remove additional trigger 1", exact: true }).click();
  await expect(second).toHaveValue("document.updated");
  await page.getByRole("button", { name: "Remove additional trigger 1", exact: true }).click();
  await save.click();
  await expect(second).toHaveCount(0);
  expect(await selections()).toEqual(["document.processed"]);
});

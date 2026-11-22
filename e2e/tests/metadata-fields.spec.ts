import { mkdir } from "node:fs/promises";
import path from "node:path";
import { expect, test, uniqueTitle } from "./fixtures";

test("manages a select field and keeps its option through retirement", async ({
  page,
}) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Field management requires the disposable global-setup instance",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  const field = uniqueTitle("Payment status");
  await page.goto("/manage");
  await page.getByRole("link", { name: "Fields", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Fields", exact: true }),
  ).toBeVisible();
  await page.getByLabel("Name", { exact: true }).fill(field);
  await page
    .getByRole("combobox", { name: "Type", exact: true })
    .selectOption("select");
  await page.getByLabel("Required", { exact: true }).check();
  await page.getByRole("button", { name: "Add field" }).click();
  await expect(
    page.getByRole("heading", { name: field, exact: true }),
  ).toBeVisible();

  await page.getByLabel("New option").fill("Awaiting payment");
  await page.getByLabel("Position", { exact: true }).fill("2");
  await page.getByRole("button", { name: "Add option" }).click();
  const original = page.getByRole("region", { name: "Awaiting payment" });
  await original
    .getByLabel("Name", { exact: true })
    .fill("Awaiting confirmation");
  await original.getByRole("button", { name: "Save option" }).click();
  const updated = page.getByRole("region", { name: "Awaiting confirmation" });
  await expect(updated).toContainText("Position 2");
  const optionName = updated.getByRole("textbox", { name: "Name", exact: true });
  expect((await optionName.boundingBox())?.width).toBeGreaterThan(200);

  await updated.getByRole("button", { name: "Retire option" }).click();
  await expect(updated).toContainText("Retired");
  await updated.getByRole("button", { name: "Restore option" }).click();
  await expect(updated).not.toContainText("Retired");

  await page.getByRole("button", { name: "Retire field" }).click();
  await expect(
    page.getByRole("button", { name: "Restore field" }),
  ).toBeVisible();
  await expect(updated).toContainText("Awaiting confirmation");
  await page.getByRole("button", { name: "Restore field" }).click();
  await expect(page.getByRole("button", { name: "Save field" })).toBeVisible();
  const screenshot = process.env.CI
    ? path.resolve(__dirname, "../test-results/lifecycle-evidence/metadata-select-field.png")
    : test.info().outputPath("select-field.png");
  await mkdir(path.dirname(screenshot), { recursive: true });
  await page.screenshot({ path: screenshot, fullPage: true });
  await test.info().attach("Select field on a narrow screen", {
    path: screenshot,
    contentType: "image/png",
  });
});

import path from "node:path";
import { expect, test, uniqueTitle } from "./fixtures";

test("edits exact custom values and follows a logical document link", async ({ page }, testInfo) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Custom field editing requires the disposable global-setup instance",
  );
  const documentTitle = uniqueTitle("Invoice");
  const targetTitle = uniqueTitle("Receipt");
  const amountName = uniqueTitle("Amount");
  const linkName = uniqueTitle("Related document");
  const catalog = await page.request.get("/api/vaults");
  expect(catalog.ok()).toBeTruthy();
  const { default_vault_id: vault } = await catalog.json();
  for (const [name, kind] of [[amountName, "decimal"], [linkName, "document_link"]]) {
    const response = await page.request.post(`/api/vaults/${vault}/metadata-fields`, { data: { name, kind } });
    expect(response.status()).toBe(201);
  }
  const original = await page.request.post("/api/documents", { data: { title: documentTitle } });
  const target = await page.request.post("/api/documents", { data: { title: targetTitle } });
  expect(original.ok()).toBeTruthy();
  expect(target.ok()).toBeTruthy();
  const { id: originalId } = await original.json();
  const { id: targetId } = await target.json();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`/documents/${originalId}?return_to=%2F%3Fview%3Dall`);
  const custom = page.locator("turbo-frame#custom-fields");
  await custom.getByRole("link", { name: `Edit ${amountName}` }).click();
  await page.getByRole("textbox", { name: "Value", exact: true }).fill("1e3");
  await page.getByRole("button", { name: "Save value" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  const amount = page.getByRole("textbox", { name: "Value", exact: true });
  await expect(amount).toHaveValue("1e3");
  await amount.fill("9007199254740993.010");
  await page.getByRole("button", { name: "Save value" }).click();
  await expect(custom.getByText("9007199254740993.01", { exact: true })).toBeVisible();
  expect(new URL(page.url()).searchParams.get("return_to")).toBe("/?view=all");

  await custom.getByRole("link", { name: `Edit ${linkName}` }).click();
  await page.getByRole("searchbox", { name: "Search this vault for a document" }).fill(targetTitle);
  await page.getByRole("main").getByRole("button", { name: "Search documents", exact: true }).click();
  expect(new URL(page.url()).searchParams.get("q")).toBe(targetTitle);
  const choices = page.getByRole("combobox", { name: "Value", exact: true });
  await expect(choices.getByRole("option", { name: documentTitle, exact: true })).toHaveCount(0);
  await choices.selectOption(targetId);
  await page.screenshot({
    path: process.env.CI
      ? path.resolve(__dirname, "../test-results/lifecycle-evidence/metadata-link-editor.png")
      : testInfo.outputPath("metadata-link-editor.png"),
  });
  await page.getByRole("button", { name: "Save value" }).click();
  const linked = custom.getByRole("link", { name: targetTitle, exact: true });
  await expect(linked).toHaveAttribute("href", new RegExp(`/documents/${targetId}$`));
  await linked.click();
  await expect(page.getByRole("heading", { name: targetTitle, exact: true })).toBeVisible();
});

import { expect, test, uniqueTitle } from "./fixtures";

test("chooses custom metadata triggers and exact transition conditions", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  const fieldName = uniqueTitle("Invoice amount");
  const workflowName = uniqueTitle("Exact amount workflow");
  const vaults = await page.request.get("/api/vaults");
  expect(vaults.ok()).toBeTruthy();
  const { default_vault_id: vault } = await vaults.json();
  const created = await page.request.post(`/api/vaults/${vault}/metadata-fields`, {
    data: { name: fieldName, kind: "decimal" },
  });
  expect(created.status()).toBe(201);
  const field = await created.json();

  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(workflowName);
  const trigger = page.getByRole("combobox", { name: "When" });
  await trigger.selectOption("metadata_field.created");
  const tagAction = page.getByRole("combobox", { name: "Action to add" }).locator('option[value="add_tags"]');
  await expect(tagAction).toBeDisabled();
  await expect(page.getByRole("group", { name: "Custom field conditions" })).toBeVisible();
  await trigger.selectOption("document.metadata.changed");
  await expect(tagAction).toBeEnabled();
  await page.getByRole("combobox", { name: "Custom field", exact: true }).selectOption({ label: fieldName });
  await page.getByRole("combobox", { name: "Field type" }).selectOption("decimal");
  await page.getByRole("combobox", { name: "Change", exact: true }).selectOption("set");
  await page.getByRole("checkbox", { name: "Match the new value" }).check();
  await page.getByRole("textbox", { name: "New value", exact: true }).fill("9007199254740993.010");
  await page.getByRole("combobox", { name: "Action to add" }).selectOption("webhook");
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "New value", exact: true })).toHaveValue("9007199254740993.01");
  const id = new URL(page.url()).pathname.split("/").pop();
  const response = await page.request.get(`/api/workflows/${id}`);
  expect(response.ok()).toBeTruthy();
  expect((await response.json()).conditions.event.metadata).toEqual({
    field_id: field.id, kind: "decimal", state: "set",
    after: { kind: "decimal", value: "9007199254740993.01" },
  });
});

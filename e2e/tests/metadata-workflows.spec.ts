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

test("shows and saves the condition fields for the selected document event", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("File changes"));
  const trigger = page.getByRole("combobox", { name: "When" });
  const processing = page.locator('[data-workflow-form-target="processingFilter"]');
  await trigger.selectOption("document.processing.ocr.completed");
  await expect(processing.getByRole("combobox", { name: "Text found", exact: true })).toBeVisible();
  await expect(page.getByLabel("Revision number")).toBeHidden();
  await trigger.selectOption("document.date_changed");
  await expect(processing).toBeHidden();
  await page.getByLabel("Previous document date", { exact: true }).fill("2024-02-29");
  await page.getByLabel("Date change", { exact: true }).selectOption("cleared");
  await trigger.selectOption("document.reviewed");
  await expect(page.getByLabel("Review origin", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Reason for marking unreviewed")).toBeHidden();
  await trigger.selectOption("document.unreviewed");
  await expect(page.getByLabel("Reason for marking unreviewed")).toBeVisible();
  await trigger.selectOption("document.revision_restored");
  await expect(page.getByLabel("Revision number")).toBeVisible();
  await expect(page.getByLabel("Revision source", { exact: true })).toBeHidden();
  await trigger.selectOption("document.revision_created");
  await expect(page.getByLabel("Previous document date", { exact: true })).toBeHidden();
  await page.getByLabel("Revision number").fill("3");
  await page.getByLabel("Revision source", { exact: true }).selectOption("restoration");

  const editor = page.getByRole("group", { name: "Nested conditions", exact: true });
  const groups = editor.locator(":scope > [data-condition-add]");
  await groups.getByRole("combobox", { name: "Group operator" }).selectOption("any");
  await groups.getByRole("button", { name: "Add group", exact: true }).click();
  const alternatives = editor.getByRole("group", { name: "Any of these", exact: true });
  await alternatives.getByRole("button", { name: "Add branch", exact: true }).click();
  const fileType = alternatives.getByRole("textbox", { name: "File type", exact: true });
  await fileType.fill("application/pdf");

  await page.getByRole("combobox", { name: "Action to add" }).selectOption("webhook");
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(page.getByLabel("Revision source", { exact: true })).toHaveValue("restoration");
  await expect(page.getByLabel("Revision number")).toHaveValue("3");
  await expect(fileType).toHaveValue("application/pdf");
  const id = new URL(page.url()).pathname.split("/").pop();
  const response = await page.request.get(`/api/workflows/${id}`);
  await expect(response).toBeOK();
  const { conditions } = await response.json();
  expect(conditions.event.document_change).toEqual({
    type: "revision", number: 3, source: "restoration",
  });
  expect(conditions.any).toEqual([{ mime_type: "application/pdf" }]);
});

import AxeBuilder from "@axe-core/playwright";
import { expect, test, uniqueTitle } from "./fixtures";

test("adds, moves, removes, and reloads ordered workflow actions", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  const name = uniqueTitle("Ordered actions");
  const tagName = uniqueTitle("Invoice tag");
  const response = await page.request.post("/api/tags", { data: { name: tagName } });
  expect(response.status()).toBe(200);
  const tag = await response.json();

  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(name);
  const add = page.getByRole("button", { name: "Add action", exact: true });
  const kind = page.getByRole("combobox", { name: "Action to add" });
  await kind.selectOption("webhook");
  await add.click();
  await add.click();
  const rows = page.locator("[data-action-row]");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/first");
  const kept = "http://127.0.0.1:9/second";
  await rows.nth(1).getByRole("textbox", { name: "URL", exact: true }).fill(kept);
  await kind.selectOption("add_tags");
  await add.click();
  await rows.nth(2).getByRole("checkbox", { name: tagName }).check();
  await rows.nth(2).getByRole("button", { name: "Move up", exact: true }).click();
  await rows.nth(0).getByRole("button", { name: "Remove action", exact: true }).click();
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).getByRole("checkbox", { name: tagName })).toBeChecked();
  await expect(rows.nth(1).getByRole("textbox", { name: "URL", exact: true })).toHaveValue(kept);
  const workflowURL = page.url();
  await rows.nth(1).getByLabel("Secret", { exact: true }).fill("unsaved-credential");
  await page.locator(".back-link").click();
  await expect(page).toHaveURL(/\/workflows$/);
  await page.goBack();
  await expect(page).toHaveURL(workflowURL);
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1).getByLabel("Secret", { exact: true })).toHaveValue("");
  const id = new URL(workflowURL).pathname.split("/").pop();
  const saved = await page.request.get(`/api/workflows/${id}`);
  expect(saved.ok()).toBeTruthy();
  expect((await saved.json()).actions).toEqual([
    { type: "add_tags", tag_ids: [tag.id] },
    { type: "webhook", method: "POST", url: kept, headers: {} },
  ]);
});

test("edits document action details and keeps Trash last", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("File reviewed invoices"));
  const add = page.getByRole("button", { name: "Add action", exact: true });
  const kind = page.getByRole("combobox", { name: "Action to add" });
  for (const action of ["rename_document", "set_document_date", "set_reviewed", "trash_document"]) {
    await kind.selectOption(action);
    await add.click();
  }
  const rows = page.locator("[data-action-row]");
  await rows.nth(0).getByRole("textbox", { name: "Document title" }).fill("Reviewed invoice");
  await rows.nth(1).getByLabel("Document date", { exact: true }).fill("2026-11-20");
  await rows.nth(2).getByRole("combobox", { name: "Review state" }).selectOption("true");
  await rows.nth(1).getByRole("button", { name: "Move up", exact: true }).click();
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();

  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(rows).toHaveCount(4);
  await expect(rows.nth(0).getByLabel("Document date", { exact: true })).toHaveValue("2026-11-20");
  await expect(rows.nth(1).getByRole("textbox", { name: "Document title" })).toHaveValue("Reviewed invoice");
  await expect(rows.nth(2).getByRole("combobox", { name: "Review state" })).toHaveValue("true");
  await expect(rows.nth(3).getByText("Move document to Trash", { exact: true })).toBeVisible();
  const id = new URL(page.url()).pathname.split("/").pop();
  const saved = await page.request.get(`/api/workflows/${id}`);
  expect(saved.ok()).toBeTruthy();
  expect((await saved.json()).actions).toEqual([
    { type: "set_document_date", date: "2026-11-20" },
    { type: "rename_document", title: "Reviewed invoice" },
    { type: "set_reviewed", reviewed: true },
    { type: "trash_document" },
  ]);
});

test("sets typed custom values and clears a field in the saved action order", async ({ page }, testInfo) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  const vaults = await page.request.get("/api/vaults");
  expect(vaults.ok()).toBeTruthy();
  const { default_vault_id: vault } = await vaults.json();
  const fieldURL = `/api/vaults/${vault}/metadata-fields`;
  const createField = async (name: string, kind: string, required = false) => {
    const response = await page.request.post(fieldURL, { data: { name: uniqueTitle(name), kind, required } });
    expect(response.status()).toBe(201);
    return response.json();
  };
  const amount = await createField("Invoice total", "money");
  const approval = await createField("Invoice approval", "select", true);
  const link = await createField("Invoice link", "document_link");
  const receiptTitle = uniqueTitle("Related receipt");
  const receipt = await page.request.post("/api/documents", { data: { title: receiptTitle } });
  expect(receipt.ok()).toBeTruthy();
  const { id: receiptID } = await receipt.json();
  const option = await page.request.post(`${fieldURL}/${approval.id}/options`, {
    data: { label: "Needs approval", position: 0 },
  });
  expect(option.status()).toBe(201);
  const { id: optionID } = await option.json();

  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("Set invoice fields"));
  const add = page.getByRole("button", { name: "Add action", exact: true });
  const kind = page.getByRole("combobox", { name: "Action to add" });
  const rows = page.locator("[data-action-row]");
  const customField = (row: number) => rows.nth(row).getByRole("combobox", { name: "Custom field", exact: true });

  await kind.selectOption("set_metadata");
  await add.click();
  await customField(0).selectOption(amount.id);
  const exact = "9007199254740993.010";
  await rows.nth(0).getByRole("textbox", { name: "Amount", exact: true }).fill(exact);
  await rows.nth(0).getByRole("textbox", { name: "Currency code" }).fill("EUR");
  await add.click();
  await customField(1).selectOption(approval.id);
  await rows.nth(1).getByRole("combobox", { name: "Value", exact: true }).selectOption(optionID);

  await kind.selectOption("clear_metadata");
  await add.click();
  await customField(2).selectOption(amount.id);
  await expect(customField(2).locator(`option[value="${approval.id}"]`)).toHaveCount(0);
  await kind.selectOption("set_metadata");
  await add.click();
  await customField(3).selectOption(link.id);
  await rows.nth(3).getByRole("searchbox", { name: "Search this vault for a document" }).fill(receiptTitle);
  await rows.nth(3).getByRole("button", { name: "Search documents", exact: true }).click();
  await rows.nth(3).getByRole("combobox", { name: "Value", exact: true }).selectOption(receiptID);
  await rows.nth(1).getByRole("button", { name: "Move up", exact: true }).click();
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();

  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(rows).toHaveCount(4);
  await expect(rows.nth(0).getByRole("combobox", { name: "Value", exact: true })).toHaveValue(optionID);
  await expect(rows.nth(1).getByRole("textbox", { name: "Amount", exact: true })).toHaveValue(exact);
  await expect(customField(2)).toHaveValue(amount.id);
  const id = new URL(page.url()).pathname.split("/").pop();
  const workflow = await page.request.get(`/api/workflows/${id}`);
  expect(workflow.ok()).toBeTruthy();
  expect((await workflow.json()).actions).toEqual([
    { type: "set_metadata", field_id: approval.id, value: { kind: "select", value: optionID } },
    { type: "set_metadata", field_id: amount.id, value: { kind: "money", value: { amount: exact, currency: "EUR" } } },
    { type: "clear_metadata", field_id: amount.id },
    { type: "set_metadata", field_id: link.id, value: { kind: "document_link", value: receiptID } },
  ]);

  const retired = await page.request.delete(`${fieldURL}/${approval.id}/options/${optionID}`);
  expect(retired.ok()).toBeTruthy();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload();
  const savedChoice = rows.nth(0).getByRole("combobox", { name: "Value", exact: true });
  await expect(savedChoice).toHaveValue(optionID);
  await expect(savedChoice.locator("option:checked")).toHaveText("Saved value");
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(savedChoice).toHaveValue(optionID);
  await rows.nth(1).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("custom-field-workflow-mobile.png") });
  const audit = await new AxeBuilder({ page }).include("[data-action-rows]").analyze();
  expect(audit.violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
});

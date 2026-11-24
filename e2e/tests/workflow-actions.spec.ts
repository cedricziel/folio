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

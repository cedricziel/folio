import { expect, test, uniqueTitle } from "./fixtures";

test("authors nested conditions and removes an empty alternative group", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );

  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("Nested conditions"));
  const editor = page.getByRole("group", { name: "Nested conditions", exact: true });
  const rootControls = editor.locator(":scope > [data-condition-add]");
  await rootControls.getByRole("combobox", { name: "Group operator" }).selectOption("any");
  await rootControls.getByRole("button", { name: "Add group", exact: true }).click();
  const alternatives = editor.getByRole("group", { name: "Any of these", exact: true });
  await expect(alternatives.getByText("An empty group never matches.")).toBeVisible();
  await rootControls.getByRole("combobox", { name: "Group operator" }).selectOption("all");
  await rootControls.getByRole("button", { name: "Add group", exact: true }).click();
  const conjunction = editor.getByRole("group", { name: "All of these", exact: true });
  await conjunction.getByRole("button", { name: "Add branch", exact: true }).click();
  const parent = conjunction.getByRole("group", { name: "Branch", exact: true });
  await parent.getByRole("textbox", { name: "File type", exact: true }).fill("application/pdf");

  const childControls = parent.locator(":scope > [data-condition-add]");
  await childControls.getByRole("combobox", { name: "Group operator" }).selectOption("none");
  await childControls.getByRole("button", { name: "Add group", exact: true }).click();
  const exclusions = parent.getByRole("group", { name: "None of these", exact: true });
  await exclusions.getByRole("button", { name: "Add branch", exact: true }).click();
  await exclusions.getByRole("textbox", { name: "File name", exact: true }).fill("draft-*");

  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
  const save = page.getByRole("button", { name: "Save workflow", exact: true });
  await save.click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(exclusions.getByRole("textbox", { name: "File name", exact: true })).toHaveValue("draft-*");

  const id = new URL(page.url()).pathname.split("/").pop();
  const saved = async () => {
    const response = await page.request.get(`/api/workflows/${id}`);
    expect(response.ok()).toBeTruthy();
    return (await response.json()).conditions;
  };
  const all = [{ mime_type: "application/pdf", none: [{ filename_glob: "draft-*" }] }];
  expect(await saved()).toEqual({ all, any: [] });

  await alternatives.getByRole("button", { name: "Remove group", exact: true }).click();
  await save.click();
  await expect(alternatives).toHaveCount(0);
  expect(await saved()).toEqual({ all });
});

test("edits a nested exact money condition without changing its previous value", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );

  const before = { kind: "money", value: { amount: "9007199254740993.01", currency: "EUR" } };
  const after = { kind: "money", value: { amount: "9007199254740993.015", currency: "EUR" } };
  const metadata = { kind: "money", state: "replaced", before, after };
  const created = await page.request.post("/api/workflows", {
    data: {
      name: uniqueTitle("Nested money condition"),
      enabled: false,
      trigger: "document.metadata.changed",
      actions: [{ type: "webhook", url: "http://127.0.0.1:9/unused" }],
      conditions: { any: [{ event: { metadata } }] },
    },
  });
  expect(created.status()).toBe(201);
  const { id } = await created.json();

  await page.goto(`/workflows/${id}`);
  const branch = page.getByRole("group", { name: "Branch", exact: true });
  const next = branch.getByRole("textbox", { name: "New value", exact: true });
  await expect(next).toHaveValue(after.value.amount);
  await next.fill("9007199254740993.02");
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(next).toHaveValue("9007199254740993.02");

  const response = await page.request.get(`/api/workflows/${id}`);
  expect(response.ok()).toBeTruthy();
  expect((await response.json()).conditions.any).toEqual([{
    event: { metadata: { ...metadata, after: { ...after, value: { amount: "9007199254740993.02", currency: "EUR" } } } },
  }]);
});

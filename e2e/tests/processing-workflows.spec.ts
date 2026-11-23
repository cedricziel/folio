import { expect, test, uniqueTitle } from "./fixtures";

test("edits processing predicates at the root and in a nested branch", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );

  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("Text extraction conditions"));
  const trigger = page.getByRole("combobox", { name: "When", exact: true });
  await trigger.selectOption("document.processing.ocr.completed");
  const root = page.locator('[data-workflow-form-target="processingFilter"]');
  await expect(root.getByText("Retry time", { exact: true })).toBeHidden();
  const found = root.getByRole("combobox", { name: "Text found", exact: true });
  await found.selectOption("false");
  await root.getByRole("textbox", { name: "Attempt", exact: true }).fill("2");

  const groups = page.getByRole("group", { name: "Nested conditions", exact: true });
  await groups.getByRole("button", { name: "Add group", exact: true }).click();
  const all = groups.getByRole("group", { name: "All of these", exact: true });
  await all.getByRole("button", { name: "Add branch", exact: true }).click();
  const nested = all.getByRole("group", { name: "Processing conditions", exact: true });
  const nestedFound = nested.getByRole("combobox", { name: "Text found", exact: true });
  await nestedFound.selectOption("true");
  await nested.getByRole("textbox", { name: "Attempt", exact: true }).fill("3");
  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
  const save = page.getByRole("button", { name: "Save workflow", exact: true });
  await save.click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);

  const id = new URL(page.url()).pathname.split("/").pop();
  const load = async () => {
    const response = await page.request.get(`/api/workflows/${id}`);
    await expect(response).toBeOK();
    return response.json();
  };
  const saved = await load();
  expect(saved.conditions).toEqual({
    event: { processing: { attempt: 2, has_text: false } },
    all: [{ event: { processing: { attempt: 3, has_text: true } } }],
  });
  await expect(found).toHaveValue("false");
  await expect(nestedFound).toHaveValue("true");

  await trigger.selectOption("document.processing.thumbnail.completed");
  await expect(found).toBeVisible();
  await expect(nestedFound).toBeVisible();
  await save.click();
  await expect(page.getByText("These processing conditions are unavailable for the selected trigger")).toBeVisible();
  expect((await load()).conditions).toEqual(saved.conditions);

  await found.selectOption("");
  await save.click();
  await expect(page.getByText("These processing conditions are unavailable for the selected trigger")).toBeVisible();
  expect((await load()).conditions).toEqual(saved.conditions);

  await nestedFound.selectOption("");
  await save.click();
  await expect.poll(async () => (await load()).conditions).toEqual({
    event: { processing: { attempt: 2 } },
    all: [{ event: { processing: { attempt: 3 } } }],
  });
  await page.reload();
  await expect(trigger).toHaveValue("document.processing.thumbnail.completed");
  await expect(found).toBeHidden();
  await expect(nestedFound).toBeHidden();
});

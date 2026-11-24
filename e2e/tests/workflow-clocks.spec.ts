import { expect, test, uniqueTitle } from "./fixtures";

test("edits a recurring workflow clock as a one-time schedule", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  const at = "2040-01-03T09:00:00.123456789Z";
  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("Clock workflow"));
  await page.getByRole("combobox", { name: "When", exact: true }).selectOption("workflow.scheduled");
  await expect(page.getByRole("group", { name: "Schedule", exact: true })).toBeVisible();
  await expect(page.getByRole("group", { name: "Nested conditions", exact: true })).toBeHidden();
  const actionKind = page.getByRole("combobox", { name: "Action to add" });
  await expect(actionKind.locator('option[value="add_tags"]')).toBeDisabled();
  const frequency = page.getByRole("combobox", { name: "Frequency" });
  await frequency.selectOption("recurring");
  const interval = page.getByRole("spinbutton", { name: "Repeat every (seconds)" });
  await interval.fill("3600");
  const start = page.getByRole("textbox", { name: "First run (UTC)" });
  await start.fill(at);
  await actionKind.selectOption("webhook");
  const add = page.getByRole("button", { name: "Add action", exact: true });
  await add.click();
  await add.click();
  const urls = ["http://127.0.0.1:9/first", "http://127.0.0.1:9/second"];
  const inputs = page.getByRole("textbox", { name: "URL", exact: true });
  for (const [index, url] of urls.entries()) await inputs.nth(index).fill(url);
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(frequency).toHaveValue("recurring");
  await expect(interval).toHaveValue("3600");
  await expect(start).toHaveValue(at);

  await interval.fill("0");
  await frequency.selectOption("once");
  await expect(interval).toBeHidden();
  const saved = page.waitForResponse((response) =>
    response.request().method() === "POST" && response.status() === 303,
  );
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await saved;
  const id = new URL(page.url()).pathname.split("/").pop();
  const response = await page.request.get(`/api/workflows/${id}`);
  expect(response.ok()).toBeTruthy();
  const stored = await response.json();
  expect(stored.cadence).toEqual({ type: "once", at });
  expect(stored.actions).toEqual(urls.map((url) => ({ type: "webhook", url, method: "POST", headers: {} })));
});

test("removes saved root filters, grouped filters and document actions when converting to a clock", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );
  const start = "2040-03-01T09:00:00Z";
  const created = await page.request.post("/api/workflows", {
    data: {
      name: uniqueTitle("Convert a filtered workflow"),
      enabled: false,
      trigger: "document.uploaded",
      conditions: { mime_type: "application/pdf", any: [{ filename_glob: "invoice-*" }] },
      actions: [
        { type: "rename_document", title: "Archive invoice" },
        { type: "webhook", url: "http://127.0.0.1:9/unused" },
      ],
    },
  });
  expect(created.status()).toBe(201);
  const { id } = await created.json();

  await page.goto(`/workflows/${id}`);
  const trigger = page.getByRole("combobox", { name: "When", exact: true });
  await trigger.selectOption("workflow.scheduled");
  await page.getByRole("checkbox", { name: "Remove saved filters outside groups" }).check();
  await expect(page.getByRole("checkbox", { name: "Remove saved actions other than webhooks" })).toHaveCount(0);
  const editor = page.getByRole("group", { name: "Nested conditions", exact: true });
  await expect(editor.getByText("Remove nested conditions before saving this clock schedule.")).toBeVisible();
  await expect(editor.getByRole("button", { name: "Add group", exact: true })).toBeHidden();
  const group = editor.getByRole("group", { name: "Any of these", exact: true });
  await group.getByRole("button", { name: "Remove group", exact: true }).click();
  await expect(editor).toBeHidden();
  await expect(trigger).toBeFocused();

  const rename = page.locator('[data-action-row][data-kind="rename_document"]');
  await expect(rename.getByText("Remove this action to use the selected trigger.")).toBeVisible();
  await rename.getByRole("button", { name: "Remove action", exact: true }).click();
  await page.getByRole("textbox", { name: "First run (UTC)" }).fill(start);
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect.poll(async () => {
    const response = await page.request.get(`/api/workflows/${id}`);
    expect(response.ok()).toBeTruthy();
    const { trigger, cadence, conditions, actions } = await response.json();
    return { trigger, cadence, conditions, actions };
  }).toEqual({
    trigger: "workflow.scheduled",
    cadence: { type: "once", at: start },
    conditions: {},
    actions: [{ type: "webhook", url: "http://127.0.0.1:9/unused", method: "POST", headers: {} }],
  });
});

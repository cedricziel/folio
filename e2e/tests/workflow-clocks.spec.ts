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
  await expect(page.getByText("Add tags", { exact: true })).toBeHidden();
  const frequency = page.getByRole("combobox", { name: "Frequency" });
  await frequency.selectOption("recurring");
  const interval = page.getByRole("spinbutton", { name: "Repeat every (seconds)" });
  await interval.fill("3600");
  const start = page.getByRole("textbox", { name: "First run (UTC)" });
  await start.fill(at);
  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
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
  expect((await response.json()).cadence).toEqual({ type: "once", at });
});

test("removes saved nested filters before converting to a clock", async ({ page }) => {
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
      conditions: { any: [{ filename_glob: "invoice-*" }] },
      actions: [{ type: "webhook", url: "http://127.0.0.1:9/unused" }],
    },
  });
  expect(created.status()).toBe(201);
  const { id } = await created.json();

  await page.goto(`/workflows/${id}`);
  const trigger = page.getByRole("combobox", { name: "When", exact: true });
  await trigger.selectOption("workflow.scheduled");
  const editor = page.getByRole("group", { name: "Nested conditions", exact: true });
  await expect(editor.getByText("Remove nested conditions before saving this clock schedule.")).toBeVisible();
  await expect(editor.getByRole("button", { name: "Add group", exact: true })).toBeHidden();
  const group = editor.getByRole("group", { name: "Any of these", exact: true });
  await group.getByRole("button", { name: "Remove group", exact: true }).click();
  await expect(editor).toBeHidden();
  await expect(trigger).toBeFocused();

  await page.getByRole("textbox", { name: "First run (UTC)" }).fill(start);
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect.poll(async () => {
    const response = await page.request.get(`/api/workflows/${id}`);
    expect(response.ok()).toBeTruthy();
    const { trigger, cadence, conditions } = await response.json();
    return { trigger, cadence, conditions };
  }).toEqual({
    trigger: "workflow.scheduled",
    cadence: { type: "once", at: start },
    conditions: {},
  });
});

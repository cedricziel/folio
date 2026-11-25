import { expect, test, uniqueTitle } from "./fixtures";

test("authors an owner workflow occurrence rule", async ({ page }) => {
  test.skip(
    Boolean(process.env.E2E_BASE_URL) || !process.env.FOLIO_VERIFY_ENV,
    "Workflow editing requires the disposable global-setup instance",
  );

  const subject = "d328c55a-1bf3-4590-b878-ad432191caba";
  await page.goto("/workflows/new");
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(uniqueTitle("Notify on workflow disable"));
  await page.getByRole("checkbox", { name: "Enabled", exact: true }).uncheck();
  await page.getByRole("combobox", { name: "When", exact: true }).selectOption("workflow.disabled");
  await expect(page.getByRole("textbox", { name: "File type", exact: true })).toHaveCount(0);
  const idField = page.getByRole("textbox", { name: "Subject ID", exact: true });
  await idField.fill(subject);
  await page.getByRole("combobox", { name: "Who made the change", exact: true }).selectOption("user");
  await page.getByRole("textbox", { name: "User ID", exact: true }).fill("vault-owner");
  await page.getByRole("combobox", { name: "Action to add" }).selectOption("webhook");
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page.getByRole("textbox", { name: "URL", exact: true }).fill("http://127.0.0.1:9/unused");
  await page.getByRole("button", { name: "Save workflow", exact: true }).click();
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(idField).toHaveValue(subject);

  const id = new URL(page.url()).pathname.split("/").pop();
  const response = await page.request.get(`/api/workflows/${id}`);
  expect(response.ok()).toBeTruthy();
  expect((await response.json()).conditions).toEqual({
    event: { subject_id: subject, actor: { type: "user", user_id: "vault-owner" } },
  });
});

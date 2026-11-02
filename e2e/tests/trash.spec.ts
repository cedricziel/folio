import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

test("restoring a trashed document returns it to the archive", async ({
  page,
}) => {
  const title = uniqueTitle("restore");
  await uploadViaUi(page, title);
  await page.getByRole("link", { name: new RegExp(title) }).click();
  page.once("dialog", (dialog) => dialog.accept());
  await page
    .getByRole("button", { name: "Move to Trash", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
  await page.goto("/trash");

  const row = page.getByRole("article").filter({ hasText: title });
  await row.getByRole("button", { name: "Restore", exact: true }).click();

  await expect(row).toHaveCount(0);
  await page.goto("/?view=all");
  await expect(
    page.getByRole("link", { name: new RegExp(title) }),
  ).toBeVisible();
});

test("Delete forever requires confirmation and removes the trashed document", async ({
  page,
}) => {
  test.setTimeout(30_000);
  const title = uniqueTitle("delete-forever");
  await uploadViaUi(page, title);
  await page.getByRole("link", { name: new RegExp(title) }).click();
  const documentUrl = page.url();
  page.once("dialog", (dialog) => dialog.accept());
  await page
    .getByRole("button", { name: "Move to Trash", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
  await page.goto("/trash");
  const row = page.getByRole("article").filter({ hasText: title });

  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toContain("This cannot be undone.");
    await dialog.dismiss();
  });
  await row
    .getByRole("button", { name: "Delete forever", exact: true })
    .click();
  await expect(
    row.getByRole("button", { name: "Restore", exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(row).toBeVisible();

  page.once("dialog", (dialog) => dialog.accept());
  await row
    .getByRole("button", { name: "Delete forever", exact: true })
    .click();

  await expect
    .poll(
      async () => {
        await page.reload();
        return row.count();
      },
      { timeout: 15_000 },
    )
    .toBe(0);
  await page.goto(documentUrl);
  await expect(
    page.getByRole("heading", { level: 1, name: title }),
  ).toHaveCount(0);
});

test("Trash reflects deletion and restore from another client without reloading", async ({ page }) => {
  const title = uniqueTitle("live-trash");
  await uploadViaUi(page, title);
  const listing = await page.request.get("/api/documents");
  const { documents } = await listing.json();
  const doc = documents.find((item: { title: string }) => item.title === title);
  await page.goto("/trash");
  await expect(page.locator("#trash-content")).toBeVisible();
  const row = page.getByRole("article").filter({ hasText: title });

  expect((await page.request.delete(`/api/documents/${doc.id}`)).status()).toBe(204);
  await expect(row).toBeVisible();
  expect((await page.request.post(`/api/documents/${doc.id}/restore`)).status()).toBe(200);
  await expect(row).toHaveCount(0);
  expect((await page.request.delete(`/api/documents/${doc.id}`)).status()).toBe(204);
  await expect(row).toBeVisible();
});

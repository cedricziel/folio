import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

test.describe("document back link", () => {
  test.describe.configure({ mode: "serial" });
  const title = uniqueTitle("back-link");

  test("returns to the all-documents list it was opened from", async ({ page }) => {
    await uploadViaUi(page, title);
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();

    const back = page.getByRole("link", { name: /Back to All documents/ });
    await expect(back).toBeVisible();
    await back.click();

    await expect(page).toHaveURL(/\/\?view=all$/);
    await expect(page.getByRole("link", { name: new RegExp(title) })).toBeVisible();
    await expect(page.getByRole("heading", { name: "All documents", level: 1 })).toBeVisible();
  });

  test("falls back to the inbox when opened directly", async ({ page }) => {
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/\/documents\//);
    const direct = page.url().split("?")[0];
    await page.goto(direct);
    const back = page.getByRole("link", { name: /Back to Inbox/ });
    await expect(back).toBeVisible();
    await back.click();
    await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
  });

  test("ignores an external return target", async ({ page }) => {
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/\/documents\//);
    const direct = page.url().split("?")[0];
    for (const bad of ["https://evil.example/", "//evil.example/", "javascript:alert(1)"]) {
      await page.goto(`${direct}?return_to=${encodeURIComponent(bad)}`);
      const href = await page.getByRole("link", { name: /Back to/ }).getAttribute("href");
      expect(href).not.toContain("evil");
      expect(href).not.toContain("javascript");
    }
  });

  test("returns to a correspondent-filtered list", async ({ page }) => {
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/\/documents\//);
    const documentId = new URL(page.url()).pathname.split("/").at(-1)!;

    const vaultId = page.url().match(/\/vaults\/([0-9a-f-]{36})\//)![1];
    const correspondent = await page.request.post(`/vaults/${vaultId}/api/correspondents`, {
      data: { name: uniqueTitle("sender") },
    });
    expect(correspondent.ok()).toBeTruthy();
    const { id: correspondentId } = await correspondent.json();
    await page.request.post(`/documents/${documentId}/correspondent`, {
      form: { correspondent_id: correspondentId },
    });

    const filtered = `/?correspondent_id=${correspondentId}`;
    await page.goto(filtered);
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
    await page.getByRole("link", { name: "Back to filtered documents" }).click();
    await expect(page).toHaveURL(new RegExp(`correspondent_id=${correspondentId}$`));
    await expect(page.getByRole("link", { name: new RegExp(title) })).toBeVisible();
  });

  test("an edit on the document page keeps the way back", async ({ page }) => {
    await page.goto("/?view=all");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/return_to=/);
    const row = page.getByRole("group", { name: /reviewed/i });
    const review = row.or(page.getByRole("button", { name: /Mark (as )?reviewed|Reviewed/i })).first();
    await review.click();
    await expect(page).toHaveURL(/return_to=/);
    await expect(page.getByRole("link", { name: "Back to All documents" })).toBeVisible();
  });

  test("a document opened in a new tab still goes back to its list", async ({ page, context }) => {
    await page.goto("/?view=all");
    const href = await page.getByRole("link", { name: new RegExp(title) }).getAttribute("href");
    const listUrl = page.url();
    const tab = await context.newPage();
    await tab.goto(href!, { referer: listUrl });
    await tab.getByRole("link", { name: "Back to All documents" }).click();
    await expect(tab).toHaveURL(/\/\?view=all$/);
    await tab.close();
  });

  test("a row updated live keeps pointing back to the list", async ({ page }) => {
    await page.goto("/?view=all");
    const link = page.getByRole("link", { name: new RegExp(title) });
    await expect(link).toHaveAttribute("href", /return_to=/);
    await page.evaluate(() => {
      const row = document.querySelector('a.f-row[href*="/documents/"]')!;
      const clean = row.getAttribute("href")!.split("?")[0];
      const stream = document.createElement("turbo-stream");
      stream.setAttribute("action", "replace");
      stream.setAttribute("target", row.id);
      stream.innerHTML = `<template><a class="f-row" id="${row.id}" href="${clean}"><div class="f-row-title">live</div></a></template>`;
      document.body.append(stream);
    });
    await expect(page.locator('a.f-row div.f-row-title', { hasText: "live" })).toBeVisible();
    await expect(page.locator('a.f-row:has(.f-row-title:text("live"))')).toHaveAttribute("href", /return_to=/);
  });
});


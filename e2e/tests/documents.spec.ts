import { readFileSync } from "node:fs";
import { expect, test, uniqueTitle, uploadViaUi, samplePdf } from "./fixtures";

test("inbox loads", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/Folio/);
  await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Upload document", exact: true }),
  ).toBeVisible();
});

test("dropping multiple files uploads each document", async ({ page, request }) => {
  const names = [uniqueTitle("drop-first") + ".pdf", uniqueTitle("drop-second") + ".pdf"];
  await page.goto("/");
  const transfer = await page.evaluateHandle(({ bytes, names }) => {
    const data = new DataTransfer();
    for (const name of names) {
      data.items.add(new File([new Uint8Array(bytes)], name, { type: "application/pdf" }));
    }
    return data;
  }, { bytes: Array.from(readFileSync(samplePdf)), names });

  try {
    await page.locator("body").dispatchEvent("dragenter", { dataTransfer: transfer });
    await expect(page.getByText("Drop files to upload", { exact: true })).toBeVisible();
    await page.locator("body").dispatchEvent("drop", { dataTransfer: transfer });
    await expect(page.getByText("Drop files to upload", { exact: true })).toBeHidden();
    for (const name of names) {
      await expect(page.getByRole("link", { name: new RegExp(name) })).toBeVisible();
    }
    await expect(page.locator(".queue-item-status").filter({ hasText: "Uploaded" })).toHaveCount(2);
  } finally {
    await transfer.dispose();
    const response = await request.get("/api/documents");
    const { documents } = await response.json();
    for (const document of documents.filter((item: { title: string }) => names.includes(item.title))) {
      await request.delete(`/api/documents/${document.id}`);
    }
  }
});

test.describe("document lifecycle", () => {
  test.describe.configure({ mode: "serial" });
  const title = uniqueTitle("lifecycle");

  test("upload through the file chooser shows the document", async ({
    page,
  }) => {
    await uploadViaUi(page, title);
    await expect(page.getByRole("link", { name: new RegExp(title) })).toBeVisible();
  });

  test("detail page opens", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page).toHaveURL(/\/documents\/[0-9a-f-]{36}$/);
    await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
    const processing = page.getByRole("region", { name: "Processing", exact: true });
    await expect(processing).toBeVisible();
    await expect(processing.getByText("Text extraction", { exact: true })).toBeVisible();
    await expect(processing.getByText("Search indexing", { exact: true })).toBeVisible();
  });

  test("search finds the document by title", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: /Search documents/ }).click();
    await page.getByPlaceholder("Search documents").last().fill(title);
    await expect(
      page.getByTestId("command-palette").getByText(title),
    ).toBeVisible();
  });

  test("moving to Trash removes the document from the archive", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Move to Trash" }).click();
    await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
    await expect(page.getByText(title)).toHaveCount(0);
    await page.getByRole("link", { name: "Trash", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Trash", exact: true })).toBeVisible();
    await expect(page.getByRole("article").filter({ hasText: title })).toBeVisible();
  });
});

test.describe("JSON API", () => {
  test("list and get return the document shape", async ({ page, request }) => {
    const title = uniqueTitle("api");
    await uploadViaUi(page, title);

    const list = await request.get("/api/documents");
    expect(list.ok()).toBeTruthy();
    const body = await list.json();
    expect(body.count).toBe(body.documents.length);
    const listed = body.documents.find((d: { title: string }) => d.title === title);
    expect(listed).toBeDefined();

    const get = await request.get(`/api/documents/${listed.id}`);
    expect(get.ok()).toBeTruthy();
    expect(await get.json()).toMatchObject({
      id: listed.id,
      title,
      mime_type: "application/pdf",
      original_filename: "sample.pdf",
    });

    await request.delete(`/api/documents/${listed.id}`);
  });
});

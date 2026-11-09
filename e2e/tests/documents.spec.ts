import { readFileSync } from "node:fs";
import type { APIRequestContext, Page } from "@playwright/test";
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

test("document date can be saved and cleared without changing import time", async ({ page, request }, testInfo) => {
  const title = uniqueTitle("document-date");
  await uploadViaUi(page, title);
  const response = await request.get("/api/documents");
  const { documents } = await response.json();
  const original = documents.find((document: { title: string }) => document.title === title);
  expect(original).toBeDefined();
  try {
    await page.goto(`/documents/${original.id}`);
    await page.getByLabel("Document date", { exact: true }).fill("2000-02-29");
    await Promise.all([
      page.waitForResponse((response) => response.request().method() === "POST" && response.url().endsWith(`/documents/${original.id}/date`)),
      page.getByRole("button", { name: "Save date", exact: true }).click(),
    ]);
    await expect(page.getByLabel("Document date", { exact: true })).toHaveValue("2000-02-29");
    const dated = await (await request.get(`/api/documents/${original.id}`)).json();
    expect(dated.document_date).toBe("2000-02-29");
    expect(dated.created_at).toBe(original.created_at);
    await page.screenshot({ path: testInfo.outputPath("document-date.png") });
    await page.getByRole("button", { name: "Clear date", exact: true }).click();
    await expect(page.getByLabel("Document date", { exact: true })).toHaveValue("");
    const cleared = await (await request.get(`/api/documents/${original.id}`)).json();
    expect(cleared.document_date).toBeNull();
    expect(cleared.created_at).toBe(original.created_at);
  } finally {
    await request.delete(`/api/documents/${original.id}`);
  }
});

test("review can retry a failed write and keeps the selected document", async ({ page, request }) => {
  const title = uniqueTitle("review-retry");
  await uploadViaUi(page, title);
  const { documents } = await (await request.get("/api/documents")).json();
  const original = documents.find((document: { title: string }) => document.title === title);
  const reviewPath = `**/documents/${original.id}/review`;
  try {
    await page.goto(`/documents/${original.id}`);
    await page.route(reviewPath, route => route.abort());
    await page.getByRole("button", { name: "Mark reviewed", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText("Could not save review");
    expect((await (await request.get(`/api/documents/${original.id}`)).json()).reviewed).toBe(false);
    await page.unroute(reviewPath);
    await page.getByRole("button", { name: "Mark reviewed", exact: true }).click();
    await expect(page.getByRole("button", { name: "Mark unreviewed", exact: true })).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`/documents/${original.id}$`));
    const reviewed = await (await request.get(`/api/documents/${original.id}`)).json();
    expect(reviewed.reviewed).toBe(true);
    expect(reviewed.created_at).toBe(original.created_at);
    await page.getByRole("button", { name: "Mark unreviewed", exact: true }).click();
    await expect(page.getByRole("button", { name: "Mark reviewed", exact: true })).toBeVisible();
  } finally {
    await request.delete(`/api/documents/${original.id}`);
  }
});

// A PDF of blank pages; poppler rebuilds the missing cross-reference table
function blankPdf(pages: number): Buffer {
  const kids = Array.from({ length: pages }, (_, i) => `${i + 3} 0 R`).join(" ");
  let pdf = `%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [${kids}] /Count ${pages} >> endobj\n`;
  for (let i = 0; i < pages; i++) {
    pdf += `${i + 3} 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >> endobj\n`;
  }
  return Buffer.from(`${pdf}trailer << /Root 1 0 R >>\n%%EOF\n`, "latin1");
}

test("a long PDF offers the previewed pages and the original for the rest", async ({ page, request }) => {
  const title = uniqueTitle("long-pdf");
  const upload = await request.post("/api/documents/upload", {
    multipart: { title, file: { name: "long.pdf", mimeType: "application/pdf", buffer: blankPdf(51) } },
  });
  expect(upload.ok()).toBeTruthy();
  const { id } = await upload.json();
  try {
    await expect
      .poll(async () => (await (await request.get(`/api/documents/${id}`)).json()).page_count, { timeout: 30_000 })
      .toBe(51);

    await page.goto(`/documents/${id}`);
    const pages = page.getByRole("region", { name: "Pages" });
    await expect(pages.getByText("Page 1 of 50")).toBeVisible();
    await expect(pages.getByRole("button", { name: "Page 50", exact: true })).toHaveCount(1);
    await expect(pages.getByRole("button", { name: "Page 51", exact: true })).toHaveCount(0);
    await expect(pages.getByText("Previews show the first 50 of 51 pages.")).toBeVisible();
    await expect(pages.getByRole("link", { name: "Download the original" })).toHaveAttribute(
      "href",
      new RegExp(`/api/documents/${id}/download$`),
    );
    expect((await request.get(`/documents/${id}/page/51/thumbnail`)).status()).toBe(404);
  } finally {
    await request.delete(`/api/documents/${id}`);
  }
});

test.describe("an open detail page follows processing", () => {
  const sections = ["facts", "pages", "processing", "text"];

  async function upload(request: APIRequestContext, buffer: Buffer): Promise<string> {
    const response = await request.post("/api/documents/upload", {
      multipart: { title: uniqueTitle("live"), file: { name: "live.pdf", mimeType: "application/pdf", buffer } },
    });
    expect(response.ok()).toBeTruthy();
    const { id } = await response.json();
    await settled(request, id);
    return id;
  }

  async function settled(request: APIRequestContext, id: string) {
    await expect
      .poll(async () => (await (await request.get(`/api/documents/${id}`)).json()).ocr_status, { timeout: 30_000 })
      .toMatch(/^(done|failed|skipped)$/);
  }

  // Tag the rendered sections, so a live replacement is told apart from the first render
  async function markStale(page: Page, id: string) {
    await page.evaluate(
      ({ id, sections }) => {
        for (const section of sections) {
          document.getElementById(`document-${id}-${section}`)!.dataset.stale = "true";
        }
      },
      { id, sections },
    );
  }

  async function expectRefreshed(page: Page, id: string) {
    for (const section of sections) {
      await expect(page.locator(`#document-${id}-${section}:not([data-stale])`)).toHaveCount(1, { timeout: 30_000 });
    }
  }

  test("reprocessing refreshes the sections and keeps the page and the edits", async ({ page, request }) => {
    const id = await upload(request, blankPdf(2));
    try {
      await page.goto(`/documents/${id}`);
      const pages = page.getByRole("region", { name: "Pages" });
      await pages.getByRole("button", { name: "Page 2", exact: true }).click();
      await expect(pages.getByText("Page 2 of 2")).toBeVisible();
      const date = page.getByLabel("Document date");
      await date.fill("2024-05-06");
      await markStale(page, id);
      const navigations: string[] = [];
      page.on("framenavigated", (frame) => frame === page.mainFrame() && navigations.push(frame.url()));

      expect((await page.request.post(`/documents/${id}/reprocess`, { maxRedirects: 0 })).status()).toBe(303);
      await expectRefreshed(page, id);
      await settled(request, id);

      await expect(pages.getByText("Page 2 of 2")).toBeVisible();
      await expect(pages.getByRole("button", { name: "Page 2", exact: true })).toHaveAttribute("aria-current", "true");
      await expect(page.getByRole("region", { name: "Processing", exact: true })).toBeVisible();
      await expect(date).toHaveValue("2024-05-06");
      expect(navigations).toEqual([]);
    } finally {
      await request.delete(`/api/documents/${id}`);
    }
  });

  test("a failed read shows its notice without a reload", async ({ page, request }) => {
    const id = await upload(request, Buffer.from("not a pdf\n"));
    try {
      await page.goto(`/documents/${id}`);
      await markStale(page, id);

      expect((await page.request.post(`/documents/${id}/reprocess`, { maxRedirects: 0 })).status()).toBe(303);
      await expectRefreshed(page, id);
      await settled(request, id);
      await expect(page.getByText("Text extraction did not finish")).toBeVisible({ timeout: 30_000 });
    } finally {
      await request.delete(`/api/documents/${id}`);
    }
  });
});

import { expect, test, uniqueTitle, samplePdf } from "./fixtures";
import type { APIRequestContext, Page, Route } from "@playwright/test";

const uploadPath = "**/documents/upload";

async function openUploadDialog(page: Page, title: string) {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Upload document", exact: true })
    .click();
  await page.getByLabel("Title (optional)").fill(title);
  await page.getByLabel("File", { exact: true }).setInputFiles(samplePdf);
}

async function expectFailedUploadPreserved(page: Page, title: string) {
  const dialog = page.locator('[data-upload-modal-target="modal"]');
  const status = dialog.getByRole("status");
  const titleInput = page.getByLabel("Title (optional)");
  const fileInput = page.getByLabel("File", { exact: true });
  const submit = dialog.getByRole("button", { name: "Upload", exact: true });

  await expect(dialog).toBeVisible();
  await expect(status).toHaveText("Couldn't upload. Try again.");
  await expect(titleInput).toHaveValue(title);
  await expect
    .poll(() =>
      fileInput.evaluate(
        (input: HTMLInputElement) => input.files?.[0]?.name ?? "",
      ),
    )
    .toBe("sample.pdf");
  await expect(submit).toBeEnabled();
}

async function expectSuccessfulRetry(page: Page, title: string) {
  const dialog = page.locator('[data-upload-modal-target="modal"]');
  await page.unroute(uploadPath);
  await dialog.getByRole("button", { name: "Upload", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByLabel("Title (optional)")).toHaveValue("");
  await expect
    .poll(() =>
      page
        .getByLabel("File", { exact: true })
        .evaluate((input: HTMLInputElement) => input.files?.length ?? 0),
    )
    .toBe(0);

  await expect
    .poll(async () => {
      const response = await page.request.get("/api/documents");
      const { documents } = await response.json();
      return documents.some(
        (document: { title: string }) => document.title === title,
      );
    })
    .toBe(true);
}

async function deleteUploadedDocument(
  request: APIRequestContext,
  title: string,
) {
  const response = await request.get("/api/documents");
  const { documents } = await response.json();
  const document = documents.find(
    (item: { title: string; id: string }) => item.title === title,
  );
  if (document) await request.delete(`/api/documents/${document.id}`);
}

test("successful modal upload closes and clears its fields", async ({
  page,
  request,
}) => {
  const title = uniqueTitle("upload-success");
  await openUploadDialog(page, title);

  try {
    const dialog = page.locator('[data-upload-modal-target="modal"]');
    await dialog.getByRole("button", { name: "Upload", exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByLabel("Title (optional)")).toHaveValue("");
    await expect
      .poll(() =>
        page
          .getByLabel("File", { exact: true })
          .evaluate((input: HTMLInputElement) => input.files?.length ?? 0),
      )
      .toBe(0);
    await expect
      .poll(async () => {
        const response = await request.get("/api/documents");
        const { documents } = await response.json();
        return documents.some(
          (document: { title: string }) => document.title === title,
        );
      })
      .toBe(true);
  } finally {
    await deleteUploadedDocument(request, title);
  }
});

test("server rejection preserves modal upload fields for retry", async ({
  page,
  request,
}) => {
  const title = uniqueTitle("upload-rejected");
  const rejectWithServerError = (route: Route) =>
    route.fulfill({
      status: 500,
      contentType: "text/plain",
      body: "Upload rejected",
    });

  await openUploadDialog(page, title);
  await page.route(uploadPath, rejectWithServerError);
  try {
    await page
      .locator('[data-upload-modal-target="modal"]')
      .getByRole("button", { name: "Upload", exact: true })
      .click();
    await expectFailedUploadPreserved(page, title);
    await expectSuccessfulRetry(page, title);
  } finally {
    await page.unroute(uploadPath);
    await deleteUploadedDocument(request, title);
  }
});

test("transport failure preserves modal upload fields for retry", async ({
  page,
  request,
}) => {
  const title = uniqueTitle("upload-transport");
  const abortUpload = (route: Route) => route.abort("failed");

  await openUploadDialog(page, title);
  await page.route(uploadPath, abortUpload);
  try {
    await page
      .locator('[data-upload-modal-target="modal"]')
      .getByRole("button", { name: "Upload", exact: true })
      .click();
    await expectFailedUploadPreserved(page, title);
    await expectSuccessfulRetry(page, title);
  } finally {
    await page.unroute(uploadPath);
    await deleteUploadedDocument(request, title);
  }
});

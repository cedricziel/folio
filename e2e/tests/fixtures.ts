import path from "node:path";
import { expect, test as base, type Page } from "@playwright/test";

export const samplePdf = path.resolve(__dirname, "../fixtures/sample.pdf");

export const test = base.extend({
  // Resolved at run time: global-setup picks the port after the config loaded.
  baseURL: async ({}, use) => {
    await use(process.env.FOLIO_BASE_URL);
  },
});

export { expect };

export function uniqueTitle(prefix = "e2e"): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

// Extension point: other upload paths (drag and drop, API) can sit next to this one.
export async function uploadViaUi(page: Page, title: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Upload document", exact: true }).click();
  await page.getByLabel("Title (optional)").fill(title);
  await page.getByLabel("File", { exact: true }).setInputFiles(samplePdf);
  await page.getByRole("button", { name: "Upload", exact: true }).click();
  await expect(page.getByText(title, { exact: true }).first()).toBeVisible();

  // The worker finishes asynchronously and SSE then updates the row in place.
  // Waiting for the worker and reloading gives later steps a stable page.
  await expect
    .poll(async () => {
      const res = await page.request.get("/api/documents");
      const { documents } = await res.json();
      const doc = documents.find((d: { title: string }) => d.title === title);
      return doc?.page_count ?? null;
    })
    .not.toBeNull();
  await page.goto("/");
}

import { readFileSync } from "node:fs";
import type { Page } from "@playwright/test";
import { expect, samplePdf, test } from "./fixtures";

async function mixedQueue(page: Page) {
  let finishActive!: () => void;
  const activeResponse = new Promise<void>((resolve) => { finishActive = resolve; });
  const requests: { url: string; body: string }[] = [];
  let failedOnce = false;
  await page.route("**/vaults/*/documents/upload", async (route) => {
    const body = route.request().postDataBuffer()!.toString();
    const boundary = body.slice(0, body.indexOf("\r\n"));
    requests.push({ url: route.request().url(), body: body.split(boundary).join("") });
    if (body.includes('filename="active.pdf"')) await activeResponse;
    if (body.includes('filename="failed.pdf"') && !failedOnce) {
      failedOnce = true;
      await route.fulfill({ status: 503, body: "Unavailable" });
      return;
    }
    await route.fulfill({ contentType: "text/vnd.turbo-stream.html", body: "" });
  });
  await page.goto("/");
  const buffer = readFileSync(samplePdf);
  await page.locator('[data-dropzone-target="fileInput"]').setInputFiles([
    { name: "completed.pdf", mimeType: "application/pdf", buffer },
    { name: "failed.pdf", mimeType: "application/pdf", buffer },
    { name: "active.pdf", mimeType: "application/pdf", buffer },
    { name: "pending.pdf", mimeType: "application/pdf", buffer },
  ]);
  const queue = page.getByRole("region", { name: "Uploads", exact: true });
  const row = (name: string) => queue.locator(".queue-item").filter({ hasText: name });
  await expect(row("completed.pdf")).toContainText("Uploaded");
  await expect(row("failed.pdf")).toContainText("Couldn't upload");
  await expect.poll(() => requests.length).toBe(3);
  await expect(row("pending.pdf")).toContainText("Waiting");
  return { queue, row, requests, finishActive };
}

test("dismissing finished uploads keeps active, waiting and failed uploads", async ({ page }) => {
  const { queue, row, finishActive } = await mixedQueue(page);
  try {
    await queue.getByRole("button", { name: "Dismiss finished", exact: true }).click();
    await expect(row("completed.pdf")).toHaveCount(0);
    await expect(row("failed.pdf")).toContainText("Couldn't upload");
    await expect(row("active.pdf")).toContainText("Uploading");
    await expect(row("pending.pdf")).toContainText("Waiting");
    finishActive();
    await expect(row("active.pdf")).toContainText("Uploaded");
    await expect(row("pending.pdf")).toContainText("Uploaded");
    await expect(row("failed.pdf").getByRole("button", { name: "Retry", exact: true })).toBeVisible();
  } finally {
    finishActive();
  }
});

test("cancelling pending uploads preserves active and failed uploads and shows cancelled outcomes", async ({ page }) => {
  const { queue, row, requests, finishActive } = await mixedQueue(page);
  try {
    await queue.getByRole("button", { name: "Cancel pending", exact: true }).click();
    await expect(row("pending.pdf")).toContainText("Cancelled");
    await expect(row("active.pdf")).toContainText("Uploading");
    await expect(row("failed.pdf")).toContainText("Couldn't upload");
    await expect(row("completed.pdf")).toContainText("Uploaded");
    await expect(queue.getByRole("button", { name: "Cancel pending", exact: true })).toBeDisabled();
    await page.locator('[data-dropzone-target="fileInput"]').setInputFiles({
      name: "new-pending.pdf",
      mimeType: "application/pdf",
      buffer: readFileSync(samplePdf),
    });
    await expect(row("new-pending.pdf")).toContainText("Waiting");
    await expect(queue.getByRole("button", { name: "Cancel pending", exact: true })).toBeEnabled();
    await queue.getByRole("button", { name: "Cancel pending", exact: true }).click();
    await expect(row("new-pending.pdf")).toContainText("Cancelled");
    await expect(row("active.pdf")).toContainText("Uploading");
    await expect(row("failed.pdf")).toContainText("Couldn't upload");
    await expect(queue.getByRole("button", { name: "Cancel pending", exact: true })).toBeDisabled();
    finishActive();
    await expect(row("active.pdf")).toContainText("Uploaded");
    expect(requests).toHaveLength(3);
    await queue.getByRole("button", { name: "Dismiss finished", exact: true }).click();
    await expect(queue.locator(".queue-item")).toHaveCount(1);
    await expect(row("failed.pdf")).toContainText("Couldn't upload");
  } finally {
    finishActive();
  }
});

test("retrying a failed upload retains the file and destination while another upload is active", async ({ page }) => {
  const { queue, row, requests, finishActive } = await mixedQueue(page);
  try {
    await row("failed.pdf").getByRole("button", { name: "Retry", exact: true }).click();
    await expect(row("failed.pdf")).toContainText("Waiting");
    await expect(row("failed.pdf").getByRole("button", { name: "Retry", exact: true })).toBeHidden();
    await expect(row("active.pdf")).toContainText("Uploading");
    finishActive();
    await expect(row("failed.pdf")).toContainText("Uploaded");
    await expect(row("pending.pdf")).toContainText("Uploaded");
    expect(requests).toHaveLength(5);
    expect(requests[3]).toEqual(requests[1]);
    await queue.getByRole("button", { name: "Dismiss finished", exact: true }).click();
    await expect(queue).toBeHidden();
  } finally {
    finishActive();
  }
});

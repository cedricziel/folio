import { readFileSync } from "node:fs";
import type { APIRequestContext, Page } from "@playwright/test";
import { expect, test, uniqueTitle, samplePdf } from "./fixtures";

type Document = {
  id: string;
  title: string;
  ocr_status: string;
  embed_status: string;
};
type Tag = { id: string; name: string };

async function createTag(
  request: APIRequestContext,
  name: string,
): Promise<Tag> {
  const response = await request.post("/api/tags", { data: { name } });
  expect(response.ok()).toBeTruthy();
  return response.json();
}

async function createDocument(
  request: APIRequestContext,
  title: string,
): Promise<Document> {
  const response = await request.post("/api/documents", {
    data: { title, content: `Content for ${title}` },
  });
  expect(response.ok()).toBeTruthy();
  return response.json();
}

async function setTag(
  request: APIRequestContext,
  documentId: string,
  tagId: string,
  present: boolean,
) {
  const url = present
    ? `/api/documents/${documentId}/tags`
    : `/api/documents/${documentId}/tags/${tagId}`;
  const response = await request.post(
    url,
    present ? { form: { tag_id: tagId } } : {},
  );
  expect(response.ok()).toBeTruthy();
}

async function deleteDocument(request: APIRequestContext, id: string) {
  await request.delete(`/api/documents/${id}`);
}

async function waitForLiveStream(page: Page, url: string) {
  const connected = page.waitForResponse((response) => {
    return (
      new URL(response.url()).pathname.endsWith("/events") &&
      response.status() === 200
    );
  });
  await page.goto(url);
  await connected;
}

test("an unrelated upload stays out of a tag-filtered list without disturbing the active filter", async ({
  page,
  request,
}) => {
  const tag = await createTag(request, uniqueTitle("live-filter"));
  const matching = await createDocument(
    request,
    uniqueTitle("matching-upload"),
  );
  await setTag(request, matching.id, tag.id, true);
  const url = `/?view=all&tag_id=${tag.id}`;
  let unrelated: Document | undefined;

  try {
    await waitForLiveStream(page, url);
    const initialCount = Number(await page.locator("#all-count").innerText());
    const matchingRow = page.locator(`#document-${matching.id}`);
    await expect(matchingRow).toBeVisible();
    await page.locator("body").press("j");
    await expect(matchingRow).toHaveAttribute("aria-selected", "true");
    await page.locator("details.panel > summary").click();
    const dateFrom = page.locator('input[name="document_date_from"]');
    await dateFrom.fill("2025-03-04");
    await dateFrom.focus();

    unrelated = await createDocument(request, uniqueTitle("unrelated-upload"));
    await expect
      .poll(async () => Number(await page.locator("#all-count").innerText()))
      .toBe(initialCount + 1);

    await expect(page.locator(`#document-${unrelated.id}`)).toHaveCount(0);
    await expect(matchingRow).toHaveAttribute("aria-selected", "true");
    await expect(page).toHaveURL(url);
    await expect(dateFrom).toHaveValue("2025-03-04");
    await expect(dateFrom).toBeFocused();
  } finally {
    if (unrelated) await deleteDocument(request, unrelated.id);
    await deleteDocument(request, matching.id);
    await request.delete(`/api/tags/${tag.id}`);
  }
});

test("adding a tag updates the matching live list after a transient refresh failure", async ({
  page,
  request,
}) => {
  const tag = await createTag(request, uniqueTitle("live-tag-on"));
  const document = await createDocument(request, uniqueTitle("tag-on"));
  const url = `/?view=all&tag_id=${tag.id}`;

  try {
    await waitForLiveStream(page, url);
    await expect(page.locator("#documents-empty")).toContainText(
      "No documents match.",
    );

    let interrupted = false;
    await page.route(
      (url) => url.pathname === "/" && url.searchParams.get("tag_id") === tag.id,
      async (route) => {
        interrupted = true;
        await route.abort();
      },
      { times: 1 },
    );
    await setTag(request, document.id, tag.id, true);

    await expect(page.locator(`#document-${document.id}`)).toBeVisible();
    expect(interrupted).toBe(true);
    await expect(page).toHaveURL(url);
  } finally {
    await deleteDocument(request, document.id);
    await request.delete(`/api/tags/${tag.id}`);
  }
});

test("removing a tag removes the row and restores the filtered empty state", async ({
  page,
  request,
}) => {
  const tag = await createTag(request, uniqueTitle("live-tag-off"));
  const document = await createDocument(request, uniqueTitle("tag-off"));
  const url = `/?view=all&tag_id=${tag.id}`;
  await setTag(request, document.id, tag.id, true);

  try {
    await waitForLiveStream(page, url);
    await expect(page.locator(`#document-${document.id}`)).toBeVisible();

    await setTag(request, document.id, tag.id, false);

    await expect(page.locator(`#document-${document.id}`)).toHaveCount(0);
    await expect(page.locator("#documents-empty")).toContainText(
      "No documents match.",
    );
    await expect(page).toHaveURL(url);
  } finally {
    await deleteDocument(request, document.id);
    await request.delete(`/api/tags/${tag.id}`);
  }
});

test("Inbox receives uploads, updates processing, and reconciles review transitions", async ({
  page,
  request,
}) => {
  await waitForLiveStream(page, "/");
  const inboxCount = page.locator("#inbox-count");
  const initialInbox = Number(await inboxCount.textContent());
  const title = uniqueTitle("live-inbox");
  const upload = await request.post("/api/documents/upload", {
    multipart: {
      title,
      file: {
        name: "live-list.pdf",
        mimeType: "application/pdf",
        buffer: readFileSync(samplePdf),
      },
    },
  });
  expect(upload.ok()).toBeTruthy();
  const document: Document = await upload.json();

  try {
    const row = page.locator(`#document-${document.id}`);
    await expect(row).toBeVisible();
    await expect
      .poll(
        async () => {
          const current = await (
            await request.get(`/api/documents/${document.id}`)
          ).json();
          return (
            current.ocr_status === "done" &&
            ["done", "skipped"].includes(current.embed_status)
          );
        },
        { timeout: 30_000 },
      )
      .toBe(true);
    await expect(row).toContainText("Needs a look");

    const reviewed = await request.put(`/api/documents/${document.id}/review`, {
      data: { reviewed: true },
    });
    expect(reviewed.ok()).toBeTruthy();
    await expect(row).toHaveCount(0);
    await expect(inboxCount).toHaveText(String(initialInbox));

    const unreviewed = await request.put(
      `/api/documents/${document.id}/review`,
      { data: { reviewed: false } },
    );
    expect(unreviewed.ok()).toBeTruthy();
    await expect(row).toBeVisible();
    await expect(inboxCount).toHaveText(String(initialInbox + 1));
    await expect(row).toContainText("Needs a look");
    await expect(page).toHaveURL("/");
  } finally {
    await deleteDocument(request, document.id);
  }
});

test("a document date edit adds a row to the open date-filtered Inbox", async ({ page, request }) => {
  const document = await createDocument(request, uniqueTitle("live-date"));
  const date = "2200-02-27";
  const url = `/?document_date_from=${date}&document_date_to=${date}`;
  try {
    await waitForLiveStream(page, url);
    await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
    await expect(page.locator("#documents-rows .f-row")).toHaveCount(0);
    const updated = await request.patch(`/api/documents/${document.id}`, {
      data: { document_date: date },
    });
    expect(updated.ok()).toBeTruthy();
    await expect(page.locator(`#document-${document.id}`)).toBeVisible();
    await expect(page.locator("#documents-empty")).toHaveCount(0);
    await expect(page).toHaveURL(url);
  } finally {
    await deleteDocument(request, document.id);
  }
});

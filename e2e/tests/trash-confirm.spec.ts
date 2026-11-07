import type { APIRequestContext, Page } from "@playwright/test";
import { expect, test, uniqueTitle } from "./fixtures";

type Case = {
  kind: string;
  body: (name: string) => object;
  api: string;
  page: string;
};

const cases: Case[] = [
  {
    kind: "source",
    body: (name) => ({
      name,
      enabled: false,
      config: {
        type: "email",
        host: "imap.example.test",
        username: "me",
        password: "not-a-real-password",
      },
    }),
    api: "/api/sources",
    page: "/settings/sources",
  },
  {
    kind: "workflow",
    body: (name) => ({
      name,
      enabled: false,
      trigger: "document.uploaded",
      actions: [{ type: "webhook", url: "http://localhost:9/hook" }],
    }),
    api: "/api/workflows",
    page: "/workflows",
  },
];

async function create(request: APIRequestContext, c: Case): Promise<string> {
  const response = await request.post(c.api, {
    data: c.body(uniqueTitle(`trash-${c.kind}`)),
  });
  expect(response.ok(), await response.text()).toBeTruthy();
  return (await response.json()).id;
}

// Answers the confirm prompt and returns its text.
async function moveToTrash(page: Page, accept: boolean): Promise<string> {
  const prompt = page.waitForEvent("dialog");
  const click = page
    .getByRole("button", { name: "Move to trash", exact: true })
    .click();
  const dialog = await prompt;
  const message = dialog.message();
  await (accept ? dialog.accept() : dialog.dismiss());
  await click;
  return message;
}

for (const c of cases) {
  test(`moving a ${c.kind} to the trash asks first`, async ({
    page,
    request,
  }) => {
    const id = await create(request, c);
    await page.goto(`${c.page}/${id}`);

    const message = await moveToTrash(page, false);
    expect(message).toContain("restore it from the trash");
    await expect(page).toHaveURL(new RegExp(`${c.page}/${id}$`));
    expect((await request.get(`${c.api}/${id}`)).status()).toBe(200);

    await moveToTrash(page, true);
    await expect(page).toHaveURL(new RegExp(`${c.page}$`));
    expect((await request.get(`${c.api}/${id}`)).status()).toBe(404);
  });
}

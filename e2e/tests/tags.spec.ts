import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

for (const name of [
  'Client "quoted" & <records>',
  'Tag" onpointerover="void(0)" data-injected="yes',
]) {
  test(`tag names remain text in both displays: ${name}`, async ({
    page,
    request,
  }) => {
    const response = await request.post("/api/tags", { data: { name } });
    expect(response.ok()).toBeTruthy();
    const tag = await response.json();
    const title = uniqueTitle("tag-text");
    try {
      await uploadViaUi(page, title);
      await page.getByRole("link", { name: new RegExp(title) }).click();
      await page
        .getByRole("button", { name: "Edit tags", exact: true })
        .click();
      await page
        .locator('[data-tags-target="availableTags"]')
        .getByRole("button", {
          name: `+ ${name}`,
          exact: true,
        })
        .click();

      const tags = page
        .locator(
          `[data-tags-target="list"] [data-tag-id="${tag.id}"], [data-tags-target="currentTags"] [data-tag-id="${tag.id}"]`,
        )
        .filter({ has: page.locator("button") });
      await expect(tags).toHaveCount(2);
      await expect(tags.nth(0)).toContainText(name);
      await expect(tags.nth(1)).toContainText(name);
      const buttons = tags.locator("button");
      await expect(buttons.nth(0)).toHaveAttribute(
        "aria-label",
        `Remove tag ${name}`,
      );
      await expect(buttons.nth(1)).toHaveAttribute(
        "aria-label",
        `Remove tag ${name}`,
      );
      await expect(
        tags.locator("[onpointerover], [data-injected]"),
      ).toHaveCount(0);
      await buttons.nth(1).click();
      await expect(tags).toHaveCount(0);
      await expect(
        page
          .locator('[data-tags-target="availableTags"]')
          .getByRole("button", { name: `+ ${name}`, exact: true }),
      ).toBeVisible();
    } finally {
      await request.delete(`/api/tags/${tag.id}`);
      const listing = await request.get("/api/documents");
      const { documents } = await listing.json();
      const document = documents.find(
        (item: { title: string }) => item.title === title,
      );
      if (document) await request.delete(`/api/documents/${document.id}`);
    }
  });
}

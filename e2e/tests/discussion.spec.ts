import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { uniqueTitle, uploadViaUi } from "./fixtures";
import { TourInstance, tour } from "./tour";

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

for (const width of [1280, 390]) {
  test(`discussion CRUD and lost-response retry at ${width}px`, async ({ browser }, testInfo) => {
    test.setTimeout(60_000);
    await instance.ready(browser, { seeded: false });
    const context = await browser.newContext({ baseURL: instance.baseURL, viewport: { width, height: 900 } });
    const page = await context.newPage();
    try {
      await tour.login(page);
      const title = uniqueTitle("discussion");
      await uploadViaUi(page, title);
      await page.getByRole("link", { name: new RegExp(title) }).click();
      await expect(page).toHaveURL(/\/documents\/[0-9a-f-]{36}$/);
      const documentID = page.url().split("/").at(-1)!;
      const notes = page.locator("#document-notes");
      await notes.scrollIntoViewIfNeeded();
      await expect(notes.getByText("No notes yet", { exact: false })).toBeVisible();
      await notes.getByText("Add note", { exact: true }).click();
      await notes.getByLabel("New note", { exact: true }).fill("<script>plain text</script>");
      const entryPath = `**/documents/${documentID}/entries`;
      let loseResponse = true;
      await page.route(entryPath, async (route) => {
        if (route.request().method() === "POST" && loseResponse) {
          loseResponse = false;
          await route.fetch({ maxRedirects: 0 });
          await route.abort("failed");
        } else {
          await route.continue();
        }
      });
      await notes.getByRole("button", { name: "Save note", exact: true }).click();
      await expect(notes.getByRole("alert")).toContainText("Your draft is still here");
      await expect(notes.getByLabel("New note", { exact: true })).toHaveValue("<script>plain text</script>");
      await expect(notes.getByLabel("New note", { exact: true })).toHaveAttribute("readonly", "");
      await notes.getByRole("button", { name: "Save note", exact: true }).click();
      await expect(notes.locator("article.entry")).toHaveCount(1);
      await expect(notes.locator("article pre")).toHaveText("<script>plain text</script>");
      await page.unroute(entryPath);
      await notes.getByText("Edit note", { exact: true }).click();
      await notes.getByLabel("Notes text", { exact: true }).fill("Updated annotation");
      await notes.getByRole("button", { name: "Save", exact: true }).click();
      await expect(notes.locator("article pre")).toHaveText("Updated annotation");
      const comments = page.locator("#document-comments");
      await comments.scrollIntoViewIfNeeded();
      await comments.getByText("Add comment", { exact: true }).click();
      await comments.getByLabel("New comment", { exact: true }).fill("A separate discussion");
      await comments.getByRole("button", { name: "Save comment", exact: true }).click();
      await expect(comments.locator("article pre")).toHaveText("A separate discussion");
      const accessibility = await new AxeBuilder({ page }).include("#document-notes").include("#document-comments").analyze();
      expect(accessibility.violations).toEqual([]);
      await page.evaluate(() => {
        (document.activeElement as HTMLElement | null)?.blur();
        window.scrollTo(0, 0);
      });
      await page.screenshot({ path: testInfo.outputPath("discussion.png"), fullPage: true });
      page.once("dialog", (dialog) => dialog.accept());
      await notes.getByRole("button", { name: "Delete note", exact: true }).click();
      await expect(notes.locator("article.entry")).toHaveCount(0);
      await expect(comments.locator("article.entry")).toHaveCount(1);
      await page.request.delete(`/api/documents/${documentID}`);
    } finally { await context.close(); }
  });
}

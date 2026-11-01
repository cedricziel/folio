import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

const palette = (page: import("@playwright/test").Page) =>
  page.getByTestId("command-palette");

test.describe("keyboard shortcuts", () => {
  const title = uniqueTitle("keys");

  test.beforeAll(async ({ browser }) => {
    const page = await browser.newPage({ baseURL: process.env.FOLIO_BASE_URL });
    await uploadViaUi(page, title);
    await page.close();
  });

  test("Control+K opens the palette and Escape closes it", async ({ page }) => {
    await page.goto("/");
    await page.keyboard.press("Control+k");
    await expect(palette(page)).toBeVisible();
    await expect(page.getByPlaceholder("Search documents").last()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(palette(page)).toBeHidden();
  });

  test("/ focuses the search", async ({ page }) => {
    await page.goto("/");
    await page.keyboard.press("/");
    await expect(palette(page)).toBeVisible();
    await expect(page.getByPlaceholder("Search documents").last()).toBeFocused();
  });

  test("u opens the upload dialog", async ({ page }) => {
    await page.goto("/");
    await page.keyboard.press("u");
    await expect(page.getByRole("dialog", { name: "Upload a document" })).toBeVisible();
  });

  test("g then i, a, s and w navigate", async ({ page }) => {
    // Turbo swaps the page after the URL changes, so wait for each heading before the next chord.
    await page.goto("/settings");
    await page.keyboard.press("g");
    await page.keyboard.press("i");
    await expect(page.getByRole("heading", { name: "Inbox", level: 1 })).toBeVisible();
    await expect(page).toHaveURL(/\/$/);

    await page.keyboard.press("g");
    await page.keyboard.press("a");
    await expect(page.getByRole("heading", { name: "All documents", level: 1 })).toBeVisible();
    await expect(page).toHaveURL(/\/\?view=all$/);

    await page.keyboard.press("g");
    await page.keyboard.press("s");
    await expect(page).toHaveURL(/\/settings$/);
    await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();

    await page.keyboard.press("g");
    await page.keyboard.press("w");
    await expect(page).toHaveURL(/\/workflows$/);
    await expect(page.getByRole("heading", { name: "Workflows", level: 1 })).toBeVisible();
  });

  test("j and k move the selection and Enter opens the document", async ({ page }) => {
    await page.goto("/?view=all");
    const rows = page.locator("#documents-rows .f-row");
    await expect(rows.first()).toBeVisible();

    await page.keyboard.press("j");
    await expect(rows.first()).toHaveAttribute("aria-selected", "true");
    await expect(rows.first()).toBeFocused();

    if ((await rows.count()) > 1) {
      await page.keyboard.press("j");
      await expect(rows.nth(1)).toHaveAttribute("aria-selected", "true");
      await expect(rows.first()).not.toHaveAttribute("aria-selected", "true");
      await page.keyboard.press("k");
      await expect(rows.first()).toHaveAttribute("aria-selected", "true");
    }

    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/documents\/[0-9a-f-]{36}$/);
  });

  test("? lists the shortcuts in a dialog that Escape closes", async ({ page }) => {
    await page.goto("/");
    await page.keyboard.press("Shift+?");
    const dialog = page.getByRole("dialog", { name: "Keyboard shortcuts" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Go to inbox")).toBeVisible();

    // The dialog traps focus: tabbing never leaves it.
    for (let i = 0; i < 4; i++) await page.keyboard.press("Tab");
    expect(
      await page.evaluate(() => document.activeElement?.closest("dialog") !== null),
    ).toBe(true);

    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });

  test("shortcuts do nothing while typing", async ({ page }) => {
    await page.goto("/");
    await page.keyboard.press("/");
    const input = page.getByPlaceholder("Search documents").last();
    await input.pressSequentially("u?/gi");
    await expect(input).toHaveValue("u?/gi");
    await expect(page.getByRole("dialog", { name: "Upload a document" })).toBeHidden();
    await expect(page.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeHidden();
    await expect(page).toHaveURL(/\/$/);
  });

  test("# asks before deleting and Keep leaves the document", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: new RegExp(title) }).click();
    await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();

    let browserDialog = false;
    page.on("dialog", () => (browserDialog = true));

    await page.keyboard.press("#");
    const confirm = page.getByRole("alertdialog", { name: "Delete this document?" });
    await expect(confirm).toBeVisible();
    await expect(confirm).toContainText(title);
    await page.getByRole("button", { name: "Keep" }).click();
    await expect(confirm).toBeHidden();

    await page.keyboard.press("Control+Backspace");
    await expect(confirm).toBeVisible();
    await confirm.getByRole("button", { name: "Delete" }).click();
    await expect(page).toHaveURL(/\/$/);
    await expect(page.getByText(title)).toHaveCount(0);
    expect(browserDialog).toBe(false);
  });
});

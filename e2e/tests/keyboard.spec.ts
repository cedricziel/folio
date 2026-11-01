import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

// Focus management for the dialogs, the command palette and the mobile drawer.

const focusIsInside = (selector: string) =>
  `document.querySelector(${JSON.stringify(selector)})?.contains(document.activeElement) ?? false`;

test.describe("dialogs", () => {
  test("upload dialog takes focus, traps Tab, closes on Escape and gives focus back", async ({
    page,
  }) => {
    await page.goto("/");
    const trigger = page.getByRole("button", {
      name: "Upload document",
      exact: true,
    });
    await trigger.focus();
    await trigger.press("Enter");

    const dialog = page.getByRole("dialog", { name: "Upload a document" });
    await expect(dialog).toBeVisible();
    await expect(page.getByLabel("Title (optional)")).toBeFocused();

    for (let i = 0; i < 12; i++) {
      await page.keyboard.press("Tab");
      expect(await page.evaluate(focusIsInside("dialog[open]"))).toBe(true);
    }

    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test("tags dialog opens with focus inside and Escape returns it", async ({
    page,
  }) => {
    const title = uniqueTitle("tags-dialog");
    await uploadViaUi(page, title);
    await page.getByRole("link", { name: new RegExp(title) }).click();

    const opener = page.getByRole("button", { name: "Edit tags" });
    await opener.click();
    const dialog = page.getByRole("dialog", { name: "Edit tags" });
    await expect(dialog).toBeVisible();
    expect(await page.evaluate(focusIsInside("dialog[open]"))).toBe(true);

    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(opener).toBeFocused();
  });
});

test.describe("mobile drawer", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("closed drawer is skipped by Tab; opening moves focus in and Escape returns it", async ({
    page,
  }) => {
    await page.goto("/");
    const menu = page.getByRole("button", { name: "Open menu" });
    await expect(menu).toHaveAttribute("aria-expanded", "false");
    await expect(menu).toHaveAttribute("aria-controls", "sidebar");

    await menu.focus();
    await page.keyboard.press("Tab");
    await expect(
      page.getByRole("button", { name: /Search documents/ }),
    ).toBeFocused();

    await menu.focus();
    await page.keyboard.press("Enter");
    await expect(menu).toHaveAttribute("aria-expanded", "true");
    expect(await page.evaluate(focusIsInside("#sidebar"))).toBe(true);
    for (let i = 0; i < 25; i++) {
      await page.keyboard.press("Tab");
      expect(await page.evaluate(focusIsInside("#sidebar"))).toBe(true);
    }

    await page.keyboard.press("Escape");
    await expect(menu).toHaveAttribute("aria-expanded", "false");
    await expect(menu).toBeFocused();
  });
});

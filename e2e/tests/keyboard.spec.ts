import { expect, test, uniqueTitle, uploadViaUi } from "./fixtures";

// Focus management for the dialogs, the command palette and the mobile drawer.

// Native modal dialogs may hand Tab to the browser chrome (activeElement is
// then the body), but never to the inert page behind them.
const focusIsInside = (selector: string) =>
  `(() => { const a = document.activeElement; return a === document.body || (document.querySelector(${JSON.stringify(selector)})?.contains(a) ?? false); })()`;

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

test.describe("command palette", () => {
  test("keeps Tab inside, closes on Escape and restores focus", async ({
    page,
  }) => {
    await page.goto("/");
    const trigger = page.getByRole("button", { name: /Search documents/ });
    await trigger.focus();
    await page.keyboard.press("Enter");

    const input = page.getByRole("combobox", { name: "Search documents" });
    await expect(input).toBeFocused();
    for (let i = 0; i < 6; i++) {
      await page.keyboard.press("Tab");
      expect(await page.evaluate(focusIsInside("dialog[open]"))).toBe(true);
    }

    await page.keyboard.press("Escape");
    await expect(input).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test("is a combobox over a named listbox with the first result selected", async ({
    page,
  }) => {
    const title = uniqueTitle("palette");
    await uploadViaUi(page, title);
    await page.getByRole("button", { name: /Search documents/ }).click();

    const input = page.getByRole("combobox", { name: "Search documents" });
    await expect(input).toHaveAttribute("aria-controls", "palette-results");
    await input.fill(title);

    const list = page.getByRole("listbox", { name: "Documents" });
    const first = list.getByRole("option").first();
    await expect(first).toContainText(title);
    await expect(first).toHaveAttribute("aria-selected", "true");
    await expect(input).toHaveAttribute(
      "aria-activedescendant",
      (await first.getAttribute("id"))!,
    );
    await expect(input).toHaveAttribute("aria-expanded", "true");
    expect(
      await list.evaluate((el) =>
        [...el.children].every((c) => c.getAttribute("role") === "option"),
      ),
    ).toBe(true);
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

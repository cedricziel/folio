import { expect, test } from "./fixtures";

for (const reducedMotion of ["no-preference", "reduce"] as const) {
  test.describe(`motion: ${reducedMotion}`, () => {
    test.use({ reducedMotion });

    test("dialog dismissal fades without delaying focus restoration", async ({
      page,
    }) => {
      await page.goto("/");
      const trigger = page.getByRole("button", {
        name: "Upload document",
        exact: true,
      });
      await trigger.click();
      const dialog = page.locator("dialog[aria-labelledby='upload-title']");
      await dialog.evaluate(async (el) => {
        await Promise.all(
          el.getAnimations().map((animation) => animation.finished),
        );
      });
      const fading = await dialog.evaluate((el) => {
        (
          el.querySelector(
            "[data-action='click->upload-modal#close']",
          ) as HTMLElement
        ).click();
        return el
          .getAnimations()
          .some((animation) =>
            (animation.effect as KeyframeEffect)
              .getKeyframes()
              .some((frame) => frame.opacity !== undefined),
          );
      });
      expect(fading).toBe(reducedMotion === "no-preference");
      await expect(trigger).toBeFocused();
      await expect(dialog).toBeHidden();
    });

    test("toast dismissal completes with motion enabled or disabled", async ({
      page,
    }) => {
      await page.goto("/");
      await page.evaluate(() =>
        (window as any).showToast("Motion test", "info", 0),
      );
      const toast = page.locator(".toast");
      await toast.evaluate(async (el) => {
        await Promise.all(
          el.getAnimations().map((animation) => animation.finished),
        );
      });
      const fading = await toast.evaluate((el) => {
        (el.querySelector("button") as HTMLElement).click();
        return el.isConnected && el.getAnimations().length > 0;
      });
      expect(fading).toBe(reducedMotion === "no-preference");
      await expect(toast).toHaveCount(0);
    });

    test("only new streamed rows fade, not replacements or duplicate insertions", async ({
      page,
    }) => {
      await page.goto("/");
      const states = await page.evaluate(async () => {
        const { renderStreamMessage } = await import("/static/vendor/turbo.js");
        const row =
          '<a class="f-row" id="document-motion-test" href="#">Motion test</a>';
        async function render(action: string, target: string) {
          const rendered = new Promise<void>((resolve) => {
            document.addEventListener(
              "turbo:before-stream-render",
              (event: any) => {
                const original = event.detail.render;
                event.detail.render = async (stream: Element) => {
                  await original(stream);
                  resolve();
                };
              },
              { once: true },
            );
          });
          renderStreamMessage(
            `<turbo-stream action="${action}" target="${target}"><template>${row}</template></turbo-stream>`,
          );
          await rendered;
          const element = document.getElementById("document-motion-test")!;
          const animations = element.getAnimations();
          const fading = animations.length > 0;
          await Promise.all(animations.map((animation) => animation.finished));
          return fading;
        }
        return [
          await render("prepend", "documents-rows"),
          await render("replace", "document-motion-test"),
          await render("prepend", "documents-rows"),
        ];
      });
      expect(states).toEqual([reducedMotion === "no-preference", false, false]);
    });

    test("mobile drawer backdrop transitions with the panel", async ({
      page,
    }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto("/");
      const menu = page.getByRole("button", { name: "Open menu" });
      const opening = await menu.evaluate((el: HTMLElement) => {
        getComputedStyle(document.querySelector(".scrim")!).opacity;
        el.click();
        return document.querySelector(".scrim")!.getAnimations().length > 0;
      });
      expect(opening).toBe(reducedMotion === "no-preference");
      await page.locator(".scrim").evaluate(async (el) => {
        await Promise.all(
          el.getAnimations().map((animation) => animation.finished),
        );
      });
      const closing = await page
        .locator(".scrim")
        .evaluate((el: HTMLElement) => {
          el.click();
          return el.getAnimations().length > 0;
        });
      expect(closing).toBe(reducedMotion === "no-preference");
      await expect(menu).toHaveAttribute("aria-expanded", "false");
      await expect(menu).toBeFocused();
      await expect(page.locator(".scrim")).toBeHidden();
    });
  });
}

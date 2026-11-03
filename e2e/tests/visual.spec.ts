import { expect, test } from "@playwright/test";
import { loadDiscussion } from "./fixtures";
import { config, tour, TourInstance, viewportsFor } from "./tour";

// Baselines are rendered on Linux (CI or the Playwright container, see
// `make e2e-update-snapshots`); font rasterisation differs on macOS and Windows.
test.skip(
  process.platform !== "linux",
  "visual baselines are Linux-only; run make e2e-update-snapshots",
);

// The first test creates the owner and seeds seven documents.
test.describe.configure({ timeout: 90_000 });

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

for (const screen of config.screens.filter((s) => s.snapshot)) {
  for (const viewport of viewportsFor(screen, ["desktop", "mobile"])) {
    for (const theme of config.themes) {
      test(`snapshot: ${screen.name} ${viewport.name} ${theme}`, async ({ browser }) => {
        await instance.ready(browser, { seeded: true });
        const context = await instance.context(browser, screen, viewport, theme);
        const page = await context.newPage();
        await tour.openScreen(page, screen);
        await loadDiscussion(page);
        await expect(page).toHaveScreenshot(`${screen.name}-${viewport.name}-${theme}.png`, {
          fullPage: true,
          animations: "disabled",
          caret: "hide",
          mask: config.mask.map((selector) => page.locator(selector)),
          maxDiffPixelRatio: 0.002,
        });
        await context.close();
      });
    }
  }
}

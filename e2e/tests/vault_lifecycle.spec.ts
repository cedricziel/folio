import { expect, test } from "@playwright/test";
import { TourInstance, tour } from "./tour";

const instance = new TourInstance();
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

for (const width of [1280, 390]) {
  test(`owners can retain and restore a vault at ${width}px`, async ({ browser }) => {
    await instance.ready(browser, { seeded: false });
    const page = await browser.newPage({ baseURL: instance.baseURL, viewport: { width, height: 900 } });
    try {
      await tour.login(page);
      await page.goto("/settings/vaults");
      const name = `vault-lifecycle-${width}-${Date.now()}`;
      await page.getByLabel("Vault name", { exact: true }).fill(name);
      await page.getByRole("button", { name: "Create vault", exact: true }).click();
      const active = page.getByRole("region", { name, exact: true });
      await expect(active).toBeVisible();
      await expect(active.getByRole("button", { name: "Add member", exact: true })).toBeVisible();
      await active.getByRole("button", { name: "Delete vault", exact: true }).click();
      await expect(active).toBeHidden();
      const deleted = page.getByRole("region", { name: `Deleted ${name}`, exact: true });
      await expect(deleted).toBeVisible();
      await deleted.getByRole("button", { name: "Restore vault", exact: true }).click();
      await expect(active).toBeVisible();
      await expect(active).toContainText(`Restored ${name}`);
    } finally { await page.close(); }
  });
}

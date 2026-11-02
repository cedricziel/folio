import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

declare global {
  interface Window {
    searchJsonGate: {
      armed: boolean;
      started: boolean;
      done: boolean;
      release: () => void;
    };
  }
}

async function delayJson(page: Page, query: string | null) {
  await page.addInitScript((query) => {
    const nativeFetch = window.fetch.bind(window);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const state = window.searchJsonGate = {
      armed: false, started: false, done: false, release,
    };
    window.fetch = async (...args) => {
      const response = await nativeFetch(...args);
      const url = new URL(response.url);
      const matches = query === null
        ? url.pathname.endsWith("/api/documents")
        : url.pathname.endsWith("/api/documents/search") && url.searchParams.get("q") === query;
      if (state.armed && matches) {
        state.armed = false;
        const nativeJson = response.json.bind(response);
        response.json = async () => {
          const data = await nativeJson();
          state.started = true;
          await gate;
          state.done = true;
          return data;
        };
      }
      return response;
    };
  }, query);
}

async function releaseJson(page: Page) {
  await page.evaluate(() => window.searchJsonGate.release());
  await page.waitForFunction(() => window.searchJsonGate.done);
  await page.evaluate(() => new Promise(requestAnimationFrame));
}

const recent = { id: "recent-document", title: "Recent insurance policy" };
const older = { id: "older-document", title: "Older invoice" };
const latest = { id: "latest-document", title: "Latest receipt" };

async function mockDocuments(page: Page) {
  await page.route("**/api/documents", (route) => route.fulfill({
    json: { documents: [recent] },
  }));
  await page.route("**/api/documents/search?*", (route) => route.fulfill({
    json: { results: [{ document: new URL(route.request().url()).searchParams.get("q") === "older" ? older : latest, score: 1 }] },
  }));
  await page.goto("/");
  await page.evaluate(() => { window.searchJsonGate.armed = true; });
  await page.getByRole("button", { name: /Search documents/ }).click();
}

test("delayed recent-document JSON cannot replace search results", async ({ page }) => {
  await delayJson(page, null);
  await mockDocuments(page);
  await page.waitForFunction(() => window.searchJsonGate.started);
  await page.getByRole("combobox", { name: "Search documents" }).fill("latest");
  const results = page.locator("#palette-results [role='option']");
  await expect(results).toHaveText([latest.title]);

  await releaseJson(page);

  await expect(results).toHaveText([latest.title]);
});

test("delayed older search JSON cannot replace the latest search results", async ({ page }) => {
  await delayJson(page, "older");
  await mockDocuments(page);
  await expect(page.locator("#palette-results [role='option']")).toHaveText([recent.title]);
  const input = page.getByRole("combobox", { name: "Search documents" });
  await input.fill("older");
  await page.waitForFunction(() => window.searchJsonGate.started);
  await input.fill("latest");
  const results = page.locator("#palette-results [role='option']");
  await expect(results).toHaveText([latest.title]);

  await releaseJson(page);

  await expect(results).toHaveText([latest.title]);
});

test("reopening the palette ignores delayed recent JSON from its previous opening", async ({ page }) => {
  await delayJson(page, null);
  let recentDocument = recent;
  await page.route("**/api/documents", (route) => route.fulfill({
    json: { documents: [recentDocument] },
  }));
  await page.goto("/");
  await page.evaluate(() => { window.searchJsonGate.armed = true; });
  await page.getByRole("button", { name: /Search documents/ }).click();
  await page.waitForFunction(() => window.searchJsonGate.started);

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("command-palette")).toBeHidden();
  recentDocument = latest;
  await page.getByRole("button", { name: /Search documents/ }).click();
  const results = page.locator("#palette-results [role='option']");
  await expect(results).toHaveText([latest.title]);
  await releaseJson(page);

  await expect(results).toHaveText([latest.title]);
});

test.describe("search availability", () => {
  test.beforeEach(async ({ page }) => {
    await page.route("**/api/documents", (route) => route.fulfill({
      json: { documents: [recent] },
    }));
    await page.goto("/");
    await page.getByRole("button", { name: /Search documents/ }).click();
    await expect(page.locator("#palette-results [role='option']")).toHaveText([recent.title]);
  });

  test("a successful empty search reports no documents instead of recent matches", async ({ page }) => {
    await page.route("**/api/documents/search?*", (route) => route.fulfill({
      json: { results: [] },
    }));

    await page.getByRole("combobox", { name: "Search documents" }).fill("insurance");

    await expect(page.getByTestId("command-palette").getByRole("status")).toHaveText("No documents found.");
    await expect(page.locator("#palette-results [role='option']")).toHaveCount(0);
  });

  test("an unavailable search labels its matching recent documents", async ({ page }) => {
    await page.route("**/api/documents/search?*", (route) => route.abort("failed"));

    await page.getByRole("combobox", { name: "Search documents" }).fill("insurance");

    await expect(page.locator("#palette-results [role='option']")).toHaveText([recent.title]);
    await expect(page.locator("[data-command-palette-target='heading']")).toHaveText("Matching recent documents");
    await expect(page.getByTestId("command-palette").getByRole("status")).toHaveText("Search is unavailable. Showing recent matches.");
  });

  test("an unavailable search distinguishes no recent matches from an empty search", async ({ page }) => {
    await page.route("**/api/documents/search?*", (route) => route.fulfill({ status: 503 }));

    await page.getByRole("combobox", { name: "Search documents" }).fill("absent");

    await expect(page.getByTestId("command-palette").getByRole("status")).toHaveText("Search is unavailable. No recent matches found.");
    await expect(page.locator("#palette-results [role='option']")).toHaveCount(0);
  });

  test("an unavailable search reports when recent documents also fail to load", async ({ page }) => {
    await page.route("**/api/documents/search?*", (route) => route.fulfill({ status: 503 }));
    await page.route("**/api/documents", (route) => route.fulfill({ status: 503 }));

    await page.getByRole("combobox", { name: "Search documents" }).fill("insurance");

    await expect(page.getByTestId("command-palette").getByRole("status")).toHaveText("Search is unavailable. Could not load recent documents.");
    await expect(page.locator("#palette-results [role='option']")).toHaveCount(0);
  });
});

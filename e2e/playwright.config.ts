import { defineConfig, devices } from "@playwright/test";

// The instance is started by global-setup.ts, which exports FOLIO_BASE_URL.
// baseURL is resolved lazily so the config can load before the server exists.
export default defineConfig({
  testDir: "./tests",
  globalSetup: "./global-setup.ts",
  globalTeardown: "./global-teardown.ts",
  // Tests share one instance and one database; keep them ordered and isolated by unique titles.
  fullyParallel: false,
  workers: 1,
  timeout: 20_000,
  expect: { timeout: 7_000 },
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI
    ? [["list"], ["html", { open: "never" }]]
    : [["list"]],
  use: {
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    // Extension point: add webkit/firefox or mobile (PWA) projects here.
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
});

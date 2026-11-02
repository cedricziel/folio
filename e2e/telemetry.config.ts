import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  testMatch: "telemetry.spec.ts",
  workers: 1,
  timeout: 30_000,
  expect: { timeout: 15_000 },
  reporter: "list",
  projects: [{ name: "chromium", use: devices["Desktop Chrome"] }],
});

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import type { Browser, BrowserContext } from "@playwright/test";

// screens.json and the helpers are shared with the verify skill's screen tour.
const scripts = path.resolve(__dirname, "../../.claude/skills/verify/scripts");
// eslint-disable-next-line @typescript-eslint/no-require-imports
export const tour = require(path.join(scripts, "tour.cjs"));
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { seed } = require(path.join(scripts, "seed.cjs"));

export type Viewport = {
  name: string;
  width: number;
  height: number;
  isMobile?: boolean;
  hasTouch?: boolean;
};
export type Screen = {
  name: string;
  path: string;
  phase: "empty" | "seeded";
  auth: "on" | "off" | "any";
  loggedOut?: boolean;
  snapshot?: boolean;
  viewports?: string[];
  status?: number;
};

export const config = tour.loadConfig() as {
  viewports: Viewport[];
  themes: ("light" | "dark")[];
  mask: string[];
  screens: Screen[];
};

// A separate instance with auth on, so the shared e2e instance's data
// (unique titles from other specs) never leaks into screenshots.
export class TourInstance {
  baseURL = "";
  private envFile = "";
  private storageState:
    Awaited<ReturnType<BrowserContext["storageState"]>> | undefined;
  private seeded = false;

  start() {
    this.envFile = execFileSync(path.join(scripts, "up.sh"), {
      encoding: "utf8",
      env: { ...process.env, FOLIO_VERIFY_AUTH_REQUIRED: "true" },
    }).trim();
    const vars = Object.fromEntries(
      readFileSync(this.envFile, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((l) => l.split(/=(.*)/s).slice(0, 2)),
    );
    this.baseURL = vars.BASE_URL;
    tour.useSetupToken(vars.VERIFY_DIR);
  }

  stop() {
    if (this.envFile)
      execFileSync(path.join(scripts, "down.sh"), [this.envFile], {
        stdio: "inherit",
      });
  }

  ownerReady() {
    return !!this.storageState;
  }

  async hasDocuments(browser: Browser) {
    const context = await browser.newContext({ baseURL: this.baseURL, storageState: this.storageState });
    const res = await context.request.get("/api/documents");
    const count = res.ok() ? (await res.json()).documents.length : 0;
    await context.close();
    return count > 0;
  }

  // Creates the owner, signs in and seeds once; safe to call from every test.
  async ready(browser: Browser, { seeded }: { seeded: boolean }) {
    if (!this.storageState) {
      const context = await browser.newContext({ baseURL: this.baseURL });
      const page = await context.newPage();
      if (!(await tour.ownerExists(context.request)))
        await tour.setupOwner(page);
      if ((await page.request.get("/", { maxRedirects: 0 })).status() !== 200)
        await tour.login(page);
      this.storageState = await context.storageState();
      await context.close();
    }
    if (seeded && !this.seeded) {
      const context = await browser.newContext({
        baseURL: this.baseURL,
        storageState: this.storageState,
      });
      await seed(context.request);
      await context.close();
      this.seeded = true;
    }
  }

  async context(
    browser: Browser,
    screen: Screen,
    viewport: Viewport,
    theme: "light" | "dark",
  ) {
    const context = await browser.newContext({
      ...tour.contextOptions(viewport, theme),
      baseURL: this.baseURL,
      storageState: screen.loggedOut ? undefined : this.storageState,
    });
    await tour.applyTheme(context, theme);
    return context;
  }
}

export function viewportsFor(screen: Screen, names?: string[]): Viewport[] {
  return config.viewports.filter(
    (v) =>
      (!screen.viewports || screen.viewports.includes(v.name)) &&
      (!names || names.includes(v.name)),
  );
}

// Screens that make sense with auth on, in tour order (setup needs a fresh instance).
export const authScreens = config.screens.filter((s) => s.auth !== "off");

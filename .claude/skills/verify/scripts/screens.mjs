#!/usr/bin/env node
// Screen tour: screenshots plus axe results for every screen in screens.json at
// every viewport and theme, against an instance started by up.sh.
//
// Usage: node screens.mjs [env-file] [--seed] [--headed] [--only a,b] [--viewports desktop,mobile]
//                         [--themes light] [--out <dir>]
// env-file defaults to $FOLIO_VERIFY_ENV. FOLIO_VERIFY_HEADED=1 equals --headed.
// Output: $VERIFY_DIR/screens/<screen>--<viewport>--<theme>.png and .axe.json,
// summary.json, and a table on stdout. --out copies the folder somewhere stable.
import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../../..");
const require = createRequire(path.join(repo, "e2e/package.json"));
const { chromium, request: apiRequest } = require("@playwright/test");
const { default: AxeBuilder } = require("@axe-core/playwright");
const tour = require(path.join(here, "tour.cjs"));
const { seed } = require(path.join(here, "seed.cjs"));

const args = process.argv.slice(2);
const flag = (name) => args.includes(`--${name}`);
const option = (name) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : undefined;
};
const list = (name) => option(name)?.split(",").filter(Boolean);
const positional = args.filter(
  (a, i) =>
    !a.startsWith("--") &&
    !args[i - 1]?.match(/^--(only|viewports|themes|out)$/),
);

const envFile = positional[0] ?? process.env.FOLIO_VERIFY_ENV;
if (!envFile) {
  console.error(
    "usage: screens.mjs <env-file> [--seed] [--headed] [--only a,b] [--out dir]",
  );
  process.exit(2);
}
const env = Object.fromEntries(
  readFileSync(envFile, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((l) => l.split(/=(.*)/s).slice(0, 2)),
);
const baseURL = env.BASE_URL;
const outDir = path.join(env.VERIFY_DIR, "screens");
mkdirSync(outDir, { recursive: true });

const headed = flag("headed") || process.env.FOLIO_VERIFY_HEADED === "1";
const config = tour.loadConfig();
const only = list("only");
const viewports = config.viewports.filter(
  (v) => !list("viewports") || list("viewports").includes(v.name),
);
const themes = config.themes.filter(
  (t) => !list("themes") || list("themes").includes(t),
);

const api = await apiRequest.newContext({ baseURL });
tour.useSetupToken(env.VERIFY_DIR);
const probe = await api.get("/", { maxRedirects: 0 });
const authRequired = probe.status() >= 300 && probe.status() < 400;

const browser = await chromium.launch({
  headless: !headed,
  slowMo: headed ? 250 : 0,
});
const results = [];
let storageState;

const pick = (screens) => screens.filter((s) => !only || only.includes(s.name));

async function capture(screen, viewport, theme) {
  if (screen.viewports && !screen.viewports.includes(viewport.name)) return;
  const context = await browser.newContext({
    ...tour.contextOptions(viewport, theme),
    baseURL,
    storageState: screen.loggedOut ? undefined : storageState,
  });
  await tour.applyTheme(context, theme);
  const page = await context.newPage();
  const consoleErrors = [];
  page.on(
    "console",
    (m) =>
      m.type() === "error" &&
      // Debug builds poll a livereload endpoint that 404s here; not a page error.
      !m.location().url?.includes("_tower-livereload") &&
      consoleErrors.push(m.text()),
  );
  page.on("pageerror", (e) => consoleErrors.push(String(e)));
  const id = `${screen.name}--${viewport.name}--${theme}`;
  const row = {
    screen: screen.name,
    viewport: viewport.name,
    theme,
    file: `${id}.png`,
  };
  try {
    const { status } = await tour.openScreen(page, screen);
    row.status = status;
    if (headed) await page.waitForTimeout(1000);
    await page.screenshot({
      path: path.join(outDir, `${id}.png`),
      fullPage: true,
    });
    const axe = await new AxeBuilder({ page }).withTags(tour.axeTags).analyze();
    writeFileSync(
      path.join(outDir, `${id}.axe.json`),
      JSON.stringify(axe.violations, null, 2),
    );
    row.serious = axe.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    ).length;
    row.violations = axe.violations.map(
      (v) => `${v.id}(${v.impact},${v.nodes.length})`,
    );
    row.overflow = await tour.hasHorizontalOverflow(page);
  } catch (e) {
    row.error = String(e).split("\n")[0];
  }
  row.consoleErrors = consoleErrors;
  results.push(row);
  await context.close();
}

async function runGroup(screens) {
  for (const viewport of viewports)
    for (const theme of themes)
      for (const screen of pick(screens))
        await capture(screen, viewport, theme);
}

async function signIn() {
  const context = await browser.newContext({ baseURL });
  const page = await context.newPage();
  if (!(await tour.ownerExists(context.request))) await tour.setupOwner(page);
  if (!(await isSignedIn(page))) await tour.login(page);
  storageState = await context.storageState();
  await context.close();
}

async function isSignedIn(page) {
  const res = await page.request.get("/", { maxRedirects: 0 });
  return res.status() === 200;
}

async function documentCount(request) {
  const res = await request.get("/api/documents");
  return res.ok() ? (await res.json()).documents.length : -1;
}

// 1. Screens that need a fresh instance (setup, empty inbox).
const empty = tour.screensFor(config, { phase: "empty", authRequired });
if (authRequired) {
  const setupFree = !(await tour.ownerExists(api));
  if (setupFree) await runGroup(empty.filter((s) => s.auth === "on"));
  await signIn();
}
const authed = await (
  await browser.newContext({ baseURL, storageState })
).request;
if ((await documentCount(authed)) === 0)
  await runGroup(empty.filter((s) => s.auth === "any"));

// 2. Seed, then everything else.
if (flag("seed")) {
  const r = await seed(authed);
  console.error(
    r.skipped
      ? "screens: data present, seed skipped"
      : "screens: seeded demo data",
  );
  // Let the broken upload fail and SSE settle.
  await new Promise((r) => setTimeout(r, 2000));
}
await runGroup(tour.screensFor(config, { phase: "seeded", authRequired }));

await browser.close();
await api.dispose();

writeFileSync(
  path.join(outDir, "summary.json"),
  JSON.stringify({ baseURL, authRequired, results }, null, 2),
);
const pad = (s, n) => String(s).padEnd(n);
console.log(
  `${pad("screen", 22)}${pad("viewport", 9)}${pad("theme", 6)}${pad("status", 7)}${pad("axe s/c", 8)}${pad("overflow", 9)}console`,
);
for (const r of results)
  console.log(
    `${pad(r.screen, 22)}${pad(r.viewport, 9)}${pad(r.theme, 6)}${pad(r.status ?? "-", 7)}${pad(r.serious ?? "-", 8)}${pad(r.overflow === undefined ? "-" : r.overflow ? "yes" : "no", 9)}${r.error ? `ERROR ${r.error}` : r.consoleErrors.length}`,
  );

let finalDir = outDir;
if (option("out")) {
  finalDir = path.resolve(option("out"));
  mkdirSync(finalDir, { recursive: true });
  cpSync(outDir, finalDir, { recursive: true });
}
console.log(`\nscreens: ${results.length} captures in ${finalDir}`);
process.exit(results.some((r) => r.error) ? 1 : 0);

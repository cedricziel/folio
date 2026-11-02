// Shared by scripts/screens.mjs (verify skill tour) and the e2e a11y/visual specs,
// so screens.json stays the single list of screens.
const { readFileSync } = require("node:fs");
const crypto = require("node:crypto");
const path = require("node:path");

const configPath = path.resolve(__dirname, "../screens.json");
const credentials = {
  username: "verify-owner",
  password: "verify-password-123",
};

function loadConfig() {
  return JSON.parse(readFileSync(configPath, "utf8"));
}

// Which screens apply for this run; `phase` and `auth` as in screens.json.
function screensFor(config, { phase, authRequired }) {
  return config.screens.filter(
    (s) =>
      s.phase === phase &&
      (s.auth === "any" || (s.auth === "on") === authRequired),
  );
}

// Context options for a viewport and theme. The page script reads
// localStorage "folio-theme"; the auth pages follow prefers-color-scheme.
function contextOptions(viewport, theme) {
  return {
    viewport: { width: viewport.width, height: viewport.height },
    isMobile: !!viewport.isMobile,
    hasTouch: !!viewport.hasTouch,
    deviceScaleFactor: 1,
    colorScheme: theme,
    reducedMotion: "reduce",
    locale: "en-US",
    timezoneId: "UTC",
  };
}

async function applyTheme(context, theme) {
  await context.addInitScript((t) => {
    try {
      localStorage.setItem("folio-theme", t);
    } catch {}
  }, theme);
}

// /setup needs the one-time token Folio prints to its log while no user exists.
let setupToken = "";
function useSetupToken(verifyDir) {
  try {
    const log = readFileSync(path.join(verifyDir, "server.log"), "utf8");
    setupToken = log.match(/setup\?token=([A-Za-z0-9_-]+)/)?.[1] ?? "";
  } catch {
    setupToken = "";
  }
  return setupToken;
}
const setupPath = () => (setupToken ? `/setup?token=${setupToken}` : "/setup");

async function ownerExists(request) {
  const res = await request.get(setupPath(), { maxRedirects: 0 });
  return res.status() !== 200;
}

async function setupOwner(page) {
  await page.goto(setupPath());
  await page.getByLabel("Username").fill(credentials.username);
  await page.getByLabel("Password").fill(credentials.password);
  await page.getByRole("button", { name: "Create account" }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/setup"));
}

async function login(page) {
  await page.goto("/login");
  await page.getByLabel("Username").fill(credentials.username);
  await page.getByLabel("Password").fill(credentials.password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
}

async function idBy(request, url, key, name) {
  const res = await request.get(url);
  const body = await res.json();
  const list = Array.isArray(body)
    ? body
    : (body[key] ?? Object.values(body).find(Array.isArray));
  const hit = list.find((x) => (x.title ?? x.name) === name);
  if (!hit) throw new Error(`tour: no ${key} named ${name}`);
  return hit.id;
}

// Registers a public client (DCR) and returns an authorize URL that shows consent.
async function consentPath(request) {
  const reg = await request.post("/oauth/register", {
    data: {
      client_name: "Claude Desktop",
      redirect_uris: ["http://127.0.0.1:33418/callback"],
      token_endpoint_auth_method: "none",
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
    },
  });
  if (!reg.ok())
    throw new Error(`tour: DCR failed ${reg.status()} ${await reg.text()}`);
  const { client_id } = await reg.json();
  const verifier = "verify-code-verifier-0123456789-abcdefghijklmnopqrstuv";
  const challenge = crypto
    .createHash("sha256")
    .update(verifier)
    .digest("base64url");
  const q = new URLSearchParams({
    response_type: "code",
    client_id,
    redirect_uri: "http://127.0.0.1:33418/callback",
    code_challenge: challenge,
    code_challenge_method: "S256",
    state: "verify",
  });
  return `/oauth/authorize?${q}`;
}

async function resolvePath(request, p) {
  const m = p.match(/^\{(\w+)(?::(.*))?\}$/) ?? p.match(/\{(\w+):([^}]*)\}/);
  if (!m) return p;
  const [token, kind, name] = m;
  if (kind === "setup") return setupPath();
  if (kind === "consent") return consentPath(request);
  if (kind === "doc")
    return `/documents/${await idBy(request, "/api/documents", "documents", name)}`;
  if (kind === "workflow")
    return `/workflows/${await idBy(request, "/api/workflows", "workflows", name)}`;
  if (kind === "tag")
    return p.replace(token, await idBy(request, "/api/tags", "tags", name));
  throw new Error(`tour: unknown placeholder ${token}`);
}

async function runSteps(page, steps = []) {
  for (const step of steps) {
    if (step.open === "palette") {
      await page.keyboard.press("ControlOrMeta+k");
      await page.getByRole("dialog", { name: "Search documents" }).waitFor();
    } else if (step.open === "upload") {
      await page
        .getByRole("button", { name: "Upload document", exact: true })
        .click();
    } else if (step.open === "menu") {
      await page.getByRole("button", { name: "Open menu" }).click();
      await page.waitForTimeout(300);
    } else if (step.type !== undefined) {
      await page.keyboard.type(step.type);
    } else if (step.pause) {
      await page.waitForTimeout(step.pause);
    } else if (step.press) {
      await page.keyboard.press(step.press);
    } else if (step.fill) {
      await page.getByLabel(step.fill[0], { exact: true }).fill(step.fill[1]);
    } else if (step.click) {
      await page
        .getByRole("button", { name: step.click, exact: true })
        .or(page.getByRole("link", { name: step.click, exact: true }))
        .first()
        .click();
    } else if (step.wait) {
      await page
        .getByText(step.wait, { exact: false })
        .first()
        .waitFor({ timeout: 10_000 });
    }
  }
}

// Opens a screen: resolves the path, navigates, waits, runs steps, settles fonts and images.
async function openScreen(page, screen) {
  const target = await resolvePath(page.request, screen.path);
  // An empty error body makes Chromium fail the navigation; the screen still shows something.
  const response = await page.goto(target).catch((e) => {
    if (!screen.status) throw e;
    return null;
  });
  if (screen.wait)
    await page
      .getByText(screen.wait, { exact: false })
      .first()
      .waitFor({ timeout: 10_000 });
  await runSteps(page, screen.steps);
  await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all(
      [...document.images]
        .filter((i) => !i.complete)
        .map(
          (i) =>
            new Promise((r) => {
              i.onload = i.onerror = r;
            }),
        ),
    );
  });
  return { status: response?.status() ?? 0, url: target };
}

async function hasHorizontalOverflow(page) {
  return page.evaluate(
    () =>
      document.documentElement.scrollWidth >
      document.documentElement.clientWidth + 1,
  );
}

const axeTags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

module.exports = {
  loadConfig,
  screensFor,
  contextOptions,
  applyTheme,
  useSetupToken,
  ownerExists,
  setupOwner,
  login,
  resolvePath,
  runSteps,
  openScreen,
  hasHorizontalOverflow,
  credentials,
  axeTags,
};

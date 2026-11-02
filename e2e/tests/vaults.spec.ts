import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import type { BrowserContext, Page, TestInfo } from "@playwright/test";
import { expect, samplePdf, test, uniqueTitle } from "./fixtures";

const household = { id: randomUUID(), name: "Household", role: "editor" };
const reference = { id: randomUUID(), name: "Reference", role: "viewer" };
let personal: { id: string; name: string };
let cookies: Awaited<ReturnType<BrowserContext["cookies"]>>;
let database: string;
let personalDocument: { id: string; title: string };
let householdDocument: { id: string; title: string };
let referenceDocument: { id: string; title: string };

function seedVaults(userId: string) {
  // Vault administration has no HTTP endpoint; only the managed fixture database is seeded.
  execFileSync(
    "python3",
    [
      "-c",
      `
import json, sqlite3, sys
from datetime import datetime, timezone
data = json.load(sys.stdin)
now = datetime.now(timezone.utc).isoformat()
with sqlite3.connect(sys.argv[1]) as db:
    for vault in data["vaults"]:
        db.execute("INSERT INTO vaults (id, name, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?)", (vault["id"], vault["name"], data["user"], now, now))
        db.execute("INSERT INTO vault_members (vault_id, principal_type, user_id, role, granted_at) VALUES (?, 'user', ?, 'owner', ?)", (vault["id"], data["user"], now))
`,
      database,
    ],
    {
      input: JSON.stringify({ user: userId, vaults: [household, reference] }),
    },
  );
}

async function capture(page: Page, testInfo: TestInfo, name: string) {
  const directory = process.env.FOLIO_VAULT_SCREENSHOTS_DIR;
  if (directory) mkdirSync(directory, { recursive: true });
  const destination = directory
    ? path.join(directory, name)
    : testInfo.outputPath(name);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: destination, fullPage: true, animations: "disabled" });
  await testInfo.attach(name, { path: destination, contentType: "image/png" });
}

async function openLibrary(page: Page, mobile: boolean) {
  if (mobile) {
    await page.getByRole("button", { name: "Open menu", exact: true }).click();
    await expect.poll(async () => (await page.locator("#sidebar").boundingBox())?.x).toBe(0);
  }
  return page.getByRole("combobox", { name: "Vault", exact: true });
}

test.beforeAll(async ({ browser }) => {
  const envFile = process.env.FOLIO_VERIFY_ENV;
  test.skip(
    !!process.env.E2E_BASE_URL || !envFile,
    "Vault seeding requires the isolated global-setup instance",
  );
  const variables = Object.fromEntries(
    readFileSync(envFile!, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((line) => line.split(/=(.*)/s).slice(0, 2)),
  );
  expect(path.basename(variables.VERIFY_DIR)).toMatch(/^folio-verify\./);
  expect(process.env.FOLIO_BASE_URL).toBe(variables.BASE_URL);
  database = path.join(variables.VERIFY_DIR, "folio.db");
  // Auth-off fixtures do not publish a setup token, so seed a user and use real login.
  const userId = randomUUID();
  const username = `vault-browser-${userId}`;
  const passwordHash = "$argon2id$v=19$m=19456,t=2,p=1$Zm9saW8tdXBncmFkZS10ZXN0$JKrDtfFpUmy4Cj/faWPtM7zwa2jH37D3Sdo3iILWE58";
  execFileSync("python3", ["-c", `
import json, sqlite3, sys, time
user = json.load(sys.stdin)
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("INSERT INTO users (id, username, username_normalized, password_hash, created_at) VALUES (?, ?, ?, ?, ?)", (user["id"], user["name"], user["name"], user["hash"], int(time.time())))
`, database], { input: JSON.stringify({ id: userId, name: username, hash: passwordHash }) });
  const context = await browser.newContext({
    baseURL: process.env.FOLIO_BASE_URL,
  });
  try {
    const page = await context.newPage();
    await page.goto("/login");
    await page.getByLabel("Username").fill(username);
    await page.getByLabel("Password").fill("before-the-upgrade");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page).toHaveURL(`${process.env.FOLIO_BASE_URL}/`);
    cookies = await context.cookies();
    const discovery = await context.request.get("/api/vaults");
    expect(discovery.ok()).toBeTruthy();
    const { vaults, default_vault_id } = await discovery.json();
    personal = vaults.find(
      (vault: { id: string }) => vault.id === default_vault_id,
    );
    expect(personal).toBeDefined();
    seedVaults(userId);
    const createDocument = async (vault: string, title: string) => {
      const response = await context.request.post(
        `/vaults/${vault}/api/documents`,
        {
          data: { title, content: title },
        },
      );
      expect(response.ok()).toBeTruthy();
      return response.json();
    };
    personalDocument = await createDocument(
      personal.id,
      "Private bank statement",
    );
    householdDocument = await createDocument(
      household.id,
      "Household energy statement",
    );
    referenceDocument = await createDocument(
      reference.id,
      "Reference handbook",
    );
    const personalTag = await context.request.post(
      `/vaults/${personal.id}/api/tags`,
      {
        data: { name: `Private finances ${household.id.slice(0, 8)}` },
      },
    );
    const householdTag = await context.request.post(
      `/vaults/${household.id}/api/tags`,
      {
        data: { name: "Home utilities" },
      },
    );
    expect(personalTag.ok()).toBeTruthy();
    expect(householdTag.ok()).toBeTruthy();
    execFileSync("sqlite3", [
      "-cmd", ".timeout 5000", database,
      `UPDATE vault_members SET role = 'editor' WHERE vault_id = '${household.id}';
       UPDATE vault_members SET role = 'viewer' WHERE vault_id = '${reference.id}';`,
    ]);
  } finally {
    await context.close();
  }
});

test.beforeEach(async ({ context }) => {
  await context.addCookies(cookies);
});

for (const viewport of [
  { name: "desktop", width: 1440, height: 1000, mobile: false },
  { name: "mobile", width: 390, height: 844, mobile: true },
]) {
  test(`native vault selector scopes the library on ${viewport.name}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    });
    await page.goto(`/vaults/${personal.id}/?view=all`);
    const selector = await openLibrary(page, viewport.mobile);
    await expect(selector).toHaveValue(personal.id);
    await selector.selectOption(household.id);
    await expect(page).toHaveURL(
      `${process.env.FOLIO_BASE_URL}/vaults/${household.id}/`,
    );
    await expect(page.locator("body")).toHaveAttribute(
      "data-vault-id",
      household.id,
    );
    await expect(page.locator("#documents-rows")).toContainText(
      householdDocument.title,
    );
    await expect(page.locator("#documents-rows")).not.toContainText(
      personalDocument.title,
    );
    await expect(page.locator("#all-count")).toHaveText("1");
    if (viewport.mobile) await openLibrary(page, true);
    await expect(page.locator(".vault-context > .f-meta")).toHaveText("Editor");
    await expect(page.locator(".side-tags")).toContainText("Home utilities");
    await expect(page.locator(".side-tags")).not.toContainText(
      "Private finances",
    );
    await expect(
      page.locator(`#document-${householdDocument.id}`),
    ).toHaveAttribute(
      "href",
      `/vaults/${household.id}/documents/${householdDocument.id}`,
    );
    await capture(page, testInfo, `vault-library-${viewport.name}.png`);
    if (viewport.mobile) await page.keyboard.press("Escape");
    await page.getByRole("button", { name: /Search documents/ }).click();
    await page
      .getByRole("combobox", { name: "Search documents" })
      .fill("Household energy");
    const result = page.locator("#palette-results [role='option']");
    await expect(result).toContainText(householdDocument.title);
    await expect(result).toHaveAttribute(
      "href",
      `/vaults/${household.id}/documents/${householdDocument.id}`,
    );
    await expect(page.getByTestId("command-palette")).not.toContainText(
      personalDocument.title,
    );
  });
}

test("upload destination changes without switching the browsed vault", async ({
  page,
}, testInfo) => {
  const title = uniqueTitle("independent-vault-upload");
  await page.goto(`/vaults/${household.id}/`);
  await page
    .getByRole("button", { name: "Upload document", exact: true })
    .click();
  const destination = page.getByLabel("Upload to", { exact: true });
  await expect(destination).toHaveValue(household.id);
  await expect(
    destination.locator(`option[value='${reference.id}']`),
  ).toHaveCount(0);
  await destination.selectOption(personal.id);
  await expect(page).toHaveURL(
    `${process.env.FOLIO_BASE_URL}/vaults/${household.id}/`,
  );
  await expect(page.locator("#library-vault")).toHaveValue(household.id);
  await page.getByLabel("Title (optional)").fill(title);
  await page.getByLabel("File", { exact: true }).setInputFiles(samplePdf);
  await capture(page, testInfo, "vault-upload-destination-desktop.png");
  const upload = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      response.url().endsWith(`/vaults/${personal.id}/documents/upload`),
  );
  await page.getByRole("button", { name: "Upload", exact: true }).click();
  expect((await upload).ok()).toBeTruthy();
  await expect(
    page.getByRole("dialog", { name: "Upload a document" }),
  ).toBeHidden();
  await expect(page.locator("#documents-rows")).not.toContainText(title);
  await expect(page.locator("#all-count")).toHaveText("1");
  const personalDocuments = await page.request.get(
    `/vaults/${personal.id}/api/documents`,
  );
  const { documents } = await personalDocuments.json();
  expect(documents).toEqual(
    expect.arrayContaining([expect.objectContaining({ title })]),
  );
  const selector = await openLibrary(page, false);
  await selector.selectOption(personal.id);
  await expect(page.locator("#documents-rows")).toContainText(title);
});

test("viewer controls stay read-only and upload destinations stay writable", async ({
  page,
}) => {
  await page.goto(`/vaults/${reference.id}/documents/${referenceDocument.id}`);
  await expect(page.locator(".vault-context > .f-meta")).toHaveText("Viewer");
  await expect(page.getByLabel("From", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Type", { exact: true })).toBeDisabled();
  await expect(
    page
      .locator(".doc-actions")
      .getByRole("button", { name: "Delete", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Edit tags", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Upload document", exact: true })
    .click();
  const destination = page.getByLabel("Upload to", { exact: true });
  await expect(
    destination.locator(`option[value='${reference.id}']`),
  ).toHaveCount(0);
  await expect(destination).not.toHaveValue(reference.id);
});

test("a delayed search from the previous vault cannot populate the next palette", async ({
  page,
}) => {
  let release!: () => void;
  let started!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const requestStarted = new Promise<void>((resolve) => {
    started = resolve;
  });
  await page.route(
    `**/vaults/${personal.id}/api/documents/search?*`,
    async (route) => {
      started();
      await gate;
      await route.fulfill({
        json: { results: [{ document: personalDocument, score: 1 }] },
      });
    },
  );
  try {
    await page.goto(`/vaults/${personal.id}/`);
    await page.getByRole("button", { name: /Search documents/ }).click();
    await page
      .getByRole("combobox", { name: "Search documents" })
      .fill("Private bank");
    await requestStarted;
    await page.keyboard.press("Escape");
    await page
      .getByRole("combobox", { name: "Vault", exact: true })
      .selectOption(household.id);
    await expect(page).toHaveURL(
      `${process.env.FOLIO_BASE_URL}/vaults/${household.id}/`,
    );
  } finally {
    release();
    await page.unrouteAll({ behavior: "wait" });
  }
  await page.getByRole("button", { name: /Search documents/ }).click();
  await page
    .getByRole("combobox", { name: "Search documents" })
    .fill("Household energy");
  await expect(page.locator("#palette-results")).toContainText(
    householdDocument.title,
  );
  await expect(page.locator("#palette-results")).not.toContainText(
    personalDocument.title,
  );
});

import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import type { BrowserContext, Locator, Page } from "@playwright/test";
import { expect, test, uniqueTitle } from "./fixtures";

const classifications = [
  { endpoint: "tags", frame: "tags-list", row: "tag", label: "tag" },
  {
    endpoint: "correspondents",
    frame: "correspondents-list",
    row: "correspondent",
    label: "correspondent",
  },
  {
    endpoint: "document_types",
    frame: "document-types-list",
    row: "document-type",
    label: "document-type",
  },
] as const;
type Classification = (typeof classifications)[number];
type Entry = { id: string; name: string; created_at: string; vault_id: string };

const editable = { id: randomUUID(), name: "Classification workspace" };
const readOnly = { id: randomUUID(), name: "Classification reference" };
let outside: { id: string };
let cookies: Awaited<ReturnType<BrowserContext["cookies"]>>;
const viewerEntries: Record<string, { active: Entry; trashed: Entry }> = {};

function frame(page: Page, classification: Classification): Locator {
  return page.locator(`turbo-frame#${classification.frame}`).first();
}

function activeRow(page: Page, classification: Classification, id: string): Locator {
  return frame(page, classification).locator(`#${classification.row}-${id}`);
}

function trashRow(page: Page, classification: Classification, vault: string, id: string): Locator {
  return page.locator("li.manage-list-item").filter({
    has: page.locator(
      `form[action="/vaults/${vault}/manage/${classification.endpoint}/${id}/restore"]`,
    ),
  });
}

async function createThroughControls(page: Page, classification: Classification, name: string): Promise<Entry> {
  const form = page.locator(
    `form[action="/vaults/${editable.id}/manage/${classification.endpoint}/create"]`,
  );
  await form.getByRole("textbox").fill(name);
  await form.getByRole("button", { name: "Add", exact: true }).click();
  await expect(frame(page, classification).locator("li[id]")).toContainText([name]);
  const response = await page.request.get(`/vaults/${editable.id}/api/${classification.endpoint}`);
  expect(response.ok()).toBeTruthy();
  const entry = (await response.json()).find((candidate: Entry) => candidate.name === name);
  expect(entry).toBeDefined();
  return entry;
}

async function moveToTrash(page: Page, classification: Classification, entry: Entry) {
  const row = activeRow(page, classification, entry.id);
  await expect(row.locator("form[action$='/delete']")).toHaveAttribute(
    "action", `/vaults/${editable.id}/manage/${classification.endpoint}/${entry.id}/delete`,
  );
  page.once("dialog", (dialog) => dialog.accept());
  await row.getByRole("button", { name: `Delete ${classification.label} ${entry.name}`, exact: true }).click();
  await expect(row).toHaveCount(0);
}

async function openTrash(page: Page, classification: Classification) {
  await frame(page, classification).locator("details.manage-trash > summary").click();
}

test.beforeAll(async ({ browser }) => {
  const envFile = process.env.FOLIO_VERIFY_ENV;
  test.skip(
    !!process.env.E2E_BASE_URL || !envFile,
    "Classification lifecycle fixtures require the isolated global-setup instance",
  );
  const variables = Object.fromEntries(
    readFileSync(envFile!, "utf8").split("\n").filter(Boolean)
      .map((line) => line.split(/=(.*)/s).slice(0, 2)),
  );
  expect(path.basename(variables.VERIFY_DIR)).toMatch(/^folio-verify\./);
  expect(process.env.FOLIO_BASE_URL).toBe(variables.BASE_URL);
  const database = path.join(variables.VERIFY_DIR, "folio.db");
  const userId = randomUUID();
  const username = `classification-browser-${userId}`;
  const passwordHash = "$argon2id$v=19$m=19456,t=2,p=1$Zm9saW8tdXBncmFkZS10ZXN0$JKrDtfFpUmy4Cj/faWPtM7zwa2jH37D3Sdo3iILWE58";
  // Only authentication and vault grants are seeded; classifications use the real HTTP mutations.
  execFileSync("python3", ["-c", `
import json, sqlite3, sys, time
from datetime import datetime, timezone
seed = json.load(sys.stdin)
now = datetime.now(timezone.utc).isoformat()
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("INSERT INTO users (id, username, username_normalized, password_hash, created_at) VALUES (?, ?, ?, ?, ?)", (seed["user"], seed["name"], seed["name"], seed["hash"], int(time.time())))
    for vault in seed["vaults"]:
        db.execute("INSERT INTO vaults (id, name, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?)", (vault["id"], vault["name"], seed["user"], now, now))
        db.execute("INSERT INTO vault_members (vault_id, principal_type, user_id, role, granted_at) VALUES (?, 'user', ?, 'owner', ?)", (vault["id"], seed["user"], now))
`, database], { input: JSON.stringify({ user: userId, name: username, hash: passwordHash, vaults: [editable, readOnly] }) });
  const context = await browser.newContext({ baseURL: process.env.FOLIO_BASE_URL });
  try {
    const page = await context.newPage();
    await page.goto("/login");
    await page.getByLabel("Username").fill(username);
    await page.getByLabel("Password").fill("before-the-upgrade");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page).toHaveURL(`${process.env.FOLIO_BASE_URL}/`);
    const discovery = await context.request.get("/api/vaults");
    expect(discovery.ok()).toBeTruthy();
    const catalog = await discovery.json();
    outside = { id: catalog.default_vault_id };
    expect(outside.id).toBeTruthy();
    expect(outside.id).not.toBe(editable.id);
    expect(outside.id).not.toBe(readOnly.id);
    cookies = await context.cookies();
    for (const classification of classifications) {
      const create = async (name: string): Promise<Entry> => {
        const response = await context.request.post(`/vaults/${readOnly.id}/api/${classification.endpoint}`, { data: { name } });
        expect(response.ok()).toBeTruthy();
        return response.json();
      };
      const active = await create(uniqueTitle(`viewer-active-${classification.endpoint}`));
      const trashed = await create(uniqueTitle(`viewer-trashed-${classification.endpoint}`));
      expect((await context.request.delete(`/vaults/${readOnly.id}/api/${classification.endpoint}/${trashed.id}`)).status()).toBe(204);
      viewerEntries[classification.endpoint] = { active, trashed };
    }
    execFileSync("python3", ["-c", `
import json, sqlite3, sys
seed = json.load(sys.stdin)
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("UPDATE vault_members SET role = 'editor' WHERE vault_id = ? AND user_id = ?", (seed["editable"], seed["user"]))
    db.execute("UPDATE vault_members SET role = 'viewer' WHERE vault_id = ? AND user_id = ?", (seed["viewer"], seed["user"]))
`, database], { input: JSON.stringify({ user: userId, editable: editable.id, viewer: readOnly.id }) });
  } finally {
    await context.close();
  }
});

test.beforeEach(async ({ context }) => {
  await context.addCookies(cookies);
});

for (const classification of classifications) {
  test(`${classification.endpoint} controls preserve document references through trash, name reuse and restore`, async ({ page }) => {
    test.setTimeout(45_000);
    const name = uniqueTitle(`lifecycle-${classification.endpoint}`);
    const renamed = `${name}-renamed`;
    await page.goto(`/vaults/${editable.id}/manage`);
    await expect(page.locator("body")).toHaveAttribute("data-vault-id", editable.id);
    const entry = await createThroughControls(page, classification, name);
    const row = activeRow(page, classification, entry.id);
    await row.getByRole("button", { name: `Rename ${classification.label} ${name}`, exact: true }).click();
    await row.locator(".manage-edit-form input[name='name']").fill(renamed);
    await row.getByRole("button", { name: "Save", exact: true }).click();
    await expect(row.locator(".manage-item-name")).toContainText(renamed);
    const activeImage = test.info().outputPath(`${classification.endpoint}-active.png`);
    await frame(page, classification).screenshot({ path: activeImage });
    await test.info().attach(`${classification.endpoint}-active`, { path: activeImage, contentType: "image/png" });
    const updatedResponse = await page.request.get(`/vaults/${editable.id}/api/${classification.endpoint}/${entry.id}`);
    expect(updatedResponse.ok()).toBeTruthy();
    const updated = await updatedResponse.json();
    expect(updated).toMatchObject({ id: entry.id, vault_id: editable.id, name: renamed, created_at: entry.created_at });

    const documentResponse = await page.request.post(`/vaults/${editable.id}/api/documents`, {
      data: { title: uniqueTitle("classification-reference"), content: "Retained classification reference" },
    });
    expect(documentResponse.ok()).toBeTruthy();
    const document = await documentResponse.json();
    await page.goto(`/vaults/${editable.id}/documents/${document.id}`);
    if (classification.endpoint === "tags") {
      await page.getByRole("button", { name: "Edit tags", exact: true }).click();
      await page.locator('[data-tags-target="availableTags"]').getByRole("button", { name: `+ ${renamed}`, exact: true }).click();
      await expect(page.locator(`[data-tags-target="list"] > [data-tag-id="${entry.id}"]`)).toContainText(renamed);
    } else {
      const selector = page.getByLabel(classification.endpoint === "correspondents" ? "From" : "Type", { exact: true });
      await selector.selectOption(entry.id);
      await expect(selector).toHaveValue(entry.id);
    }
    await page.goto(`/vaults/${editable.id}/manage`);
    await moveToTrash(page, classification, updated);
    await page.goto(`/vaults/${editable.id}/documents/${document.id}`);
    await expect(page.getByRole("heading", { level: 1, name: document.title })).toBeVisible();
    if (classification.endpoint === "tags") {
      await expect(page.locator(`[data-tags-target="list"] > [data-tag-id="${entry.id}"]`)).toContainText(`${renamed} (deleted)`);
      await page.getByRole("button", { name: "Edit tags", exact: true }).click();
      await expect(page.locator(`[data-tags-target="availableTags"] [data-tag-id="${entry.id}"]`)).toHaveCount(0);
    } else {
      const retained = page.getByLabel(classification.endpoint === "correspondents" ? "From" : "Type", { exact: true }).locator(`option[value="${entry.id}"]`);
      await expect(retained).toHaveText(`${renamed} (deleted)`);
      await expect(retained).toBeDisabled();
    }

    const scopedOut = await page.request.get(`/vaults/${outside.id}/api/${classification.endpoint}/${entry.id}`);
    expect(scopedOut.status()).toBe(404);
    expect((await page.request.post(`/vaults/${outside.id}/api/${classification.endpoint}/${entry.id}/restore`)).status()).toBe(404);
    const outsideCreate = await page.request.post(`/vaults/${outside.id}/api/${classification.endpoint}`, { data: { name: renamed } });
    expect(outsideCreate.ok()).toBeTruthy();
    const outsideEntry = await outsideCreate.json();
    await page.goto(`/vaults/${editable.id}/manage`);
    await expect(activeRow(page, classification, outsideEntry.id)).toHaveCount(0);
    const replacement = await createThroughControls(page, classification, renamed);
    expect(replacement.id).not.toBe(entry.id);
    await openTrash(page, classification);
    const retained = trashRow(page, classification, editable.id, entry.id);
    await expect(retained).toContainText("Deleted");
    await retained.getByRole("button", { name: "Restore", exact: true }).click();
    await expect(frame(page, classification).getByRole("alert")).toHaveText("Restore failed. An active entry already uses this name.");
    await openTrash(page, classification);
    await expect(retained).toBeVisible();
    await expect(activeRow(page, classification, replacement.id)).toBeVisible();
    const conflictImage = test.info().outputPath(`${classification.endpoint}-restore-conflict.png`);
    await frame(page, classification).screenshot({ path: conflictImage });
    await test.info().attach(`${classification.endpoint}-restore-conflict`, { path: conflictImage, contentType: "image/png" });
    await moveToTrash(page, classification, replacement);
    await openTrash(page, classification);
    await retained.getByRole("button", { name: "Restore", exact: true }).click();
    await expect(activeRow(page, classification, entry.id)).toBeVisible();
    await expect(frame(page, classification).getByRole("alert")).toHaveCount(0);
    const restoredResponse = await page.request.get(`/vaults/${editable.id}/api/${classification.endpoint}/${entry.id}`);
    expect(restoredResponse.ok()).toBeTruthy();
    expect(await restoredResponse.json()).toMatchObject({ id: entry.id, name: renamed, vault_id: editable.id, created_at: entry.created_at, deleted_at: null });
    await page.goto(`/vaults/${editable.id}/documents/${document.id}`);
    if (classification.endpoint === "tags") {
      const restored = page.locator(`[data-tags-target="list"] > [data-tag-id="${entry.id}"]`);
      await expect(restored).toContainText(renamed);
      await expect(restored).not.toContainText("(deleted)");
    } else {
      const restored = page.getByLabel(classification.endpoint === "correspondents" ? "From" : "Type", { exact: true });
      await expect(restored).toHaveValue(entry.id);
      await expect(restored.locator(`option[value="${entry.id}"]`)).toHaveText(renamed);
    }
  });

  test(`viewer can inspect ${classification.endpoint} trash but cannot mutate or restore`, async ({ page }) => {
    const { active, trashed } = viewerEntries[classification.endpoint];
    await page.goto(`/vaults/${readOnly.id}/manage`);
    await expect(page.locator("body")).toHaveAttribute("data-vault-id", readOnly.id);
    await expect(activeRow(page, classification, active.id)).toContainText(active.name);
    await expect(frame(page, classification).getByRole("button", { name: /^Rename |^Delete / })).toHaveCount(0);
    await expect(page.locator(`form[action="/vaults/${readOnly.id}/manage/${classification.endpoint}/create"]`)).toHaveCount(0);
    await openTrash(page, classification);
    await expect(frame(page, classification).locator("details.manage-trash")).toContainText(trashed.name);
    await expect(frame(page, classification).getByRole("button", { name: "Restore", exact: true })).toHaveCount(0);
    const endpoint = `/vaults/${readOnly.id}/api/${classification.endpoint}`;
    expect((await page.request.post(endpoint, { data: { name: uniqueTitle("forbidden-create") } })).status()).toBe(403);
    expect((await page.request.put(`${endpoint}/${active.id}`, { data: { name: "Forbidden rename" } })).status()).toBe(403);
    expect((await page.request.delete(`${endpoint}/${active.id}`)).status()).toBe(403);
    expect((await page.request.post(`${endpoint}/${trashed.id}/restore`)).status()).toBe(403);
    expect((await page.request.post(`/vaults/${readOnly.id}/manage/${classification.endpoint}/${trashed.id}/restore`)).status()).toBe(403);
    const live = await page.request.get(`${endpoint}/${active.id}`);
    expect(live.ok()).toBeTruthy();
    expect(await live.json()).toMatchObject({ id: active.id, name: active.name, deleted_at: null });
    const trash = await page.request.get(`${endpoint}/trash`);
    expect(trash.ok()).toBeTruthy();
    expect(await trash.json()).toEqual(expect.arrayContaining([expect.objectContaining({ id: trashed.id, name: trashed.name })]));
  });
}

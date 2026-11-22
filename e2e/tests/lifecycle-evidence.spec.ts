import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { config, tour, TourInstance } from "./tour";

const instance = new TourInstance();
const evidenceDirectory = path.resolve(__dirname, "../test-results/lifecycle-evidence");
const legacyActor = "former-household-paperwork-account-with-a-retained-external-identity<archived>";
const screen = { name: "sources", path: "/settings/sources", phase: "seeded", auth: "on" } as const;

test.describe.configure({ timeout: 90_000 });
test.beforeAll(() => instance.start());
test.afterAll(() => instance.stop());

function seedHistory(vaultID: string, sourceID: string, legacySourceID: string, legacyWorkflowID: string) {
  expect(path.basename(instance.verifyDirectory)).toMatch(/^folio-verify\./);
  // Historical records need no live folder, mailbox, webhook receiver, or user archive.
  execFileSync("python3", ["-c", `
import json, sqlite3, sys
seed = json.load(sys.stdin)
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("PRAGMA foreign_keys = ON")
    db.execute("UPDATE ingestion_sources SET created_by = NULL, updated_by = ? WHERE id = ? AND vault_id = ?", (seed["actor"], seed["legacy_source"], seed["vault"]))
    db.execute("UPDATE workflows SET created_by = NULL, updated_by = ? WHERE id = ? AND vault_id = ?", (seed["actor"], seed["legacy_workflow"], seed["vault"]))
    db.execute("INSERT INTO ingest_items (id, vault_id, source_id, source_kind, source_managed, locator_key, attempt_id, started_at, lease_expires_at, decision_fingerprint, state) VALUES (?, ?, ?, 'folder', 1, ?, ?, '2026-11-02T10:00:00Z', '2026-11-02T10:10:00Z', ?, 'failed')", (seed["item"], seed["vault"], seed["source"], "a" * 64, seed["attempt"], "b" * 64))
    db.execute("INSERT INTO ingest_outcomes (vault_id, source_id, item_id, attempt_id, event_id, occurred_at, outcome, document_id) VALUES (?, ?, ?, ?, ?, '2026-11-02T10:00:00Z', 'failed', NULL)", (seed["vault"], seed["source"], seed["item"], seed["attempt"], seed["event"]))
`, path.join(instance.verifyDirectory, "folio.db")], {
    input: JSON.stringify({
      vault: vaultID, source: sourceID, legacy_source: legacySourceID,
      legacy_workflow: legacyWorkflowID, actor: legacyActor,
      item: randomUUID(), attempt: randomUUID(), event: randomUUID(),
    }),
  });
}

async function capture(page: Page, info: TestInfo, name: string) {
  mkdirSync(evidenceDirectory, { recursive: true });
  const stem = path.join(evidenceDirectory, name);
  const axe = await new AxeBuilder({ page }).withTags(tour.axeTags).analyze();
  const blocking = axe.violations.filter((violation) => ["serious", "critical"].includes(violation.impact ?? ""));
  const overflow = await tour.hasHorizontalOverflow(page);
  const clipped = await page.locator(".lifecycle-meta").evaluateAll((elements) =>
    elements.filter((element) => element.scrollWidth > element.clientWidth + 1 || element.scrollHeight > element.clientHeight + 1)
      .map((element) => element.textContent),
  );
  writeFileSync(`${stem}.json`, JSON.stringify({ url: page.url(), overflow, clipped, axe }, null, 2));
  await page.screenshot({ path: `${stem}.png`, fullPage: true, animations: "disabled" });
  await info.attach(name, { path: `${stem}.png`, contentType: "image/png" });
  await info.attach(`${name}-accessibility`, { path: `${stem}.json`, contentType: "application/json" });
  expect(blocking, `${name}: serious/critical accessibility violations`).toEqual([]);
  expect(overflow, `${name}: horizontal page overflow`).toBe(false);
  expect(clipped, `${name}: lifecycle attribution is clipped`).toEqual([]);
}

for (const viewport of config.viewports.filter((candidate) => ["desktop", "mobile"].includes(candidate.name))) {
  for (const theme of config.themes) {
    test(`retained lifecycle evidence: ${viewport.name} ${theme}`, async ({ browser }, info) => {
      await instance.ready(browser, { seeded: false });
      const context = await instance.context(browser, screen, viewport, theme);
      const page = await context.newPage();
      const prefix = `${viewport.name}-${theme}`;
      try {
        const vaultResponse = await page.request.post("/api/vaults", { data: { name: `Lifecycle workspace ${prefix}` } });
        expect(vaultResponse.status()).toBe(201);
        const vault = await vaultResponse.json();
        const scope = `/vaults/${vault.id}`;
        const sourceInput = { enabled: false, config: { type: "folder", path: "/unused/lifecycle-fixture" } };
        const sourceResponse = await page.request.post(`${scope}/api/sources`, { data: { ...sourceInput, name: "Household archive" } });
        expect(sourceResponse.status()).toBe(201);
        const source = await sourceResponse.json();
        expect(source.created_by).toBeTruthy();
        const legacySourceResponse = await page.request.post(`${scope}/api/sources`, { data: { ...sourceInput, name: "Earlier household archive" } });
        expect(legacySourceResponse.status()).toBe(201);
        const legacySource = await legacySourceResponse.json();
        const workflowInput = { enabled: false, trigger: "document.uploaded", conditions: {}, actions: [{ type: "webhook", url: "http://127.0.0.1:1/unused" }] };
        const workflowResponse = await page.request.post(`${scope}/api/workflows`, { data: { ...workflowInput, name: "Household notifications" } });
        expect(workflowResponse.status()).toBe(201);
        const workflow = await workflowResponse.json();
        const legacyWorkflowResponse = await page.request.post(`${scope}/api/workflows`, { data: { ...workflowInput, name: "Earlier household notifications" } });
        expect(legacyWorkflowResponse.status()).toBe(201);
        const legacyWorkflow = await legacyWorkflowResponse.json();
        seedHistory(vault.id, source.id, legacySource.id, legacyWorkflow.id);

        await page.goto(`${scope}/settings/sources`);
        const activeSource = page.locator(".source-row").filter({ has: page.locator(`form[action$='/${source.id}/delete']`) });
        await expect(activeSource).toContainText(`by ${source.created_by}`);
        await expect(page.locator(".lifecycle-meta").filter({ hasText: "System or unknown" })).toContainText(legacyActor);
        await capture(page, info, `${prefix}-sources-active`);
        await activeSource.getByRole("button", { name: "Move to trash", exact: true }).click();
        await page.getByRole("link", { name: "Deleted sources", exact: true }).click();
        const retainedSource = page.locator(".source-row").filter({ has: page.locator(`form[action$='/${source.id}/restore']`) });
        await expect(retainedSource).toContainText(`by ${source.created_by}`);
        await capture(page, info, `${prefix}-sources-trash`);
        await retainedSource.getByRole("link", { name: "Outcomes", exact: true }).click();
        await expect(page).toHaveURL(`${instance.baseURL}${scope}/settings/sources/${source.id}/outcomes`);
        await expect(page.getByText("This source is in Trash. Earlier outcomes remain available.", { exact: true })).toBeVisible();
        await expect(page.getByText("Import failed", { exact: true })).toBeVisible();
        await capture(page, info, `${prefix}-source-retained-outcomes`);
        await page.getByRole("link", { name: "Sources", exact: true }).click();
        await expect(page).toHaveURL(`${instance.baseURL}${scope}/settings/sources?deleted=true`);
        await expect(page.getByRole("button", { name: "Restore", exact: true })).toBeVisible();

        await page.goto(`${scope}/workflows`);
        const activeWorkflow = page.locator(".f-row").filter({ has: page.locator(`a[href$='/${workflow.id}/history']`) });
        await expect(activeWorkflow).toContainText(`by ${workflow.created_by}`);
        if (viewport.name === "mobile") {
          const details = await activeWorkflow.locator(".row-main").boundingBox();
          const actions = await activeWorkflow.locator(".form-actions").boundingBox();
          expect(details).not.toBeNull();
          expect(actions).not.toBeNull();
          expect(actions!.y).toBeGreaterThanOrEqual(details!.y + details!.height);
        }
        await expect(page.locator(".lifecycle-meta").filter({ hasText: "System or unknown" })).toContainText(legacyActor);
        await capture(page, info, `${prefix}-workflows-active`);
        expect((await page.request.delete(`${scope}/api/workflows/${workflow.id}`)).status()).toBe(204);
        await page.getByRole("link", { name: "Deleted workflows", exact: true }).click();
        const retainedWorkflow = page.locator(".f-row").filter({ has: page.locator(`a[href$='/${workflow.id}/history']`) });
        await expect(retainedWorkflow).toContainText(`by ${workflow.created_by}`);
        await capture(page, info, `${prefix}-workflows-trash`);
        await retainedWorkflow.getByRole("link", { name: "Run history", exact: true }).click();
        await expect(page.getByRole("heading", { name: "Household notifications history", exact: true })).toBeVisible();
        await expect(page.locator(".lifecycle-meta")).toContainText(`by ${workflow.created_by}`);
        await capture(page, info, `${prefix}-workflow-retained-history`);
      } finally {
        await context.close();
      }
    });
  }
}

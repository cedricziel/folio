---
name: verify
description: Verify a Folio change in the running app by launching an ephemeral instance (own port, temp DB and storage, no user config), exercising it through the API, SSE and a browser (including a screenshot and axe tour of every screen), then tearing it down. Use after changing routes, templates, the worker, config or storage, and before claiming a feature works. Not for unit tests (use `cargo test`) or for touching a real or deployed Folio.
---

# Verify Folio end to end

Run a throwaway instance, poke it, delete it. Everything uses test data in a temp dir; never point this at a real Folio, the user's `config.toml` or `.env`, or any hosted service.

Scripts live in `.claude/skills/verify/scripts/`. Call them by path from the repo root (or any worktree).

## 1. Run the whole loop

```bash
S=.claude/skills/verify/scripts
$S/with-instance.sh bash -c "$S/smoke.sh \$FOLIO_VERIFY_ENV"
```

`with-instance.sh <cmd>` builds if needed, starts the instance, exports `FOLIO_VERIFY_ENV` (path of the env file), `BASE_URL` and `VERIFY_DIR`, runs the command, and always tears down (trap), even on failure.

## 2. Or step by step

1. `scripts/build.sh` builds `folio-app` once (uses `~/.local/bin/cargo-slot` when present, `CARGO_INCREMENTAL=0`). Set `CARGO_TARGET_DIR` inside the worktree when sharing a machine with other agents. `up.sh` builds on demand if the binary is missing; set `FOLIO_BIN` to use another binary (for example a release build in CI).
2. `env_file=$(scripts/up.sh)` picks a free port, creates `${TMPDIR:-/tmp}/folio-verify.XXXXXX` (DB, storage, log in `server.log`), starts the app, waits for readiness and prints the env file path. The env file defines `BASE_URL`, `PID`, `VERIFY_DIR`, and the effective `FOLIO_VERIFY_AUTO_OCR` so a separately invoked smoke test uses the instance configuration. `source` it in later steps.
3. `scripts/smoke.sh "$env_file"` runs the API checks: upload the fixture PDF through the UI route (so the worker handles it), poll until the worker has set `page_count` (with provider `local` OCR yields no text, so `content` stays null), list and get via `/api`, download and compare bytes, assert an SSE event arrived on `/events`, delete and expect 404.
4. Drive the UI (next section).
5. `scripts/down.sh "$env_file"` kills the PID and removes the temp dir. Idempotent. When using `up.sh` directly, install `trap '$S/down.sh "$env_file"' EXIT` first.

Why the instance is isolated: `Config::load()` reads `./config.toml` from the working directory and `dotenvy` walks up from it looking for `.env`. `up.sh` therefore runs the app from the temp dir with a scrubbed environment (`env -i`) and symlinks `static/` in, since the app serves `./static` relative to the working directory.

Config names are `FOLIO_SERVER__HOST`, `FOLIO_SERVER__PORT`, `FOLIO_DATABASE__URL`, `FOLIO_STORAGE__PATH`, `FOLIO_AI__PROVIDER=local` (no AI calls). The old single-underscore names are still accepted as aliases but are deprecated. New settings go into the env block of `up.sh` only.

## 3. Drive the UI

- With Playwright MCP browser tools available: navigate to `BASE_URL`, snapshot, click through the flow, and save screenshots under `$VERIFY_DIR/shots/`. Prefer accessible roles and labels (`Upload document` button, `Title (optional)` and `File` fields, `Search documents...` trigger).
- Otherwise run the committed suite, which starts its own instance through the same scripts: `make e2e`, or `cd e2e && npx playwright test` (set `E2E_BASE_URL` to reuse an instance you already started). Failure screenshots and traces land in `e2e/test-results/`, the HTML report in `e2e/playwright-report/`.
- Screenshots of a passing flow: `cd e2e && npx playwright screenshot --browser chromium "$BASE_URL" "$VERIFY_DIR/inbox.png"`.

Known quirk: after the worker finishes, an SSE event swaps the list for a variant whose links target the `document-detail` frame, so clicking a document right then shows "Content missing". Wait for processing, then reload, before clicking through.

The Turbo/Stimulus UI updates the DOM after fetches; wait on visible text (`expect(...).toBeVisible()`), not on `networkidle` (debug builds hold a livereload connection open).

## 4. Screen tour (required for UI changes)

Any change to templates, `static/`, CSS or routes that render HTML MUST run the screen tour, and you MUST look at the result before claiming the change works.

```bash
S=.claude/skills/verify/scripts
$S/with-instance.sh bash -c "node $S/screens.mjs \$FOLIO_VERIFY_ENV --seed --out \$PWD/screens-out"
# Auth pages (setup, login, consent) need auth on:
FOLIO_VERIFY_AUTH_REQUIRED=true $S/with-instance.sh bash -c "node $S/screens.mjs \$FOLIO_VERIFY_ENV --seed"
```

- `screens.json` (next to this file) lists the screens, viewports (desktop 1280x800, tablet 768x1024, mobile 390x844) and themes (light, dark). It is also read by `e2e/tests/a11y.spec.ts` and `e2e/tests/visual.spec.ts`, so add a new screen there once and both the tour and CI pick it up.
- `--seed` loads deterministic demo data (`scripts/seed.cjs`: correspondents, types, tags, seven generated PDFs, a workflow, a source, one broken upload). `node $S/seed.cjs $FOLIO_VERIFY_ENV` seeds on its own (auth off only).
- Auth is detected from the instance: with auth on the tour captures `/setup` first, creates a throwaway owner, signs in and captures the rest, including OAuth consent through a dynamic client registration.
- Flags: `--only inbox,document`, `--viewports desktop,mobile`, `--themes dark`, `--headed` (or `FOLIO_VERIFY_HEADED=1`: visible Chromium, slowed down, one second per screen), `--out <dir>` (copy the results somewhere that survives teardown; `down.sh` deletes `$VERIFY_DIR`).
- Output in `$VERIFY_DIR/screens/`: `<screen>--<viewport>--<theme>.png`, the matching `.axe.json` (axe-core violations, WCAG 2.2 AA tags) and `summary.json`. Stdout ends with a table: screen, viewport, theme, HTTP status, serious/critical axe count, horizontal overflow, console errors.

Then read the PNGs (Read tool) for every screen your change touches, at least desktop and mobile in both themes, and report what you saw: layout, overflow, contrast in dark mode, focus and states. A green table is not a substitute for looking. A new serious/critical axe violation or horizontal overflow on mobile is a regression.

## 5. Feature areas as they land

Short notes for features that do not exist on main yet. Check the code first; skip a note whose feature is absent.

- Health endpoints: `up.sh` already prefers `/readyz` once it returns something other than 404. Add `/healthz` and `/readyz` assertions to `smoke.sh`.
- OpenAPI: fetch the served spec and check that `/api/documents` is listed; compare against the shape asserted in `e2e/tests/documents.spec.ts`.
- Auth: `up.sh` runs with `auth.required=false` unless `FOLIO_VERIFY_AUTH_REQUIRED=true` is set. `FOLIO_VERIFY_AUTH_REQUIRED=true $S/with-instance.sh bash -c "$S/auth.sh \$FOLIO_VERIFY_ENV"` exercises the setup token, login, `/api` 401, the public PWA assets, the same-origin check and the registration rate limit (the setup link is read from `server.log`). By hand: open the setup link from the log to create the owner with a throwaway password, then log in. Extend `smoke.sh` with the session cookie or token and `e2e/` with a login fixture. Passkeys, OIDC and forward-auth need a local fake provider or headers set by curl; never a real IdP.
- MCP at `/mcp`: `claude mcp add --transport http folio-verify "$BASE_URL/mcp"` (remove it afterwards) or a JSON-RPC probe: `curl -sS -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' "$BASE_URL/mcp"`.
- Webhooks and workflows: start a local receiver (`python3 -m http.server`, or `nc -l`) on a free port, point the webhook at `http://127.0.0.1:<port>`, run the trigger (upload), and check the receiver log.
- Postgres or S3 adapters: start them with testcontainers or a local container, pass the URL through `up.sh` env, and run `smoke.sh` unchanged; the API contract is the same.
- Pipeline startup configuration: run the smoke test with `FOLIO_VERIFY_OCR_PROVIDER=none FOLIO_VERIFY_AUTO_OCR=false FOLIO_VERIFY_AUTO_EMBED=false`. On the fresh database, `/settings` must select `none` and leave both processing checkboxes unchecked; uploads must finish with skipped OCR and embeddings. The scripts pass these test options through the scrubbed environment.
- Real OCR or embeddings: the throwaway instance uses local Tesseract and no embeddings by default. Test hosted providers only with a dedicated test key supplied by the user, never from the environment you inherited.

## Client telemetry

Run `FOLIO_BIN=<new binary> npm run verify:telemetry --prefix e2e` to exercise
an authenticated throwaway Folio and a local OTLP receiver. The verifier checks
all three browser signals, W3C context, configuration authentication and
server-only export credentials. It creates and cleans up its own ports,
profiles, database and storage. `npm run test:telemetry --prefix e2e` covers
browser disabled, outage and privacy cases without a Folio instance.

Swift telemetry has an opt-in real-network smoke test. Against a separately
started disposable Folio, set `FOLIO_TELEMETRY_SMOKE_URL` and a test-only
`FOLIO_TELEMETRY_SMOKE_TOKEN`, then run
`swift test --package-path apple/FolioKit --filter realRelaySmoke` alone. The
test uses temporary preferences and reports only fresh trace and resource IDs.
Never use the real app profile or a user account for this test. Verify those
IDs in the chosen telemetry destination when validating a deployment.

## Apple navigation

Run `xcrun swift test --package-path apple/FolioKit` for native changes. For list and detail loading, the focused suites are `DocumentListViewModelTests`, `DocumentListFailureTests`, and `DocumentDetailViewModelTests`. The detail suite covers cancellation followed by a successful retry. Run `xcrun swift-format lint` on changed Swift files.

Check navigation in a throwaway simulator with test credentials and an isolated server. Open a document, return to the list, and open it again. Check that rows remain visible and the preview loads. Also leave detail before the download finishes and reopen it. Keep list loading on the containing split view so compact navigation does not cancel it when hiding the sidebar. Do not use an existing signed-in simulator or device for this check.

---
name: verify
description: Verify a Folio change in the running app by launching an ephemeral instance (own port, temp DB and storage, no user config), exercising it through the API, SSE and a browser (including a screenshot and axe tour of every screen), then tearing it down. Use after changing routes, templates, the worker, config or storage, and before claiming a feature works. For Apple share attachments, use the isolated provider regression checks below. Never touch a real or deployed Folio.
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

1. `scripts/build.sh` builds `folio-app` once (uses `~/.local/bin/cargo-slot` when present, `CARGO_INCREMENTAL=0`). Set `CARGO_TARGET_DIR` inside the worktree when sharing a machine with other agents. For long batches, set `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` to limit disposable build artifacts. Reclaim space by cleaning only the task-owned target directory. `up.sh` builds on demand if the binary is missing; set `FOLIO_BIN` to use another binary (for example a release build in CI).
2. `env_file=$(scripts/up.sh)` picks a free port, creates `${TMPDIR:-/tmp}/folio-verify.XXXXXX` (DB, storage, log in `server.log`), starts the app, waits for readiness and prints the env file path. The env file defines `BASE_URL`, `PID`, `VERIFY_DIR`, and the effective `FOLIO_VERIFY_AUTO_OCR` so a separately invoked smoke test uses the instance configuration. `source` it in later steps.
3. `scripts/smoke.sh "$env_file"` runs the API checks: upload the fixture PDF through the UI route (so the worker handles it), poll until the worker has set `page_count` (with provider `local` OCR yields no text, so `content` stays null), list and get via `/api`, retrieve the generated JPEG thumbnail with private no-store headers, download and compare bytes, assert an SSE event arrived on `/events`, delete and expect 404.
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
- Webhook action identity: run `cargo test -p folio-infra --test workflows webhook_actions_have_independent` for a local deduplicating receiver with two actions and a retry. In an isolated app, send two webhook actions to one local receiver with different routing headers; verify distinct `X-Folio-Delivery` IDs, one shared `X-Folio-Run`, and stable IDs when an action retries.
- Webhooks and workflows: start a local receiver (`python3 -m http.server`, or `nc -l`) on a free port, point the webhook at `http://127.0.0.1:<port>`, run the trigger (upload), and check the receiver log.
- Postgres or S3 adapters: start them with testcontainers or a local container, pass the URL through `up.sh` env, and run `smoke.sh` unchanged; the API contract is the same.
- Classification forms: on a throwaway document with existing correspondent and type, submit malformed and missing fields to both `/documents/:id/{correspondent,document_type}` and `/api/documents/:id/{correspondent,document_type}`. Expect 400 with both associations preserved. Explicit empty fields clear only the selected association, and valid IDs assign it. Run `cargo test -p folio-api --test routes classification_forms_reject` plus the vault-isolation tests.
- Modal uploads: run `FOLIO_BIN=<new binary> npm test --prefix e2e -- upload-failures.spec.ts keyboard.spec.ts`. The tests cover accepted uploads, server rejection, transport failure, retained title/file inputs, retry, status announcements, and dialog keyboard navigation. The fixture starts and removes its own instance.
- Upload queue: run `FOLIO_BIN=<new binary> npm test --prefix e2e -- upload-queue.spec.ts`. In a mixed queue, Dismiss finished removes only uploaded and cancelled rows, Cancel pending retains cancelled outcomes while an active request continues, and Retry resends the failed file to its original vault. Inspect the queue on mobile in both themes.
- Recovery-code privacy: run `npm test --prefix e2e -- recovery-cache.spec.ts` with the new `FOLIO_BIN`. The test creates an isolated owner and authenticator, checks that acknowledged recovery codes leave no Turbo snapshot, retains HTTP `no-store`, and exercises Back/Forward navigation and link focus. Both throwaway instances are removed afterward.
- Pipeline startup configuration: run the smoke test with `FOLIO_VERIFY_OCR_PROVIDER=none FOLIO_VERIFY_AUTO_OCR=false FOLIO_VERIFY_AUTO_EMBED=false`. On the fresh database, `/settings` must select `none` and leave both processing checkboxes unchecked; uploads must finish with skipped OCR and embeddings. The scripts pass these test options through the scrubbed environment.
- Search palette: run `FOLIO_BIN=<new binary> npm test --prefix e2e -- search-races.spec.ts keyboard.spec.ts`. The browser tests delay JSON completion across recent/search requests and closing/reopening, then check that only current results render. They distinguish successful empty results from unavailable search, recent fallback matches, and fallback failure. Include the palette in the screenshot and axe tour.
- OCRmyPDF: install the local `ocrmypdf` CLI and language packs, select `ocrmypdf` in processing settings, and upload an image rendered from the test PDF. Check `ocr_status=done` and the expected text; a PDF with an existing text layer bypasses the OCR provider. Run `cargo test -p folio-infra ocrmypdf_reads_a_rendered_pdf_locally` to cover rendered PDF recognition and missing language packs.
- Real OCR or embeddings: the throwaway instance uses local Tesseract and no embeddings by default. Test hosted providers only with a dedicated test key supplied by the user, never from the environment you inherited.

## Document tags

Run `FOLIO_BIN=<new binary> npm test --prefix e2e -- tags.spec.ts` against the
disposable instance. The tests add quoted and attribute-like tag names, check
literal text and accessible remove labels in both displays, and remove the tag
through the dialog. Include the document screen in the screenshot and axe tour.

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

## Interface motion

Run `cd e2e && npx playwright test motion.spec.ts keyboard.spec.ts` against the
disposable instance. The motion suite checks dialog focus restoration, toast
dismissal, drawer backdrops, and one-time streamed row fades with reduced motion
on and off. Replacements and duplicate row insertions must not replay the fade.

The Apple package tests cover upload, trash, undo, and redo animation signals
separately from ordinary reloads and failed or invisible changes. In an isolated simulator, check upload notice appearance
and dismissal, new rows, and confirmed deletion with Reduce Motion on and off.
Keep native navigation and immediate privacy locking unchanged.

## Apple navigation

Run `xcrun swift test --package-path apple/FolioKit` for native changes. For list and detail loading, the focused suites are `DocumentListViewModelTests`, `DocumentListFailureTests`, and `DocumentDetailViewModelTests`. The detail suite covers cancellation followed by a successful retry. Run `xcrun swift-format lint` on changed Swift files.

Check navigation in a throwaway simulator with test credentials and an isolated server. Open a document, return to the list, and open it again. Check that rows remain visible and the preview loads. Also leave detail before the download finishes and reopen it. Keep list loading on the containing split view so compact navigation does not cancel it when hiding the sidebar. Do not use an existing signed-in simulator or device for this check.

## Apple share attachments

For share attachment changes, run the provider regression suite with isolated build and preference storage. This does not launch Folio or access its server configuration. Do not run the Keychain round-trip suite against a user's keychain.

```bash
share_verify_dir=$(mktemp -d /private/tmp/folio-share-verify.XXXXXX)
trap 'rm -rf "$share_verify_dir"' EXIT
mkdir -p "$share_verify_dir/home"
CFFIXED_USER_HOME="$share_verify_dir/home" \
CLANG_MODULE_CACHE_PATH="$share_verify_dir/cache" \
xcrun swift test --disable-sandbox --package-path apple/FolioKit \
  --scratch-path "$share_verify_dir/build" --cache-path "$share_verify_dir/spm-cache" \
  --config-path "$share_verify_dir/spm-config" --security-path "$share_verify_dir/spm-security" \
  --jobs 2 --filter 'SharedAttachmentLoaderTests|ShareViewModelTests|ShareViewModelBackgroundTests|BackgroundUploaderTests|ShareUploadFailureTests|UploadBatchRunnerTests|UploadBatchRunnerHTTPTests|ShortcutUploadRetryTests'
```

The suite covers names with and without extensions, absent suggestions, unsafe basenames, inferred MIME types, duplicate names, mixed and total failures, unsupported types, and retry after a read failure. The background upload suites drive the real `BackgroundUploader` through a host-scoped `URLProtocol` stub and a temporary store directory: 401, 500, 415, transport failure and success, retry with a refreshed token, the body kept until the server accepts it, and earlier failures listed in the next share sheet. The upload batch suites cover tagged share and Shortcut retries: upload succeeds and tagging fails, the second file fails, a lost upload answer found on the server instead of uploaded again, a different batch starting fresh, a retry keeping its original vault, and a Shortcut rerun resuming from the progress file. Background relaunch (`handleEventsForBackgroundURLSession`) needs a throwaway simulator or device signed in to an isolated server: share a file, kill the extension, stop the server, and check the share is listed as failed with Retry the next time the sheet opens. Compile the share extension locally and require the Apple CI lane, which builds both iOS Simulator and macOS, before merging. Use temporary DerivedData and package storage. Do not launch a user profile or access real server credentials.

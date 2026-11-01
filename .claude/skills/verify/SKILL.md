---
name: verify
description: Verify a Folio change in the running app by launching an ephemeral instance (own port, temp DB and storage, no user config), exercising it through the API, SSE and a browser, then tearing it down. Use after changing routes, templates, the worker, config or storage, and before claiming a feature works. Not for unit tests (use `cargo test`) or for touching a real or deployed Folio.
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
2. `env_file=$(scripts/up.sh)` picks a free port, creates `${TMPDIR:-/tmp}/folio-verify.XXXXXX` (DB, storage, log in `server.log`), starts the app, waits for readiness and prints the env file path. The env file defines `BASE_URL`, `PID`, `VERIFY_DIR`. `source` it in later steps.
3. `scripts/smoke.sh "$env_file"` runs the API checks: upload the fixture PDF through the UI route (so the worker handles it), poll until the worker has set `page_count` (with provider `local` OCR yields no text, so `content` stays null), list and get via `/api`, download and compare bytes, assert an SSE event arrived on `/events`, delete and expect 404.
4. Drive the UI (next section).
5. `scripts/down.sh "$env_file"` kills the PID and removes the temp dir. Idempotent. When using `up.sh` directly, install `trap '$S/down.sh "$env_file"' EXIT` first.

Why the instance is isolated: `Config::load()` reads `./config.toml` from the working directory and `dotenvy` walks up from it looking for `.env`. `up.sh` therefore runs the app from the temp dir with a scrubbed environment (`env -i`) and symlinks `static/` in, since the app serves `./static` relative to the working directory.

Config names today are `FOLIO_SERVER_HOST`, `FOLIO_SERVER_PORT`, `FOLIO_DATABASE_URL`, `FOLIO_STORAGE_PATH`, `FOLIO_AI_PROVIDER=local` (no AI calls). Extension point: when the env-first config change lands these move to `__` nesting (`FOLIO_SERVER__PORT`, ...); update the env block in `up.sh` only.

## 3. Drive the UI

- With Playwright MCP browser tools available: navigate to `BASE_URL`, snapshot, click through the flow, and save screenshots under `$VERIFY_DIR/shots/`. Prefer accessible roles and labels (`Upload document` button, `Title (optional)` and `File` fields, `Search documents...` trigger).
- Otherwise run the committed suite, which starts its own instance through the same scripts: `make e2e`, or `cd e2e && npx playwright test` (set `E2E_BASE_URL` to reuse an instance you already started). Failure screenshots and traces land in `e2e/test-results/`, the HTML report in `e2e/playwright-report/`.
- Screenshots of a passing flow: `cd e2e && npx playwright screenshot --browser chromium "$BASE_URL" "$VERIFY_DIR/inbox.png"`.

Known quirk: after the worker finishes, an SSE event swaps the list for a variant whose links target the `document-detail` frame, so clicking a document right then shows "Content missing". Wait for processing, then reload, before clicking through.

The Turbo/Stimulus UI updates the DOM after fetches; wait on visible text (`expect(...).toBeVisible()`), not on `networkidle` (debug builds hold a livereload connection open).

## 4. Feature areas as they land

Short notes for features that do not exist on main yet. Check the code first; skip a note whose feature is absent.

- Health endpoints: `up.sh` already prefers `/readyz` once it returns something other than 404. Add `/healthz` and `/readyz` assertions to `smoke.sh`.
- OpenAPI: fetch the served spec and check that `/api/documents` is listed; compare against the shape asserted in `e2e/tests/documents.spec.ts`.
- Auth: open `$BASE_URL/setup` to create the owner with a throwaway password, then log in. Extend `smoke.sh` with the session cookie or token and `e2e/` with a login fixture. Passkeys, OIDC and forward-auth need a local fake provider or headers set by curl; never a real IdP.
- MCP at `/mcp`: `claude mcp add --transport http folio-verify "$BASE_URL/mcp"` (remove it afterwards) or a JSON-RPC probe: `curl -sS -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' "$BASE_URL/mcp"`.
- Webhooks and workflows: start a local receiver (`python3 -m http.server`, or `nc -l`) on a free port, point the webhook at `http://127.0.0.1:<port>`, run the trigger (upload), and check the receiver log.
- Postgres or S3 adapters: start them with testcontainers or a local container, pass the URL through `up.sh` env, and run `smoke.sh` unchanged; the API contract is the same.
- Real OCR or embeddings: `FOLIO_AI_PROVIDER=local` skips them. Test those providers only with a dedicated test key supplied by the user, never from the environment you inherited.

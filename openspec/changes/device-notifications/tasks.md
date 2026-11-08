# Tasks

Phase 2 lands as stacked PRs:

- **Server** (groups 1–7) on `claude/feat-359-device-notifications`, rebased onto the #306 fix before merge
- **Apple** (groups 8–10) on a branch stacked on it

Each task is one semantic commit, and each test below is written first and seen to fail.

## 1. Domain model and storage

- [ ] 1.1 Add the folio-core models:
  - `PushDevice`, with per-token registration times
  - `NotificationPreferences`, defaulting to processed off, needs_review and failed on, scope `mine` and vaults `all`
  - `NotificationKind`, `PendingNotification` and `AlertSummary`

  Verify with unit tests for the defaults and a JSON round-trip.

- [ ] 1.2 Add the `PushDeviceRepository` port:
  - devices: `upsert`, `list_for_user`, `delete`, `clear_token`
  - first outcomes: `record_first_outcome`, which reports whether the row was new
  - pending notifications: `insert_pending` and `prune`
  - the sender's lease: `claim_due_devices`, `take_pending` and `release`
  - device state

  Add migration `20261108160000_push_devices.sql` to `migrations/sqlite/` and `migrations/postgres/`. Verify with repository tests on SQLite and Postgres:
  - an upsert updates only the sent token's registration time
  - a token moving to another user deletes the old row
  - `record_first_outcome` is true once
  - `(device, document, kind)` dedupes
  - two concurrent claims never return the same device
  - deleting a user or a document cascades

## 2. Authorization ownership

- [ ] 2.1 Expose `grant_id` on `AuthContext`. Verify with a validator test: an OAuth bearer has its grant, and a personal token has none.
- [ ] 2.2 Delete devices inside the auth-store revocation transactions: by `grant_id` in `revoke_grant` and `revoke_token`, which covers refresh reuse, and by `(user_id, client_id)` in `revoke_client_tokens`. Verify with SQLite and Postgres tests for `/oauth/revoke`, Security-page revocation and refresh-token reuse.
- [ ] 2.3 Add `grant_is_live(grant_id)`, which ignores used and expired tokens. Verify with tests: a grant with only `used = 1` refresh tokens is not live.

## 3. Device API (JSON)

- [ ] 3.1 Add `PUT /api/devices/{installation_id}`. It responds `201` or `200` with `push_available`, `403` for a personal token or cookie, and `422` for a bad platform, environment, token, locale or vault. Verify with an API test for each status.
- [ ] 3.2 Add `GET /api/devices`, which returns no tokens and reports `push_available`, and an idempotent `DELETE /api/devices/{installation_id}`. Verify with API tests, including that deleting another person's installation ID leaves it unchanged.
- [ ] 3.3 Add `POST /api/devices/{installation_id}/test`, responding `202`, `409` or `429`. Verify with API tests against a recording transport.
- [ ] 3.4 Regenerate `openapi/openapi.json`. Verify that the OpenAPI drift test passes.

## 4. Planning from the outbox

- [ ] 4.1 Implement `NotificationPlanner: EventConsumer`. Verify with unit tests over in-memory fakes.
  - It handles `Processed` with origin User or Workflow, and `…failed` envelopes.
  - On a first outcome it records the outcome and inserts pending rows in one transaction.
  - It offers a document only to devices registered at or before the document's creation.
  - Its audience is the uploader under `mine`, and `can(EditDocument { uploaded_by })` under `editable`, in the vault from `origin_vault()`.

  The tests cover:
  - first upload
  - reprocess and re-embed of a notified document
  - re-embed of a document older than the device
  - imports
  - a retry that is still scheduled
  - a viewer under `editable`
  - group edit access
  - the vault filter
  - duplicate delivery

- [ ] 4.2 Wire the planner into the dispatcher in `folio-app/src/main.rs`. Verify with an integration test: upload, process with the local pipeline and no AI provider, and find one pending `needs_review` row. Reprocessing the document adds none.

## 5. Sending

- [ ] 5.1 Add the `PushTransport` port, the `none` adapter, and the alert templates. The templates produce English and German count text, `thread-id` only for a single vault, `vault` and `document` only when single, the category, and `mutable-content` only with `show_titles`. Verify with unit tests, including that a serialized payload never contains the document title or vault name.
- [ ] 5.2 Implement `NotificationSender`. Verify with tests on a controllable clock and a recording transport:
  - 200 outcomes in 3 minutes give at most 2 alerts
  - two vaults give one alert without a `thread-id`
  - the cap carries outcomes over
  - a reviewed, trashed or no-longer-editable document is dropped
  - a revoked grant sends nothing and deletes the device
  - two sender instances send exactly once

  The behavior under test:
  - claimed device leases
  - the quiet window and the 5-minute ceiling
  - one alert across vaults
  - the hourly cap
  - checks for grant liveness, edit access, trash and review
  - priority 10 for a single document or a failure
  - the batch collapse ID
  - the 24 h prune

- [ ] 5.3 Handle APNs responses per design Decision 7, including the per-token 410 comparison. Verify with a table-driven test, including a dead alert token whose device refreshed its widget token later.
- [ ] 5.4 Send widget reload pushes, at most one per device every 15 minutes and outside the alert cap. Verify with a sender test: three Inbox changes in a minute give one `widgets` push.

## 6. APNs transport

- [ ] 6.1 Add the `ApnsTransport` adapter:
  - an ES256 provider token, re-signed after 50 minutes or when the key changes
  - HTTP/2 to the sandbox and production hosts, or to `push.apns.base_url` (h2c)
  - the headers from Decision 6

  Verify with unit tests on the JWT, and with an in-process h2c stub test that checks headers and the mapping of 200, 410, 400 and 403.

## 7. Settings and end-to-end

- [ ] 7.1 Add `PushConfig` and the `push.*` `SETTINGS` entries: a `_FILE` key, `Redacted` Debug, and `base_url` from configuration only. Document them in `docs/configuration.md`. Verify with config tests and the docs-sync test.
- [ ] 7.2 Add encrypted settings values (`enc:v1:` with `SecretBox`), refused when `auth.secret_key` comes from the database. Add the effective push config with a 5 s cache. Verify with unit tests for the round-trip, the refusal and precedence.
- [ ] 7.3 Add the "Push notifications" card, `POST /settings/push` (PKCS#8 P-256 validation, admin only) and `POST /settings/push/test`. The card shows the stub URL warning. Verify with route tests for save, a bad key, the refusal and non-admin `403`.
- [ ] 7.4 Add an `e2e/` Playwright spec: start an h2c APNs stub, run Folio with `FOLIO_PUSH__TRANSPORT=apns` and `FOLIO_PUSH__APNS__BASE_URL` pointing at it, sign in through OAuth with PKCE, register a device, upload a document and assert that the stub receives one alert without the title. Verify with `make e2e`.
- [ ] 7.5 Run `make format`, `make lint`, `make test` on SQLite and Postgres, and `make e2e`. Run the `verify` skill on an ephemeral instance against the stub, and extend `.claude/skills/verify` with the push checks.

## 8. Apple: registration and permission

- [ ] 8.1 Copy the regenerated OpenAPI into `apple/FolioKit/Sources/FolioAPI/openapi.json`, and add `registerDevice`, `listDevices`, `deleteDevice` and `sendTestNotification` to `FolioService`. Verify with FolioKit tests against `MockFolioService`.
- [ ] 8.2 Add `PushRegistration`:
  - environment from the build configuration, plus locale and preferences
  - re-registers when the token, preferences or server change
  - all token refreshes go through the existing provider under `oauthRefreshLock`
  - `DELETE` before revoke in `AppModel.signOut()` and the reconnect path, then delivered notifications are cleared

  Add `aps-environment` to `project.yml`. Verify with unit tests on fakes, including the order of `DELETE` and revoke on sign-out.

- [ ] 8.3 Add the permission flow (`.provisional` on first sign-in, the full prompt after the first finished upload or scan) and Settings → Notifications (three toggles, scope, the title opt-in with the App lock note, a test button, and an explanation of `push_available: false`). Verify with view-model tests.

## 9. Apple: handling and actions

- [ ] 9.1 Add `NotificationRouter`: categories `FOLIO_DOCUMENT` (Open, and Mark reviewed with `.authenticationRequired`) and `FOLIO_INBOX`, with `hiddenPreviewsBodyPlaceholder`. It routes through `IntentRouter`, falling back to the Inbox, and `willPresent` suppresses the banner on the matching Inbox. Verify with unit tests that parse sample payloads.
- [ ] 9.2 Make Mark reviewed call the review endpoint and remove the delivered notification. Verify with a unit test on the mock service, and confirm that Apple CI (the Swift gate) passes.

## 10. Apple: title opt-in extension

- [ ] 10.1 Add the `FolioNotificationService` target, sharing the keychain group and App Group. For a `mutable-content` single-document alert, it fetches the title through `FolioService` within 5 s, with any refresh under `SharedConfiguration.oauthRefreshLock`, and keeps the generic text when signed out, refused or out of time. Verify with unit tests of its content builder against a mock service that succeeds, returns `403`, times out, and needs a refresh, the last one asserting the lock is taken.

## Workflow follow-up

- Archive the change as the last commit of the final PR.
- Follow-ups: the relay adapter once the owner decides, the Inbox badge (absolute count, reset on open), and background refresh for widgets and the badge.

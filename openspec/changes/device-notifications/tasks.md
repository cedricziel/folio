# Tasks

Phase 2 lands as stacked PRs:

- **Server** (groups 1–7) on `claude/feat-359-device-notifications`
- **Apple** (groups 8–10) on a branch stacked on it

Each task is one semantic commit, and each test below is written first and seen to fail.

## 1. Domain model and storage

- [ ] 1.1 Add `PushDevice`, `NotificationPreferences` (defaults: processed off, needs_review on, failed on, show_titles off, vaults all), `NotificationKind` and `PendingNotification` to folio-core models. Verify with a unit test for the preference defaults and JSON round-trip.
- [ ] 1.2 Add the `PushDeviceRepository` port: `upsert`, `list_for_user`, `delete`, `delete_for_grants`, `clear_token`, `insert_pending` (idempotent), `due_devices`, `take_pending`, `state`/`update_state`, `prune_pending`. Add migration `20261108160000_push_devices.sql` in `migrations/sqlite/` and `migrations/postgres/`. Verify with repository tests on SQLite and Postgres: upsert replaces tokens, a token moving to another user deletes the old row, a duplicate `insert_pending` is a no-op, and deleting a user cascades.

## 2. Grant ownership and revocation

- [ ] 2.1 Expose `grant_id` on `folio_auth::AuthContext` from the access-token row. Verify with a validator test that an OAuth bearer carries its grant, and a personal token none.
- [ ] 2.2 Add `GrantRevocationListener` (folio-auth trait) and call it from `revoke_grant`, `revoke_token`, `revoke_client_tokens` and the refresh-reuse path. Implement it in folio-core with `delete_for_grants`. Verify with a test that revoking through `/oauth/revoke` and the Security page deletes the grant's devices.
- [ ] 2.3 Add `grant_is_live(grant_id)` to the auth store. Verify with a test that it is false after revocation and after every token in the grant has expired.

## 3. Device API (JSON)

- [ ] 3.1 Add `PUT /api/devices/{installation_id}` with utoipa docs. It returns `201` or `200`, `403` for personal tokens and cookies, and `422` for a bad platform, environment, token or vault, and the response includes `push_available`. Verify with API tests for each status code.
- [ ] 3.2 Add `GET /api/devices` (no tokens in the body) and an idempotent `DELETE /api/devices/{installation_id}` (`204`, never touching another person's device). Verify with API tests.
- [ ] 3.3 Add `POST /api/devices/{installation_id}/test` (`202`, `409` without push, `429` after one test a minute). Verify with API tests using a recording fake transport.
- [ ] 3.4 Regenerate `openapi/openapi.json`. Verify that the OpenAPI drift test passes.

## 4. Planning notifications from the outbox

- [ ] 4.1 Implement `NotificationPlanner: EventConsumer`:
  - `Processed` with `reviewed_at` gives `processed`; without it, `needs_review`
  - `document.processing.*.failed` gives `failed`
  - `retry_scheduled` is ignored

  Insert pending rows for devices with the kind enabled, a role in the vault and a matching vault filter. Verify with unit tests over an in-memory repository and fake `AccessPolicy`, covering each kind, a retry, a revoked vault, a group member, the vault filter and duplicate delivery.

- [ ] 4.2 Wire the planner into the dispatcher consumers in `folio-app/src/main.rs`. Verify with an integration test that uploads, processes with the local pipeline (no AI provider), and finds one pending `needs_review` row per device.

## 5. Sending, coalescing and limits

- [ ] 5.1 Add the `PushTransport` port and the `none` adapter, plus the pure payload builder (generic localized text, `thread-id`, collapse ID within 64 bytes, category, `mutable-content` only with `show_titles`, document ID only for a single document, never a title). Verify with unit tests, including one that the serialized payload never contains the document title.
- [ ] 5.2 Implement `NotificationSender`:
  - quiet window and 5-minute ceiling
  - hourly cap with carry-over
  - grant liveness and vault role re-check
  - dropping reviewed `needs_review` rows
  - a 24 h prune

  Verify with tests on a controllable clock and a recording transport: 200 outcomes in 3 minutes give at most 2 alerts, the cap holds and carries over, reviewing before the send drops the row, and a revoked grant sends nothing and deletes the device.

- [ ] 5.3 Add response handling per design Decision 7: token clearing on 410 using the timestamp, 400 token errors, `ExpiredProviderToken` re-sign, unhealthy transport on other 403s, 429 and 5xx backoff. Verify with a table-driven test over the transport outcomes.
- [ ] 5.4 Add widget reload pushes (one per device every 15 minutes at most, `content-changed` only, outside the alert cap). Verify with a sender test where three Inbox changes in a minute give one widgets push.

## 6. Transports

- [ ] 6.1 Add the `ApnsTransport` adapter:
  - ES256 provider token via `p256`, cached and re-signed after 50 minutes, and immediately on key change
  - HTTP/2 client, sandbox and production hosts
  - all headers per Decision 6

  Verify with unit tests on the JWT claims and signature, and with an h2c stub server test that checks headers and the mapping of 200, 410, 400 and 403 responses.

- [ ] 6.2 Add the `RelayTransport` adapter for the relay contract in design Decision 1. Verify with a stub HTTP server test of request shape, bearer token and status mapping.

## 7. Settings

- [ ] 7.1 Add `PushConfig` and the `push.*` `SETTINGS` entries with `_FILE` secrets and `Redacted` Debug, and document them in `docs/configuration.md`. Verify with config parsing tests and the existing docs-sync test.
- [ ] 7.2 Add encrypted settings values (`enc:v1:` via `SecretBox`), and an effective push config with a 5 s cache where the database wins and empty database secrets fall back to the config. Verify with unit tests for encrypted and plain round-trips and for precedence.
- [ ] 7.3 Add the "Push notifications" card on `/settings`, `POST /settings/push` (PKCS#8 P-256 validation, admin only) and `POST /settings/push/test`. The card shows key presence, sources, the last credential error and the device count. Verify with route tests for save, a bad key, non-admin `403` and the Turbo Stream response, and with a new `e2e/` Playwright spec for the card.
- [ ] 7.4 Run `make format`, `make lint`, `make test` (SQLite and Postgres) and `make e2e`. Run the `verify` skill on an ephemeral instance, with registration and test send against the h2c stub as the APNs base URL. Extend `.claude/skills/verify` with the push checks.

## 8. Apple: registration and permission

- [ ] 8.1 Copy the regenerated OpenAPI into `apple/FolioKit/Sources/FolioAPI/openapi.json`, and add `registerDevice`, `listDevices`, `deleteDevice` and `sendTestNotification` to `FolioService`. Verify with FolioKit tests against `MockFolioService` and a URLProtocol stub.
- [ ] 8.2 Add `PushRegistration`: build-config environment, re-registering on token, preference or server change, and deleting before revoke in `AppModel.signOut()` and the reconnect path. Add `aps-environment` to `project.yml`. Verify with unit tests on a fake registrar and service, including that sign-out sends `DELETE` before revoke and clears delivered notifications.
- [ ] 8.3 Add the permission flow: `.provisional` on first sign-in, and the full prompt with a pre-prompt after the first finished upload or scan. Add Settings → Notifications with the three toggles, the title opt-in (with the App lock note), a test button, and the `push_available: false` explanation. Verify with view-model unit tests.

## 9. Apple: handling and actions

- [ ] 9.1 Add `NotificationRouter`: categories `FOLIO_DOCUMENT` (Open, Mark reviewed with `.authenticationRequired`) and `FOLIO_INBOX`, with `hiddenPreviewsBodyPlaceholder`. Taps route through `IntentRouter` to the document or Inbox, falling back to the Inbox on no access. `willPresent` suppresses the banner on the matching Inbox. Verify with unit tests that parse sample payloads into routes and actions.
- [ ] 9.2 Make Mark reviewed call the review endpoint and remove the delivered notification. Verify with a unit test on the mock service.

## 10. Apple: title opt-in extension

- [ ] 10.1 Add the `FolioNotificationService` target (shared keychain group and App Group). It fetches the document title for `mutable-content` alerts within 5 s and falls back to the generic text. Verify with a unit test of its content builder over a mock service that succeeds, fails and times out.
- [ ] 10.2 Confirm that Apple CI (the Swift gate) is green and that `make format`, `make lint` and `make test` still pass for the server.

## Workflow follow-up

- Archive the change as the last commit of the final PR (`openspec archive device-notifications`).
- Follow-up issues: background-refresh fallback for `push_available: false`, Inbox badge, and the project-run relay service.

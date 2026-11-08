# Device notifications for processing and review

## Why

Uploads and scans hand off to server processing, but the Apple app only learns the result when someone opens it again. The server records every processing outcome in the outbox and streams it over SSE to open web pages, but nothing reaches a phone or Mac (#359, part of #351).

A self-hosted server can't push to the App Store build on its own: only the Apple Developer team that owns `com.cedricziel.folio` can sign APNs requests for it. The design has to keep installs without push credentials working, let an operator with credentials turn push on, and leave a safe seam for a project-run relay.

## What Changes

- **Device registration API (JSON).**
  - `PUT /api/devices/{installation_id}` registers an APNs alert token and, optionally, a widget token, with locale and preferences.
  - `GET /api/devices` lists the caller's devices and reports `push_available`.
  - `DELETE /api/devices/{installation_id}` removes a device.
  - `POST /api/devices/{installation_id}/test` sends a test alert.
  - Only OAuth clients may register.
- **Registrations end with the app's authorization.**
  - Sign-out, revoking the app on the web, refresh-token reuse and user deletion delete the devices in the same transaction.
  - The sender also re-checks that the grant still has a live token.
  - Fixing #306 (tokens issued at the same moment as a revocation) is a prerequisite.
- **First completions only.**
  - An outbox consumer records each document's first completion and first terminal failure, then offers `needs_review`, `processed` or `failed` to eligible devices.
  - Reprocessing, re-embedding, repair, imports, scheduled retries, and documents older than the device's registration never notify.
- **Right audience.**
  - By default only the uploader is notified.
  - People can widen this to documents they may edit, but never to documents they can only view.
  - Trashed documents, reviewed documents and lost access drop the pending notification.
- **One alert per device per flush.**
  - Distinct documents are counted across vaults, after a 30 s quiet window and never more than 5 minutes after the first outcome.
  - Each device gets at most 12 alerts an hour.
  - Each device's batch is claimed with a lease, so replicas never double-send.
- **Generic payloads.** Alert text comes from fixed templates filled with counts. Payloads never carry titles, names or content, and alerts carry a lock-screen placeholder that names no document.
- **Widget reloads.** The same sender sends `apns-push-type: widgets` pushes to widget tokens, at most once per device every 15 minutes.
- **Transport.**
  - A `PushTransport` port with `none` (default) and `apns` adapters. `apns` uses direct HTTP/2 with ES256 token auth.
  - Dead tokens are cleared on `410` against each token's own registration time, and on `BadDeviceToken` and `DeviceTokenNotForTopic`.
  - A Bitwarden-style relay, where the device registers with the relay and the server holds only a handle, is designed but not built until the owner decides whether to run one.
- **Settings.**
  - A `[push]` section in `config.toml` and a "Push notifications" card on `/settings`. Database values win, and `config.toml` is the fallback.
  - The `.p8` key may be stored in the database only when `auth.secret_key` comes from the environment or a file.
  - `push.apns.base_url`, read from configuration only, points tests at a local HTTP/2 stub.
- **Apple app.**
  - Asks for notification permission after the first finished upload or scan.
  - Registers its token, and deletes it before revoking on sign-out.
  - Opens the document or the Inbox through `IntentRouter`.
  - Offers _Open_ and _Mark reviewed_ actions.
  - Adds a Settings → Notifications screen.

## Capabilities

### New Capabilities

- `device-notifications`: covers
  - device registration
  - preferences
  - revocation
  - audience and vault scoping
  - first-completion triggers
  - coalescing and rate limits
  - payload privacy
  - deep links and actions
  - invalid tokens
  - widget reload pushes
  - test notifications

### Modified Capabilities

- `settings`: push configuration, key storage rules, the test endpoint override and the settings card.

## Impact

- **Hexagonal layers.**
  - **folio-core:**
    - Models: `PushDevice`, `NotificationPreferences`, `NotificationKind`, `PendingNotification` and `AlertSummary`.
    - Ports: `PushDeviceRepository` and `PushTransport`.
    - Services: the `NotificationPlanner` outbox consumer, the `NotificationSender` and the alert templates.
  - **folio-infra:**
    - SQLite and Postgres repositories.
    - Device deletion inside the auth-store revocation transactions.
    - The `ApnsTransport` adapter (reqwest over HTTP/2, ES256 via `p256`).
    - Push configuration and encrypted settings values.
  - **folio-auth:** `AuthContext.grant_id`, and `grant_is_live` ignoring used refresh tokens.
  - **folio-api:** device routes, the settings card and forms, and OpenAPI.
  - **folio-app:** wires the planner into the outbox dispatcher and spawns the sender.
- **HTTP.**
  - New JSON API routes: `GET /api/devices`, `PUT` and `DELETE /api/devices/{installation_id}`, and `POST /api/devices/{installation_id}/test`.
  - New HTML (Turbo) routes: `POST /settings/push` and `POST /settings/push/test`.
  - `openapi/openapi.json` and its Apple copy are regenerated.
- **Settings.** New `push.*` keys in `config.toml` (with `FOLIO_PUSH__*` and `_FILE` for the key) and in the settings table. Database rows win, except `push.apns.base_url`, which is configuration only.
- **Database.** New migration `20261108160000_push_devices.sql` in both `migrations/sqlite/` and `migrations/postgres/`. It creates `push_devices`, `push_document_outcomes`, `push_pending` and `push_device_state`, and changes no existing rows.
- **Dependencies.** `p256` becomes a direct folio-infra dependency; it is already in the lockfile. No new service.
- **Apple.**
  - The `aps-environment` entitlement.
  - FolioKit `PushRegistration` and `NotificationRouter`.
  - Sign-out and reconnect changes in `AppModel`.
  - No new extension target.
  - Nothing in `Sharing/`, `FileProvider/`, `Telemetry/` or App Intents.
- **Ordering.** The #306 fix lands before the server PR merges.

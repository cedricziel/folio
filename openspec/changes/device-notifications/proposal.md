# Device notifications for processing and review

## Why

Uploads and scans hand off to server processing, but the Apple app only learns the result when someone opens it again. The server already records every processing outcome in the outbox and streams it over SSE to open web pages. Nothing reaches a phone or Mac in someone's pocket or menu bar (#359, part of #351).

A self-hosted server also can't push to the App Store build on its own. Only the Apple Developer team that owns the bundle ID `com.cedricziel.folio` can sign APNs requests for it. So the design has to keep self-hosted installs working without push, let an operator with credentials enable it, and leave a clean seam for a project-run relay.

## What Changes

- **Device registration API (JSON).** `PUT /api/devices/{installation_id}` registers or updates an APNs token for the signed-in OAuth grant (alert and, separately, widget tokens), with the person's notification preferences. `GET /api/devices` lists that person's devices, and `DELETE /api/devices/{installation_id}` removes one. Only OAuth clients may register; personal access tokens get `403`.
- **Revocation follows the grant.** A device registration belongs to the OAuth grant that created it. Sign-out (`/oauth/revoke`), revoking an app on the web Security page, and token-family revocation delete its devices. Delivery also re-checks that the grant is still live and that the person still has a role in the vault, so a missed cleanup never results in a push.
- **Notification triggers.** A new outbox consumer turns three outcomes into pending notifications for every eligible device:
  - `processed` (`DocumentEvent::Processed`)
  - `needs_review` (`Processed` while the document is unreviewed)
  - `failed` (terminal `document.processing.*.failed` envelopes)

  It does no network I/O, so a slow APNs call can't stall SSE or workflows.

- **Coalesced delivery.** A sender drains pending notifications per device after a short quiet window. It sends one alert per window: "1 document needs review", or "3 documents need review" with a link to the Inbox. Each alert carries a collapse ID and a per-vault thread ID, and the sender enforces an hourly cap per device. A bulk import yields a handful of alerts, not one per page or event.
- **Non-revealing by default.** Payloads carry counts, a vault ID and at most one document ID, never titles or content. Showing document titles is a per-device opt-in. The title is then fetched on the device by a Notification Service Extension, so it still never passes through APNs or a relay.
- **Widget reloads.** The same sender sends `apns-push-type: widgets` with `content-changed` to registered widget tokens when the Inbox changes, coalesced more coarsely. This prepares for the widgets sub-issue of #351.
- **Pluggable transport.** A `PushTransport` port has three adapters:
  - `none`, the default: registration works, nothing is sent, and the app is told push is unavailable.
  - `apns`, direct HTTP/2 with ES256 token auth.
  - `relay`, an HTTPS hand-off to a relay that holds the key.

  APNs error responses remove dead tokens: `410 Unregistered`/`ExpiredToken`, `400 BadDeviceToken` and `DeviceTokenNotForTopic`.

- **Settings.** A `[push]` section in `config.toml` and a "Push notifications" card on `/settings` set the transport, APNs key ID, team ID, topic and `.p8` key (or relay URL and token), the quiet window and the hourly cap. The card also has a "Send test notification" button. As elsewhere, database values win and `config.toml` is the fallback. The `.p8` key and relay token are encrypted at rest.
- **Apple app.**
  - Asks for notification permission after the first successful upload or scan, not at launch.
  - Registers its token once signed in, and deletes it (best effort) before revoking on sign-out or server change.
  - Opens a notification on the document or the Inbox through the existing `IntentRouter`.
  - Offers _Open_ and _Mark reviewed_ actions.
  - Adds a Notification Service Extension for the title opt-in.
  - Shows a notification settings screen with the three outcome toggles and the title opt-in.

## Capabilities

### New Capabilities

- `device-notifications`: registering Apple devices for a person and grant, which outcomes trigger a notification, vault scoping, coalescing and rate limits, what payloads may reveal, deep links and actions, token invalidation and widget reload pushes.

### Modified Capabilities

- `settings`: adds the push notification settings card and the effective push configuration from `config.toml` and the database.

## Impact

- **Hexagonal layers.**
  - **folio-core:**
    - Models: `PushDevice`, `NotificationPreferences`, `NotificationKind` and `PendingNotification`.
    - Ports: `PushDeviceRepository`, `PushTransport`, and a `GrantLiveness` check on the auth side.
    - Services: the `NotificationPlanner` outbox consumer (pure, no I/O beyond the repository), the `NotificationSender` loop and the coalescing policy.
  - **folio-infra:** SQLite and Postgres repositories, the `ApnsTransport` adapter (reqwest over HTTP/2, ES256 JWT cached for 50 minutes), the `RelayTransport` adapter and config parsing.
  - **folio-auth:** `AuthContext` gains `grant_id`, and grant revocation deletes the grant's devices.
  - **folio-api:** the JSON routes, the settings card and form, and OpenAPI annotations.
  - **folio-app:** wires the consumer into the outbox dispatcher and spawns the sender.
- **HTTP.** New JSON API routes only: `GET /api/devices`, `PUT` and `DELETE /api/devices/{installation_id}`, and `POST /api/devices/{installation_id}/test`. One new HTML (Turbo) form route, `POST /settings/push`, plus `POST /settings/push/test`. `openapi/openapi.json` and the Swift client copy in `apple/FolioKit/Sources/FolioAPI/openapi.json` are regenerated.
- **Settings.** New `push.*` keys in both `config.toml` (with `FOLIO_PUSH__*` env vars and `_FILE` variants for secrets) and the settings table. The database wins, and `config.toml` is the fallback. Per-device preferences live on the device row, not in settings.
- **Database.** New migration `20261108160000_push_devices.sql` in both `migrations/sqlite/` and `migrations/postgres/`. It creates `push_devices`, `push_pending` and `push_device_state`. No existing rows change.
- **Dependencies.** `p256` (already in the lockfile transitively) becomes a direct dependency of folio-infra for ES256 signing. reqwest already negotiates HTTP/2 over rustls ALPN. No new service is required for the `none` and `apns` transports.
- **Apple.**
  - New entitlement `aps-environment` on the app, and a new `FolioNotificationService` extension target that shares the keychain group.
  - FolioKit gains `PushRegistration` and `NotificationRouter`.
  - The sign-out and reconnect paths in `AppModel` call device deletion.
  - This work stays out of `Sharing/`, `FileProvider/`, `Telemetry/` and App Intents. It reuses `IntentRouter` as a caller only.

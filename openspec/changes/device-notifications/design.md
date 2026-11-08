# Design

## Context

What the server has today:

- **Outbox.** Every document outcome goes to `outbox_events` in the producing transaction (`OutboxStore`, `crates/folio-core/src/ports/outbox.rs`).
  - `OutboxDispatcher` hands each row to its `EventConsumer`s at least once, with concurrency 8 (`crates/folio-core/src/services/outbox_dispatcher.rs`).
  - If one consumer fails, every consumer sees the event again.
  - The consumers are the SSE `BroadcastConsumer` and the `WorkflowEngine`, wired in `folio-app/src/main.rs`.
- **Outcomes.**
  - `DocumentEvent::Processed` is emitted by `finish_embed_stage` (`crates/folio-infra/src/queue/worker.rs`), and in the OCR-completion transaction when embedding is skipped. That covers first processing, reprocess, re-embed and repair alike. It carries an `EventOrigin` (`User | Workflow | Import`) and an `EventSnapshot` with `vault_id` and `reviewed_at`, but no run ID.
  - A terminal failure writes an envelope-only `document.processing.{stage}.failed` row with `run_id` and `reason_code` (`processing_events.rs`). A retry writes `…retry_scheduled` instead.
  - "Needs review" is derived: processed while `reviewed_at IS NULL`.
- **SSE.** `/events` streams to open web pages only. It filters by `origin_vault()` (`crates/folio-api/src/routes/events.rs:163`). The Apple app doesn't use SSE.
- **Auth.**
  - Apple clients use OAuth 2.1 with PKCE and one `client_id` per server.
  - A sign-in's code, access tokens and refresh tokens share a `grant_id` that stays the same across refreshes. A rotated refresh token is kept with `used = 1`.
  - `/oauth/revoke` and refresh reuse (`crates/folio-auth/src/server.rs:446`) revoke the grant. The Security page calls `revoke_client_tokens(user, client)`, which has no grant IDs.
  - #306: tokens issued at the same moment as a revocation can survive it.
- **Vault access.** `AccessPolicy::effective_role` and `can(user, vault, VaultAction)`. `VaultAction::EditDocument { uploaded_by }` distinguishes contributors from editors. Documents carry `uploaded_by`.
- **Settings.**
  - `config.toml` keys are declared in `SETTINGS` (`crates/folio-infra/src/config.rs`) and mirrored in the `settings` table, which admins edit on `/settings`.
  - `SecretBox` encrypts TOTP secrets.
  - If `auth.secret_key` is unset, its fallback key is generated into the same `settings` table (`folio-app/src/main.rs:319`), so a copy of the database can decrypt them.
- **Apple app.**
  - No push setup yet.
  - `IntentRouter` routes `.inbox` and `.document(id, vaultID:)`.
  - Token refresh across processes must hold `SharedConfiguration.oauthRefreshLock`.
  - The App Store bundle ID `com.cedricziel.folio` belongs to the maintainer's developer team.

**The constraint that shapes transport:** APNs accepts a provider key only for apps of the team that created it ([token auth][apns-token], [responses][apns-resp], [transfer][apns-transfer]). A self-hoster can't push to the App Store build with a key of their own.

## Goals / Non-Goals

**Goals:**

- Push is off by default and settings switch it on. Nothing breaks without credentials.
- Nobody is notified about a document they can't act on, or after their app's authorization ends.
- A bulk import, reprocess or re-embed never floods a device.
- Payload text and IDs are safe to pass through Apple and, later, a relay.

**Non-Goals:**

- Implementing the relay. It stays a designed port until the owner decides whether to run one (Decision 1).
- Sending titles through APNs or a relay. The title opt-in is resolved on the device (Decision 4).
- In-app SSE on Apple.
- Live Activities, controls and the widgets themselves (other #351 sub-issues).
- Email or web push.
- The Inbox badge (deferred; see Decision 5).

## Decisions

### Decision 1: Transport — `none` and direct `apns` now, the relay designed but not built

| Option                                                                                                 | Self-hoster configures         | App Store app?                 | Verdict                                                                                                                                                 |
| ------------------------------------------------------------------------------------------------------ | ------------------------------ | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A. Direct APNs.** HTTP/2 to `api.push.apple.com`, ES256 JWT from a `.p8` key ([sending][apns-send]). | Team ID, key ID, topic and key | Only with the maintainer's key | **Build now.** It serves the maintainer's server, CI, and self-builders with their own bundle ID and team.                                              |
| **B. Relay.** A project-run service holds the key ([Bitwarden][bw-relay], [ntfy][ntfy]).               | Enrollment credentials         | Yes                            | **Design only**, as below. It needs an owner decision on hosting, enrollment and abuse.                                                                 |
| **C. Background refresh with local notifications.** `BGAppRefreshTask` polls the server.               | Nothing                        | Yes                            | **Not an alert path.** iOS decides whether and when it runs ([WWDC19][bg-wwdc], [forum][bg-forum]). Reserved for refreshing widgets and a future badge. |
| **D. Live SSE.**                                                                                       | Nothing                        | Yes                            | Foreground only. Out of scope.                                                                                                                          |
| **E. NSE.** It rewrites `mutable-content` alerts on the device, with about 30 s ([docs][nse]).         | Nothing                        | Yes                            | Not needed. Alerts stay generic (Decision 4).                                                                                                           |

`PushTransport` is a folio-core port. folio-infra implements `NoneTransport` and `ApnsTransport`.

**The relay, designed for later, Bitwarden-style:**

- **Enrollment.** A server enrolls once with the relay and receives an installation ID and key. The relay enforces per-installation quotas.
- **Device registration.** The app registers its APNs token **with the relay**, naming the server's relay installation ID from `GET /api/devices` capabilities. The relay returns an opaque handle, and the app gives the server only that handle. The server never sees an APNs token.
- **Sending.** The server sends `{handle, kind_counts: {needs_review, processed, failed}, vault_id?, document_id?, single_vault: bool, locale}`.
- **The relay builds `aps` from its own fixed templates** and picks priority and topic. It rejects handles that belong to another installation. It accepts no free-form text, priority or topic, so an enrolled server can't phish App Store users.
- **Already shaped for it:** the server-side payload builder in this change emits exactly this structured summary, and the APNs adapter renders it locally with the same templates. Adding the relay adds an adapter and a handle column, and changes no requirement.

### Decision 2: Device identity, ownership and revocation

- **Rows.** A device is `(user_id, installation_id)`, upserted by `PUT /api/devices/{installation_id}`. The row stores `client_id` and `grant_id` from the access token. `AuthContext` gains `grant_id`, which stays the same for the life of a sign-in across refresh rotations. Only OAuth requests may register.
- **Revocation in the same transaction.** The folio-infra auth-store adapters delete `push_devices` rows inside the revocation transaction, on the same database:
  - `revoke_grant` and `revoke_token` delete by `grant_id`
  - `revoke_client_tokens(user, client)` deletes by `(user_id, client_id)`
  - deleting a user cascades through a foreign key
- **Liveness backstop.** `grant_is_live(grant_id)` is true only with an unexpired access token, or an unexpired refresh token with `used = 0`. The sender checks it before each send.
- **#306 is a prerequisite** for the server PR. Until it lands, a token issued at the same moment as a revocation can survive, and a device registered with it would still look live. The server PR is rebased onto the #306 fix before merge. The tasks note this, and it adds no extra mechanism here.
- **App side.** The app calls `DELETE /api/devices/{id}` before revoking, best effort, and clears delivered notifications on sign-out.
- **Token uniqueness.** A token is unique per server. Registering a token that another person's row holds deletes that row.

### Decision 3: Triggers — first completion only, recorded per document

The `Processed` payload has no run ID, and `finish_embed_stage` emits it for reprocess, re-embed and repair too. So "first completion" is decided by the planner, not by the event:

- `NotificationPlanner: EventConsumer` handles `Processed` with origin `User` or `Workflow`; `Import` is ignored. It also handles `document.processing.*.failed` envelopes.
- In one transaction it inserts into `push_document_outcomes(document_id, outcome)`, where `outcome` is `completed` or `failed`, with `ON CONFLICT DO NOTHING`.
  - **Row inserted:** this is the document's first completion, or first terminal failure. The planner then writes pending rows.
  - **Row already there:** a reprocess, re-embed, repair or duplicate delivery, so nothing happens.
- **Documents older than the feature.** Their first completion happened before any device registered. A device is offered a document only if `document.created_at` is at or after the device's `created_at`, so a later repair or re-embed of an old Inbox document never notifies.
- **Kind.** `completed` with snapshot `reviewed_at` set gives `processed`; unset gives `needs_review`.
- **Candidates.**
  - Devices with the kind enabled and a matching `vaults` filter.
  - Under `scope: mine`, only devices of `document.uploaded_by`. That is one person, so it is cheap.
  - Under `editable`, devices of people for whom `can(user, vault, EditDocument { uploaded_by })` holds.
  - The vault comes from `origin_vault()`, as in SSE.
  - With no port listing a vault's users, the planner loads devices with `scope = editable` and checks `can` once per distinct user per batch. That is tens of rows on a self-hosted server.
- **Pending rows.** `push_pending(device_id, document_id, kind, vault_id, created_at)` with primary key `(device_id, document_id, kind)`, so counts are distinct documents.

The planner does no network I/O. A slow APNs call therefore can't make the dispatcher replay events to SSE and workflows.

### Decision 4: Payloads stay generic; titles only fetched on the device

```json
{
  "aps": {
    "alert": { "title": "Folio", "body": "3 documents need review" },
    "thread-id": "vault:<uuid>",
    "category": "FOLIO_INBOX",
    "sound": "default"
  },
  "folio": {
    "v": 1,
    "vault": "<uuid>|null",
    "document": "<uuid>|null",
    "counts": { "needs_review": 3, "processed": 0, "failed": 0 }
  }
}
```

- **Text.** Server-side templates fill in the counts, localized from the device's `locale`; Phase 2 ships English and German. `vault` is set only for a single vault, `document` only for a single document, and `thread-id` only for a single vault.
- **Lock screen.** Categories set `hiddenPreviewsBodyPlaceholder`.
- **Title opt-in.** #359 asks for one, because App lock doesn't cover notifications. With `show_titles` on, the server adds `mutable-content: 1`, and the payload still carries no title.
- **`FolioNotificationService` (NSE).** For a single-document alert it fetches the title with the app's credentials from the shared keychain, within a 5 s budget, and sets it as the subtitle.
- **Safeguards:**
  - Any token refresh goes through the existing token provider under `SharedConfiguration.oauthRefreshLock`. A refresh outside it would trip refresh reuse detection (`server.rs:446`) and revoke the grant.
  - An NSE can't suppress an alert without the notification-filtering entitlement. So the base text is always generic, and the NSE adds a title only after a successful fetch: signed out means no credentials, and another account gets a `403`. An alert from a previous session therefore never shows a title.
- **Why fetch on the device:** the opt-in has the same privacy whatever the transport, which matters for a future relay.

### Decision 5: One alert per device per flush, limits and widgets

- **Sender.** `NotificationSender` ticks every 5 s and is woken by the planner.
- **Lease.** It claims due devices with a lease (`push_devices.claimed_until`, set with `UPDATE … WHERE claimed_until < now` on SQLite, `FOR UPDATE SKIP LOCKED` on Postgres). Only one replica sends a device's batch.
- **When a device is due.** Its newest pending row is older than the quiet window (30 s), or its oldest is older than 5 minutes.
- **Before sending, for each device:**
  1. check grant liveness
  2. drop rows for documents now trashed, reviewed (for `needs_review`), or no longer editable
  3. apply the hourly cap
  4. build **one** alert across all vaults
- **After the send.** Sent rows are deleted, and the lease is released. Rows older than 24 h are pruned.
- **Collapse ID.** It is the batch ID, used only so a retry of the same batch doesn't stack. Batches never replace each other's counts.
- **Priority.** 10 for a single document or any `failed`, otherwise 5. `apns-expiration` is now plus 24 h.
- **Widgets.** A `widgets` push with `content-changed` goes to the widget token at most every 15 minutes per device when a covered vault's Inbox changes ([WidgetKit push][widget-push]). There are forum reports of iOS 26 accepting widget pushes without reloading. The widget work therefore keeps its timeline reload policy, and pushes are only an extra nudge.
- **Badge.** Deferred to a follow-up. When it comes, it is an absolute Inbox count across the person's vaults, sent with each alert and reset by the app on open. A background refresh may update it.

### Decision 6: APNs adapter

- **Client.** reqwest with `http2_prior_knowledge()` against `https://api.push.apple.com`, or `api.sandbox.push.apple.com` per device environment. One client and connection is shared.
- **Test endpoint.** `push.apns.base_url`, from config or environment only, replaces both hosts. It allows cleartext h2c for the test stub, and `/settings` and the startup log warn when it is set.
- **Provider token.** ES256 JWT (`kid`, `iss`, `iat`) via `p256::ecdsa`, cached and re-signed after 50 minutes. Apple says no more often than every 20 and at least every 60 ([token auth][apns-token]). It is also re-signed when the key changes.
- **Headers.** `apns-push-type`, `apns-topic` (with `.push-type.widgets` appended for widgets), `apns-priority`, `apns-expiration`, `apns-collapse-id` (a 36-byte UUID) and `apns-id`.
- **Tests.** The request builder and response mapper are pure functions with unit tests. The adapter is tested against an in-process h2c stub.

### Decision 7: Response handling

| Response                                         | Action                                                                                                                                             |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 200                                              | Delete the sent rows and record `last_delivered_at`.                                                                                               |
| 410 `Unregistered` or `ExpiredToken`             | Compare `timestamp` with **that token's** `registered_at`. If later, clear the token and delete the device when no token is left.                  |
| 400 `BadDeviceToken` or `DeviceTokenNotForTopic` | Clear the token. Don't retry.                                                                                                                      |
| 400 or 413, other reasons                        | Log with `apns-id` and drop the batch.                                                                                                             |
| 403 `ExpiredProviderToken`                       | Re-sign once and retry.                                                                                                                            |
| 403, other reasons                               | Mark the transport unhealthy, keep the rows, pause 15 minutes or until settings change, and show the error on `/settings` and in `push_available`. |
| 429                                              | Back off per device, from 1 minute doubling to 30.                                                                                                 |
| 5xx or network error                             | Back off from 15 s to 15 minutes ([responses][apns-resp]).                                                                                         |

### Decision 8: Settings and key storage

- **Keys.** `push.*` keys live in `config.toml`, `SETTINGS` and `docs/configuration.md`, and in the `settings` table. Database rows win and `config.toml` is the fallback, read through a 5-second cache.
- **The `.p8` key in the database.** It is encrypted with `SecretBox` under an `enc:v1:` prefix. It is accepted **only** when `auth.secret_key` came from the environment or a file. Otherwise the save is refused and the operator is pointed at `FOLIO_PUSH__APNS__PRIVATE_KEY_FILE`, because a database copy would hold both the ciphertext and its key.
- **Validation.** A key must parse as PKCS#8 P-256.
- **`push.apns.base_url`** is never read from the database.
- **Preferences** live on the device row.

### Decision 9: Apple app

- **Permission.** Ask after the first finished upload or scan, with a one-line pre-prompt. Settings → Notifications can also ask. Use `.provisional` on first sign-in.
- **Registration.**
  - `PushRegistration` in FolioKit registers through `UIApplicationDelegateAdaptor` or `NSApplicationDelegateAdaptor`.
  - It sends `PUT /api/devices/{installationID}` with platform, environment (Debug is `sandbox`, otherwise `production`), `locale` and preferences, whenever the token, preferences or server change.
  - It never refreshes OAuth tokens outside `oauthRefreshLock`. It goes through the existing token provider.
- **Handling.**
  - `UNUserNotificationCenterDelegate` routes taps to `IntentRouter`.
  - `willPresent` suppresses the banner on the matching Inbox.
  - **Mark reviewed** is `.authenticationRequired` and uses `FolioService`.
- **Sign-out.** `DELETE /api/devices/{id}` before revoking, then `removeAllDeliveredNotifications()`.
- **Extension.** A `FolioNotificationService` target (iOS and macOS) sharing the keychain group and App Group (Decision 4).
- **Boundaries.** The work stays out of `Sharing/`, `FileProvider/`, `Telemetry/` and App Intents, except for reading `InstallationIdentity`.

### Decision 10: JSON API and OpenAPI

`crates/folio-api/src/routes/devices.rs` adds `GET /api/devices`, `PUT` and `DELETE /api/devices/{installation_id}`, and `POST /api/devices/{installation_id}/test`, all annotated with utoipa. `GET /api/devices` also returns `push_available`. `openapi/openapi.json` is regenerated, and the Apple copy in `apple/FolioKit/Sources/FolioAPI/openapi.json` is updated in the Apple PR.

## Risks / Trade-offs

- **[App Store users on third-party servers get no push until a relay exists.]** → `push_available: false`, and the app explains why.
- **[The maintainer's `.p8` key is team-wide.]** → It is never shipped. A future relay fixes text, priority and topic on its own side.
- **[A document processed before its uploader registered the device is never notified.]** → Intended: it prevents floods on enabling push. The Inbox still shows it.
- **[A first completion while the preference was off is not notified later.]** → Intended. Re-enabling applies to new documents.
- **[Revocation races until #306 lands.]** → #306 is a prerequisite for merge. The liveness check catches every other path.
- **[A coalescing delay of up to 30 s.]** → The window is configurable down to 0.
- **[Scanning devices to find `editable` audiences.]** → Bounded by registered devices and cached per batch. A port can replace it later.
- **[Encrypted settings rows are new.]** → They use a prefix scheme, plaintext rows still read, and tests cover both.

## Migration Plan

- **Migration.** `20261108160000_push_devices.sql` in `migrations/sqlite/` and `migrations/postgres/` creates:
  - `push_devices`, with unique `(user_id, installation_id)`:
    - `id`, `user_id` (FK users, cascade), `installation_id`, `client_id`, `grant_id`
    - `platform`, `environment`, `locale`, `preferences` JSON
    - `alert_token`, `alert_token_registered_at`, `widget_token`, `widget_token_registered_at`
    - `created_at`, `updated_at`, `last_delivered_at`
    - indexes on `grant_id` and `(user_id, client_id)`, and unique partial indexes on each token
  - `push_document_outcomes(document_id, outcome, created_at)`, primary key `(document_id, outcome)`, FK to documents with cascade
  - `push_pending(device_id, document_id, kind, vault_id, created_at)`, primary key `(device_id, document_id, kind)`, cascading from devices
  - delivery bookkeeping on the `push_devices` row itself (`claimed_until`, `window_started_at`, `alerts_in_window`, `backoff_until`, `failures`, `widgets_dirty_at`, `widgets_sent_at`, `test_sent_at`), so the lease and the state change in one row

  No existing rows change.

- **Deploy.** The transport defaults to `none`.
- **Rollback.** Set the transport to `none`. The tables can stay until a later migration drops them.
- **Order.**
  1. The #306 fix.
  2. The server PR (`none` and `apns`).
  3. The Apple PR, stacked on the server PR.
  4. The relay adapter, if the owner decides to run a relay.

## Decided Questions

- **Relay.** Designed, not built, pending the owner (Decision 1).
- **Badge.** Deferred. It will be an absolute count, reset on open (Decision 5).
- **Background refresh.** Only for widgets and a future badge, never the alert path (Decision 1).
- **Titles.** An opt-in, fetched on the device by the NSE under the refresh lock; never in payloads (Decision 4).

[apns-token]: https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns
[apns-send]: https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns
[apns-resp]: https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns
[apns-transfer]: https://developer.apple.com/forums/thread/795723
[bw-relay]: https://bitwarden.com/help/configure-push-relay/
[ntfy]: https://docs.ntfy.sh/config/#ios-instant-notifications
[bg-wwdc]: https://wwdcnotes.com/documentation/wwdc19-707-advances-in-app-background-execution/
[bg-forum]: https://developer.apple.com/forums/thread/724506
[nse]: https://developer.apple.com/documentation/usernotifications/modifying-content-in-newly-delivered-notifications
[widget-push]: https://developer.apple.com/documentation/widgetkit/updating-widgets-with-widgetkit-push-notifications

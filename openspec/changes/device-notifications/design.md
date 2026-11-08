# Design

## Context

What the server has today:

- **Outbox.** Every document outcome is written to `outbox_events` in the producing transaction (`OutboxStore`, `crates/folio-core/src/ports/outbox.rs`). `OutboxDispatcher` hands each row to a list of `EventConsumer`s at least once (`crates/folio-core/src/services/outbox_dispatcher.rs`). If one consumer fails, the event is released and every consumer sees it again. Today the consumers are the SSE `BroadcastConsumer` and the `WorkflowEngine`, wired in `folio-app/src/main.rs`.
- **Outcomes.**
  - Success is `DocumentEvent::Processed`. It is emitted by `finish_embed_stage` in `crates/folio-infra/src/queue/worker.rs`, and in the OCR-completion transaction when embedding is skipped (`queue/stages/{sqlite,postgres}.rs`). Its `EventSnapshot` carries `vault_id` and `reviewed_at`.
  - A terminal failure has no `DocumentEvent`. Job settlement (`queue/recovery/{sqlite,postgres}.rs`) writes an envelope-only `document.processing.{ocr|embedding|thumbnail}.failed` row with `audience: Vault`. A retry writes `…retry_scheduled` instead.
  - "Needs review" is derived (`Document::status()`): processed and `reviewed_at IS NULL`. Reviewing emits a `document.reviewed` envelope.
- **SSE.** `/events` streams to open web pages only. The Apple app doesn't use SSE.
- **Auth.** Apple clients use OAuth 2.1 with PKCE and one dynamically registered `client_id` per server. Every code, access token and refresh token of a sign-in shares a `grant_id`. `/oauth/revoke` deletes the whole grant, and the Security page revokes all tokens of a (user, client). `AuthContext` doesn't expose `grant_id` yet.
- **Vault access.** `AccessPolicy::effective_role(user, vault)` resolves direct and group grants. No port lists the users of a vault.
- **Settings.** `config.toml` keys are declared in `SETTINGS` (`crates/folio-infra/src/config.rs`) and mirrored in the `settings` table, edited at admin-only `/settings` cards. Secrets in the table are plaintext today. Only TOTP secrets use `folio_auth::secret::SecretBox`.
- **Apple app.**
  - No push setup yet: no `aps-environment`, no `UNUserNotificationCenter`, no AppDelegate.
  - `IntentRouter` already routes `.inbox` and `.document(id, vaultID:)`.
  - Sign-out runs through `AppModel.signOut()`. A server change is sign-out followed by onboarding.
  - The App Store bundle ID is `com.cedricziel.folio`, owned by the maintainer's developer team.

The hard constraint: APNs authenticates the sender against the **app's team**. A `.p8` key works only for apps of the team that created it, and a request signed by another team's key fails with `InvalidProviderToken` or `DeviceTokenNotForTopic` ([APNs token auth][apns-token], [handling responses][apns-resp], [team ownership after transfer][apns-transfer]). A self-hoster can't make their own key to push to the App Store Folio app. They can push to it only with the maintainer's key, directly or through a relay, or to a Folio they built and signed themselves.

## Goals / Non-Goals

**Goals:**

- A transport-neutral pipeline that installs with push off and keeps working. Turning push on is a settings change.
- A notification is never sent to a device whose person has lost the vault, signed out or revoked the app.
- Payloads are safe to pass through Apple and a third-party relay.
- One code path for alerts and widget reloads.

**Non-Goals:**

- Running the relay service. This change defines the client side of the relay protocol only; see Decision 1.
- In-app live updates over SSE on Apple.
- Live Activities, controls, and widgets themselves, which belong to other sub-issues of #351. This change only accepts and uses widget tokens.
- Email or web push.
- Notifications about workflows, comments or sharing.

## Decisions

### Decision 1: Delivery transport — direct APNs and a relay behind one port; `none` by default

| Option                                            | How it works                                                                                                                                                                                                                                                                                                                                | What a self-hoster configures                            | Works with the App Store app?                      | Notes                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A. Direct APNs**                                | Server opens HTTP/2 to `api.push.apple.com` and signs an ES256 JWT with a `.p8` key ([sending requests][apns-send]).                                                                                                                                                                                                                        | Team ID, key ID, topic, `.p8` key (file or `/settings`). | Only if the server holds the **maintainer's** key. | Simplest and fully private. The key can push to every app of its team, so it must not be handed out ([forum][apns-scope]). Good for the maintainer's own server, and for self-builders with their own bundle ID, team and key.                                                                                                                          |
| **B. Relay**                                      | Server POSTs `{token, environment, push_type, topic suffix, collapse_id, priority, expiration, payload}` to a relay that holds the key and forwards to APNs. This is the model of [Bitwarden][bw-relay] (installation ID and key from bitwarden.com/host) and [ntfy][ntfy] (`upstream-base-url`, only a poll ID and hashed topic upstream). | Relay URL and a per-installation token.                  | Yes.                                               | Needs someone to run, secure and pay for the relay, plus an abuse policy and installation enrollment. The relay sees device tokens, timing and counts, not content (Decision 4).                                                                                                                                                                        |
| **C. Background refresh and local notifications** | `BGAppRefreshTask` wakes the app, which polls the server and posts local notifications.                                                                                                                                                                                                                                                     | Nothing.                                                 | Yes.                                               | iOS decides when, or whether, it runs. `earliestBeginDate` is a floor, not a schedule, and runs can be hours late or skipped ([WWDC19 notes][bg-wwdc], [forum][bg-forum]). It needs per-person usage history and does nothing after a force-quit. macOS needs `NSBackgroundActivityScheduler` instead. Fine as a fallback, not as "your scan is ready". |
| **D. Live SSE while the app runs**                | App subscribes to `/events`.                                                                                                                                                                                                                                                                                                                | Nothing.                                                 | Yes.                                               | Only while the app is in the foreground. iOS suspends sockets in the background. It tells the open app to refresh but doesn't notify. Out of scope here (Non-Goals).                                                                                                                                                                                    |
| **E. Notification Service Extension**             | Not a transport. It runs on the device for alerts with `mutable-content: 1` and has about 30 s to rewrite the content ([docs][nse]).                                                                                                                                                                                                        | Nothing.                                                 | Yes.                                               | Lets the device fetch the title from the server itself (Decision 4). Not needed for the default generic text.                                                                                                                                                                                                                                           |

**Choice:** a `PushTransport` port in folio-core, with three adapters in folio-infra: `none` (default), `apns` (option A) and `relay` (option B). The server pipeline (registration, triggers, coalescing, scoping and token cleanup) is the same for all three.

- **Phase 2 builds `none` and `apns` fully.** That covers the maintainer's own instance, CI and self-builders.
- **The `relay` adapter is a thin HTTP client for a documented JSON contract** (below), so a relay can come later without touching the pipeline.
- **Whether the project runs a relay is an owner decision outside the code.** It is listed under Open Questions, since it changes neither specs nor tasks. Until one exists, App Store users on third-party servers get `push_available: false`.

**Why not C as the main path:** Apple gives no delivery guarantee or minimum frequency, so it can't meet "tell me when it's done". It may come later as a fallback for `push_available: false`. It is listed as a follow-up and kept out of this change.

**Relay contract:**

```
POST {relay.url}/v1/push
Authorization: Bearer {relay.token}
{ "token": "<hex>", "environment": "production|sandbox",
  "push_type": "alert|widgets", "priority": 10|5,
  "collapse_id": "...", "expiration": <unix>, "payload": { "aps": … } }
```

The relay answers with the APNs status and `reason` verbatim. The server handles them exactly as in Decision 7. The relay picks the topic (`com.cedricziel.folio` or `….push-type.widgets`), so a server can't push to other apps of the team.

### Decision 2: Device identity is (person, installation), owned by the OAuth grant

- **Key.** The app already keeps a keychain UUID per installation (`InstallationIdentity`). `PUT /api/devices/{installation_id}` upserts on `(user_id, installation_id)`.
- **Ownership.** The row stores `grant_id` and `client_id`. `AuthContext` gains `grant_id`, and only OAuth-authenticated requests may register.
- **Revocation.** `revoke_grant`, `revoke_token` and `revoke_client_tokens` delete devices for the affected grants. This goes through a `GrantRevocationListener` hook that folio-auth calls and folio-core implements, so folio-auth stays independent of the push tables.
- **Backstop.** The sender also checks `grant_is_live(grant_id)` before each send. Revocation already deletes token rows, so this check is cheap and catches any path that bypasses the hook.
- **App sign-out.** The app calls `DELETE /api/devices/{id}` before revoking, best effort. A server change is a sign-out, so it is covered too.
- **Token uniqueness.** A token is unique per server. Registering a token that another row holds deletes that row, which handles a shared iPad that switches accounts.

**Alternatives considered:**

- Keying on `grant_id` alone breaks when the refresh rotation family changes.
- Keying on the token alone makes token refresh a delete-and-insert and loses preferences.

### Decision 3: Triggers come from an outbox consumer that writes rows; a separate sender does I/O

- **Planner.** `NotificationPlanner` implements `EventConsumer`. For each event it decides the outcome:
  - `Processed` with snapshot `reviewed_at` set gives `processed`; with it unset, `needs_review`.
  - An envelope whose `event_type` matches `document.processing.*.failed` gives `failed`.

  It then lists candidate devices (see below) and inserts `push_pending(device_id, outbox_event_id, kind, vault_id, document_id)` with `ON CONFLICT DO NOTHING`. Re-dispatch is therefore idempotent, and the planner never makes a network call that could hold up SSE or workflows.

- **Sender.** `NotificationSender` is a loop, ticking every 5 s and woken by the planner. For each device whose oldest pending row is older than the quiet window, or than the 5-minute cap, it:
  1. checks the grant
  2. checks the vault role for each pending row and drops rows the person can no longer see
  3. drops `needs_review` rows whose document is now reviewed
  4. applies the hourly cap from `push_device_state`
  5. builds one payload per vault
  6. sends it, then deletes the rows sent

  Pending rows older than 24 hours are pruned.

- **Candidate devices.** No port lists a vault's users, and adding one that expands groups is a cross-cutting change. The planner loads all devices that have the outcome enabled. On a self-hosted server that is tens of rows. It then calls `effective_role(user, vault)` once per distinct user and caches the result for the batch. If device counts grow, a `VaultAccess::users_with_access(vault)` query can replace this without changing behavior.

**Alternatives considered:**

- Sending from inside the consumer: an APNs timeout would release the event and replay it to SSE and workflows.
- Putting pushes through the jobs table: that mixes user-facing processing jobs with delivery, and coalescing needs per-device state anyway.

### Decision 4: Payloads reveal nothing by default; titles only through the NSE, fetched from the server

Default payload:

```json
{
  "aps": {
    "alert": { "title": "Folio", "body": "3 documents need review" },
    "thread-id": "vault:<uuid>",
    "category": "FOLIO_INBOX",
    "sound": "default",
    "mutable-content": 0
  },
  "folio": {
    "v": 1,
    "vault": "<uuid>",
    "kinds": { "needs_review": 3 },
    "document": null
  }
}
```

- **Text.** The body is built on the server from counts. The server localizes it from the `locale` the device sends; Phase 2 ships English and German, matching the app.
- **Single document.** `document` carries the document ID only when exactly one document is covered.
- **Title opt-in.** With `show_titles` on, the server sets `mutable-content: 1`. `FolioNotificationService` reads the shared keychain token, calls `GET /api/documents/{id}`, and sets `subtitle` to the title. It has a 5 s budget and keeps the generic text on failure.
- **Why fetch on the device.** The title never leaves the server inside APNs or relay traffic, so the opt-in has the same privacy whichever transport is used. That matters most for the relay.
- **Lock screen.** Categories set `hiddenPreviewsBodyPlaceholder` ("Document update"), so "Show Previews: When Unlocked" shows nothing specific. App lock doesn't cover notifications, so the opt-in screen says so.

**Alternative considered:** putting the title in the payload when the transport is `apns`. That is simpler, but it sends titles through Apple and makes privacy depend on a server setting the person can't see. Rejected.

### Decision 5: Coalescing, grouping and rate limits

- **Quiet window.** 30 s, configurable, with a 5-minute ceiling. A steady import still produces an alert every 5 minutes at most.
- **Hourly cap.** 12 alerts per device per rolling hour, configurable. Held rows roll into the next allowed alert.
- **Collapse and threading.** `apns-collapse-id = "<kind-set>:<vault>"`, at most 64 bytes, so a newer summary replaces the older one on screen. `thread-id` is per vault, so Notification Center groups by vault.
- **Priority and expiry.** Priority 10 for `failed`, 5 for the others. `apns-expiration` is now plus 24 h.
- **Badge.** Off in Phase 2. An Inbox badge needs a per-person count across vaults and clearing semantics, so it is listed as a follow-up.
- **Widgets.** One `widgets` push per device every 15 minutes at most, sent when any `needs_review`, `processed`, `failed` or `document.reviewed` change touches a vault the device covers. The system budgets these anyway ([WidgetKit push][widget-push]).

### Decision 6: APNs adapter details

- **Client.** A reqwest client built with `http2_prior_knowledge()` against `api.push.apple.com:443`, or `api.sandbox.push.apple.com` per device environment. One client is shared, since it pools one connection per host and reuses it, as Apple recommends.
- **Provider token.** ES256 JWT `{alg: ES256, kid}` and `{iss: team_id, iat}`, signed with `p256::ecdsa`. It is cached and re-signed after 50 minutes. Apple says to refresh no more often than every 20 minutes and no less often than every 60 ([token auth][apns-token]); refreshing more often causes `429 TooManyProviderTokenUpdates`. A key change in settings forces a new token.
- **Headers.** `apns-push-type` (`alert` or `widgets`), `apns-topic` (topic, or topic with `.push-type.widgets` appended), `apns-priority`, `apns-expiration`, `apns-collapse-id`, and an `apns-id` UUID that is logged with the outcome.
- **Testing.** Request building and response mapping are pure functions with unit tests. The adapter is tested end to end against a local HTTP/2 stub (`hyper` server, h2c) that the base URL is pointed at. No test talks to Apple.

### Decision 7: Response handling

| Response                                         | Action                                                                                                                                                                                             |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 200                                              | Delete the sent pending rows and update `last_delivered_at`.                                                                                                                                       |
| 410 `Unregistered` or `ExpiredToken`             | If `timestamp` is later than the token's `updated_at`, clear the token and delete the device if no token is left. Otherwise keep it, since the token was re-registered after Apple invalidated it. |
| 400 `BadDeviceToken` or `DeviceTokenNotForTopic` | Clear the token. Don't retry.                                                                                                                                                                      |
| 400 or 413, other reasons                        | Log as a bug with `apns-id`, drop the rows. Don't retry.                                                                                                                                           |
| 403 `ExpiredProviderToken`                       | Re-sign once and retry.                                                                                                                                                                            |
| 403, other reasons                               | Mark the transport unhealthy and keep rows. Health is reported in `/settings` and `push_available`. The sender pauses for 15 minutes, or until settings change.                                    |
| 429 `TooManyRequests`                            | Back off for that device: 1 min, doubling up to 30 min.                                                                                                                                            |
| 5xx or network error                             | Retry with backoff from 15 s up to 15 min. Apple says 5xx may be retried after 15 minutes ([handling responses][apns-resp]); the backoff reaches that within a few attempts.                       |

### Decision 8: Settings

- **Storage.** `push.*` keys live in `config.toml` (`PushConfig` in `config.rs`, declared in `SETTINGS`, documented in `docs/configuration.md`) and in the `settings` table. Precedence is the same as for pipeline settings: database rows win and `config.toml` is the fallback.
- **Secrets.** `push.apns.private_key` and `push.relay.token` are stored with `SecretBox`, using the key already used for TOTP secrets. That gives the first encrypted rows in `settings`, under an `enc:v1:` prefix, so plain values still read.
- **Validation.** A key is accepted only if it parses as PKCS#8 P-256.
- **Reloading.** The effective configuration is read through a 5-second cache, like the AI settings, so a save applies without a restart.
- **Per-person preferences** live on the device row, because they differ between an iPhone and a Mac.

### Decision 9: Apple app

- **Permission.** Ask after the first upload or scan the app sees finish, with a one-line pre-prompt ("Get notified when Folio has read your scan?"). Settings → Notifications can also ask. Use `.provisional` authorization on first sign-in, so quiet notifications can arrive before the prompt, on iOS and macOS.
- **Registration.**
  - `PushRegistration` in FolioKit calls `registerForRemoteNotifications()` through a `UIApplicationDelegateAdaptor` or `NSApplicationDelegateAdaptor`, then `PUT /api/devices/{installationID}` whenever the token, preferences or server change.
  - The environment comes from the build configuration: Debug is `sandbox`, Release and TestFlight are `production`.
  - Widget tokens are passed in later by the widget extension's `WidgetPushHandler`, through the App Group.
- **Handling.**
  - `UNUserNotificationCenterDelegate` routes taps to `IntentRouter.shared.route(.document(id, vaultID:))` or `.inbox`.
  - `willPresent` shows a banner only when the app isn't showing that vault's Inbox, and refreshes the list otherwise.
  - **Mark reviewed** is a background action with `.authenticationRequired` that calls the existing review endpoint through `FolioService`.
- **Sign-out.** `AppModel.signOut()` calls `DELETE /api/devices/{id}` before revoking, and clears delivered notifications (`removeAllDeliveredNotifications`).
- **Extension.** The new `FolioNotificationService` target is iOS only for the opt-in. macOS supports NSEs too, so it is added to both if the build allows. It uses the existing keychain access group and App Group.
- **Boundaries.** The app work stays out of `Sharing/`, `FileProvider/`, `Telemetry/` and App Intents. It only reads `InstallationIdentity`.

### Decision 10: JSON API and OpenAPI

New utoipa-annotated routes go in `crates/folio-api/src/routes/devices.rs`: `devices.list`, `devices.put`, `devices.delete` and `devices.test`. Schemas: `DeviceRegistration`, `Device`, `NotificationPreferences` and `VaultSelection`. `openapi/openapi.json` is regenerated, and the Apple copy at `apple/FolioKit/Sources/FolioAPI/openapi.json` is updated in the Apple PR so the generated Swift client gains `Operations.devices_*`.

## Risks / Trade-offs

- **[App Store users on third-party servers get no push until a relay exists.]** → Registration still works and reports `push_available: false`. The app explains why in Settings → Notifications. The background-refresh fallback stays a follow-up option.
- **[The maintainer's `.p8` key is team-wide.]** → It is never shipped or shared. A relay, if run, holds it and fixes the topic server-side.
- **[A coalescing delay of up to 30 s feels slower than a per-event push.]** → For a single upload the window ends 30 s after processing. That is acceptable for "your scan is ready", and the window is configurable down to 0.
- **[At-least-once outbox plus at-least-once APNs can duplicate an alert.]** → The unique key on `push_pending` and the collapse ID make a duplicate replace the earlier alert rather than stack.
- **[Iterating devices to resolve the audience doesn't scale to large installs.]** → The query is bounded by the number of registered devices and cached per batch. A port can replace it later (Decision 3).
- **[The NSE adds a target, an extension memory limit (about 24 MB) and a network call during delivery.]** → It runs only when `show_titles` is on, and falls back to the generic text.
- **[Encrypted settings rows are new.]** → They use a prefix scheme, existing plaintext keeps working, and a unit test covers both.

## Migration Plan

- **Migration.** `20261108160000_push_devices.sql` in `migrations/sqlite/` and `migrations/postgres/` creates:
  - `push_devices(id, user_id, installation_id, client_id, grant_id, platform, environment, alert_token, widget_token, locale, preferences JSON, created_at, updated_at, last_delivered_at)`, with unique `(user_id, installation_id)`, unique `alert_token` and unique `widget_token` (nullable)
  - `push_pending(device_id, outbox_event_id, kind, vault_id, document_id, created_at)`, with primary key `(device_id, outbox_event_id, kind)`, ON DELETE CASCADE from `push_devices`
  - `push_device_state(device_id, window_started_at, alerts_in_window, backoff_until, widgets_sent_at)`
  - a foreign key from `user_id` to `users` with ON DELETE CASCADE

  No existing rows change.

- **Deploy.** The default transport is `none`, so upgrading changes nothing until an operator configures push.
- **Rollback.** Set the transport to `none`. The tables can stay. Reverting the code leaves unused tables, which a later migration can drop.
- **Order.** The server PR lands first, the Apple PR stacks on it, and the NSE comes last within the Apple PR.

## Open Questions

- **Will the project run a relay for the App Store build** (hosting, enrollment and abuse limits, privacy notice)? This doesn't change this change's specs or tasks. It decides only whether `push.transport = "relay"` gets a public default URL later.
- **Do we want an app icon badge with the Inbox count?** That would be a follow-up and needs cross-vault count semantics.
- **Do we want the background-refresh fallback (option C)** for servers with `push_available: false`? That would be a follow-up issue.

[apns-token]: https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns
[apns-send]: https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns
[apns-resp]: https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns
[apns-transfer]: https://developer.apple.com/forums/thread/795723
[apns-scope]: https://developer.apple.com/forums/thread/831957
[bw-relay]: https://bitwarden.com/help/configure-push-relay/
[ntfy]: https://docs.ntfy.sh/config/#ios-instant-notifications
[bg-wwdc]: https://wwdcnotes.com/documentation/wwdc19-707-advances-in-app-background-execution/
[bg-forum]: https://developer.apple.com/forums/thread/724506
[nse]: https://developer.apple.com/documentation/usernotifications/modifying-content-in-newly-delivered-notifications
[widget-push]: https://developer.apple.com/documentation/widgetkit/updating-widgets-with-widgetkit-push-notifications

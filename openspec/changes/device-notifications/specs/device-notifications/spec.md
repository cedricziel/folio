# Spec Delta

## Purpose

Tell people on their Apple devices when their documents finish processing, need review or fail, without revealing document contents and without breaking self-hosted installs that have no push credentials.

## ADDED Requirements

### Requirement: Device registration

An OAuth client SHALL register a device with `PUT /api/devices/{installation_id}`. The body SHALL carry:

- `platform`: `ios` or `macos`
- `environment`: `sandbox` or `production`
- `locale`
- optional `alert_token` and `widget_token`
- `preferences`

A device SHALL be identified by the person and `installation_id`, and SHALL belong to the request's OAuth client and grant. Registering again SHALL replace the preferences, the grant and any token sent. Each token SHALL keep its own registration time.

The response SHALL include `push_available`. Personal access tokens and session cookies SHALL be rejected.

#### Scenario: First registration

- **WHEN** a signed-in app sends `PUT /api/devices/{installation_id}` with an alert token and locale `de`
- **THEN** the server responds `201` with the device, its default preferences and `push_available`

#### Scenario: Token refresh

- **WHEN** the same installation sends `PUT /api/devices/{installation_id}` with a new alert token
- **THEN** the server responds `200`, and only the alert token's registration time changes

#### Scenario: Same token moves to another person

- **WHEN** a token registered for one person is registered by another person on the same server
- **THEN** the earlier registration holding that token is deleted

#### Scenario: Personal access token

- **WHEN** `PUT /api/devices/{installation_id}` is called with a personal access token
- **THEN** the server responds `403` and stores nothing

#### Scenario: Invalid body

- **WHEN** the body has an unknown platform or environment, a non-hexadecimal token, or a vault the caller cannot access
- **THEN** the server responds `422` and stores nothing

#### Scenario: Push not configured

- **WHEN** the push transport is `none`
- **THEN** registration succeeds with `push_available: false` and nothing is ever sent

### Requirement: Notification preferences

Preferences SHALL contain:

| Preference                             | Default |
| -------------------------------------- | ------- |
| `processed`                            | off     |
| `needs_review`                         | on      |
| `failed`                               | on      |
| `scope`: `mine` or `editable`          | `mine`  |
| `vaults`: `all` or a list of vault IDs | `all`   |
| `show_titles` | off |

`mine` limits notifications to documents the person uploaded. `editable` widens them to every document the person may edit.

#### Scenario: Defaults

- **WHEN** a device registers without preferences
- **THEN** it receives `needs_review` and `failed` notifications for documents its person uploaded, in every vault they can access

### Requirement: Listing and removing devices

`GET /api/devices` SHALL return the caller's devices without token values. `DELETE /api/devices/{installation_id}` SHALL delete the caller's device and its pending notifications, and SHALL be idempotent.

#### Scenario: List

- **WHEN** a person with two devices calls `GET /api/devices`
- **THEN** the server responds `200` with both devices, their platforms, preferences and last delivery time, and no tokens

#### Scenario: Delete

- **WHEN** a device calls `DELETE /api/devices/{installation_id}` twice
- **THEN** both calls respond `204`

#### Scenario: Another person's device

- **WHEN** a person deletes an installation ID registered only by someone else
- **THEN** the server responds `204` and the other device is unchanged

### Requirement: Registrations end with the app's authorization

A device SHALL stop receiving notifications when its app's authorization ends. This covers:

- the app's sign-out, which revokes its grant
- revoking the app on the web Security page
- refresh-token reuse
- deleting the user

The device rows SHALL be deleted in the same transaction as the revocation. Before sending, the sender SHALL skip and delete any device whose grant has no unexpired access token and no unused, unexpired refresh token.

#### Scenario: Sign-out

- **WHEN** the app revokes its refresh token on sign-out
- **THEN** the device registered under that grant is deleted, and no later outcome produces a push to it

#### Scenario: Revoked on the web

- **WHEN** a person revokes the Folio app under connected apps
- **THEN** every device that person registered through that client is deleted in the same transaction

#### Scenario: Only a used refresh token remains

- **WHEN** a device's grant has only refresh tokens already rotated (used)
- **THEN** the sender sends nothing to it and deletes it

### Requirement: Who is notified

A device SHALL receive a notification about a document only when all of the following hold at the time of sending:

- its person may edit that document in the document's vault
- the vault is not deleted
- the document is not trashed
- the device's `scope` and `vaults` preferences include it

Under `scope: mine`, the document SHALL also have been uploaded by the device's person.

#### Scenario: Viewer is not notified

- **WHEN** a viewer of a vault has `scope: editable` and another member's upload needs review
- **THEN** the viewer's devices receive nothing, because they could not mark it reviewed

#### Scenario: Uploader by default

- **WHEN** an editor's own upload needs review, and another editor's upload in the same vault needs review too
- **THEN** with default preferences the first editor is notified only about their own upload

#### Scenario: Access removed while pending

- **WHEN** a person loses access to the vault before the pending notification is sent
- **THEN** it is dropped without sending

#### Scenario: Trashed while pending

- **WHEN** a document is moved to the trash while its notification is pending
- **THEN** it is dropped without sending

#### Scenario: Group membership

- **WHEN** a person can edit in a vault only through a group
- **THEN** their devices are notified as for a direct member

### Requirement: Notification triggers

Each document SHALL produce at most one notification per device for each outcome:

- `processed`: the first completed processing of a document that is already reviewed
- `needs_review`: the first completed processing of a document that is unreviewed
- `failed`: a processing stage fails terminally, after its retries are used up

Completions caused by reprocessing, re-embedding, repair or an import, and retries that are still scheduled, SHALL NOT notify. A device SHALL NOT be notified about documents created before it registered.

#### Scenario: Upload needs review

- **WHEN** a person uploads a document and processing finishes while it is unreviewed
- **THEN** each of their eligible devices gets one pending `needs_review` notification

#### Scenario: Re-embedding the Inbox

- **WHEN** an admin re-embeds every document, and 300 unreviewed documents complete again
- **THEN** no notification is produced

#### Scenario: Reprocessing after a notification

- **WHEN** a document that already produced `needs_review` is reprocessed and completes again
- **THEN** no second notification is produced

#### Scenario: Retry is not a failure

- **WHEN** OCR fails and a retry is scheduled
- **THEN** no notification is produced, and one `failed` notification is produced only if the last attempt fails

#### Scenario: Duplicate delivery

- **WHEN** the outbox delivers the same event twice
- **THEN** the document is counted once

#### Scenario: AI provider unconfigured

- **WHEN** no AI provider is configured, so processing ends without embeddings
- **THEN** the outcome is still produced when processing finishes

### Requirement: Coalescing and rate limits

The server SHALL hold a device's pending notifications until none has arrived for the quiet window (default 30 seconds), or for at most 5 minutes after the first one. It SHALL then send one alert covering all of them, across vaults.

The alert SHALL count distinct documents per outcome:

- "1 document needs review"
- "4 documents need review"
- "3 documents need review, 1 failed"

It SHALL carry a per-vault `thread-id` only when every document is in one vault. A device SHALL receive no more alerts per rolling hour than the hourly cap (default 12), and held outcomes SHALL fold into the next allowed alert.

Each pending notification SHALL be sent by one server process only, even when several replicas run.

#### Scenario: Bulk upload

- **WHEN** 200 documents a person uploaded finish processing within 3 minutes
- **THEN** each of their devices receives at most two alerts in that time, together counting 200 documents

#### Scenario: Several vaults

- **WHEN** documents in two vaults need review within one quiet window
- **THEN** the device receives one alert counting both, with no `thread-id`, that opens the Inbox

#### Scenario: Hourly cap reached

- **WHEN** a device has received the hourly cap of alerts and another outcome arrives
- **THEN** no alert is sent until the hour allows one, and that alert counts the held outcomes

#### Scenario: Reviewed before sending

- **WHEN** a document is marked reviewed while its `needs_review` notification is pending
- **THEN** it is left out of the alert, and no alert is sent if nothing else is pending

#### Scenario: Two replicas

- **WHEN** two Folio processes share a database and a device has pending notifications
- **THEN** exactly one alert is sent for them

### Requirement: Non-revealing payloads

An alert SHALL contain only:

- generic localized text built from counts and outcomes
- the vault ID when exactly one vault is covered
- the document ID when exactly one document is covered
- a category

It SHALL NOT contain a document title, filename, tag, correspondent, extracted text or vault name, whatever transport is used. When the device has `show_titles` on, the alert SHALL also carry `mutable-content: 1`, so the device can fetch the title itself. Alerts SHALL carry a lock-screen placeholder that names no document.

Notification text SHALL NOT be free-form input to the transport. It SHALL come only from fixed templates filled with counts.

#### Scenario: Default text

- **WHEN** one document needs review
- **THEN** the alert reads "1 document needs review", and the request to APNs contains no title

#### Scenario: Locked device

- **WHEN** the person's device is set to show previews only when unlocked
- **THEN** the locked screen shows "Folio" and the placeholder, which names no document

### Requirement: Title opt-in

With `show_titles` on, the device SHALL fetch a single document's title from the server with its own credentials and show it in the alert. It SHALL show the generic text when any of these hold:

- the fetch fails or takes longer than 5 seconds
- the device is signed out
- the server refuses access

The fetch SHALL NOT refresh credentials outside the app's shared refresh lock. The settings screen SHALL say that App lock does not cover notifications.

#### Scenario: Title shown

- **WHEN** one document needs review on a device with `show_titles` on
- **THEN** the APNs payload has no title, and the device shows the title it fetched

#### Scenario: Signed out before delivery

- **WHEN** an alert queued before sign-out arrives after it
- **THEN** the device has no credentials and shows the generic text

#### Scenario: Token refresh during the fetch

- **WHEN** the extension's access token has expired while the app is refreshing
- **THEN** the extension waits for the shared refresh lock and uses the refreshed token, and the grant is not revoked for refresh-token reuse

### Requirement: Deep links and actions

An alert SHALL open:

- the document, in its vault, when it covers one document
- that vault's Inbox, when it covers several documents in one vault
- the Inbox of the current vault, when it covers several vaults

The single-document category SHALL offer **Open** and **Mark reviewed**. Mark reviewed SHALL need the device unlocked and SHALL use the existing review API. If access is gone or the document is missing, opening SHALL show the Inbox.

#### Scenario: Open one document

- **WHEN** a person taps a single-document alert
- **THEN** the app opens that document in its vault

#### Scenario: Mark reviewed from the notification

- **WHEN** a person chooses Mark reviewed on a locked device
- **THEN** the system asks them to unlock, the document is marked reviewed, and the alert is removed

#### Scenario: Several vaults

- **WHEN** a person taps an alert covering two vaults
- **THEN** the app opens the Inbox of its current vault

### Requirement: Invalid tokens

The server SHALL clear a token that APNs reports as inactive:

- `410 Unregistered` or `ExpiredToken`, when the reported time is after that token's own registration time
- `400 BadDeviceToken` or `DeviceTokenNotForTopic`

A device left with no token SHALL be deleted. The server SHALL retry `429` and `5xx` responses with backoff, keeping the pending notifications.

A provider credential error SHALL stop sending, mark push unavailable, and keep pending notifications for up to 24 hours.

#### Scenario: App deleted

- **WHEN** APNs answers `410 Unregistered` for an alert token registered before the reported time
- **THEN** the alert token is cleared, even if the device's widget token was refreshed later

#### Scenario: Wrong environment

- **WHEN** APNs answers `BadDeviceToken`
- **THEN** that token is cleared and not retried

#### Scenario: Bad key

- **WHEN** APNs answers `403 InvalidProviderToken`
- **THEN** sending stops, `/settings` shows the error, and registrations report `push_available: false`

### Requirement: Widget reload pushes

When the Inbox of a vault the device covers changes, the server SHALL send a `widgets` push with only `content-changed: true` to the device's widget token. These pushes SHALL go at most once per device every 15 minutes, and SHALL follow the same scoping and revocation rules as alerts. They SHALL NOT count toward the alert cap.

#### Scenario: Inbox changes

- **WHEN** three documents enter the Inbox within a minute for a device with a widget token
- **THEN** one `widgets` push is sent, with only `content-changed: true` in `aps`

### Requirement: Test notification

`POST /api/devices/{installation_id}/test` SHALL respond:

- `202` after queueing a test alert to the caller's device
- `409` when push is unavailable
- `429` after more than one test in a minute

#### Scenario: Test from the app

- **WHEN** the app asks for a test on a configured server
- **THEN** the server responds `202` and the device receives "Notifications from Folio are working"

#### Scenario: Test without push

- **WHEN** the transport is `none`
- **THEN** the server responds `409`

# Spec Delta

## Purpose

Tell people on their Apple devices when their documents finish processing, need review or fail, without revealing document contents and without breaking self-hosted installs that have no push credentials.

## ADDED Requirements

### Requirement: Device registration

An OAuth client SHALL register a device with `PUT /api/devices/{installation_id}`. The body SHALL carry:

- `platform`: `ios` or `macos`
- `environment`: `sandbox` or `production`
- `alert_token`: the APNs token for alerts, optional
- `widget_token`: the WidgetKit push token, optional
- `preferences`: `processed`, `needs_review`, `failed`, `show_titles` and `vaults`

`vaults` SHALL be either `all` or a list of vault IDs. A device SHALL be identified by the pair of the person and `installation_id`, and SHALL belong to the OAuth grant of the request's access token. Registering again with the same `installation_id` SHALL replace the tokens, preferences and grant.

The defaults SHALL be:

- `processed`: off
- `needs_review`: on
- `failed`: on
- `show_titles`: off
- `vaults`: `all`

The response SHALL include `push_available`, which is false when the server's transport is `none` or failing its credential check.

Requests authenticated with a personal access token or a session cookie SHALL be rejected. An APNs token SHALL be stored only by the server that received it, and SHALL NOT appear in logs, traces or error messages except as a short hash.

#### Scenario: First registration

- **WHEN** a signed-in app sends `PUT /api/devices/{installation_id}` with an alert token
- **THEN** the server responds `201` with the stored device, its default preferences and `push_available`

#### Scenario: Token refresh

- **WHEN** the same installation sends `PUT /api/devices/{installation_id}` with a new token
- **THEN** the server responds `200`, the old token is no longer used, and pending notifications for the device are kept

#### Scenario: Same token moves to another person

- **WHEN** a token already registered for one person is registered by another person on the same server
- **THEN** the earlier registration that holds that token is deleted, so one physical device never receives another person's notifications

#### Scenario: Personal access token

- **WHEN** `PUT /api/devices/{installation_id}` is called with a personal access token
- **THEN** the server responds `403` and stores nothing

#### Scenario: Invalid body

- **WHEN** the body has an unknown platform or environment, a token that is not hexadecimal, or a vault the caller cannot access
- **THEN** the server responds `422` and stores nothing

#### Scenario: Push not configured

- **WHEN** the push transport is `none`
- **THEN** registration still succeeds with `push_available: false`, and no notification is ever sent

### Requirement: Listing and removing devices

`GET /api/devices` SHALL return the caller's devices, without their tokens. `DELETE /api/devices/{installation_id}` SHALL delete the caller's device and its pending notifications. It SHALL be idempotent.

#### Scenario: List

- **WHEN** a person with two registered devices calls `GET /api/devices`
- **THEN** the server responds `200` with both devices, their platforms, preferences and last delivery time, and no token values

#### Scenario: Delete

- **WHEN** a device calls `DELETE /api/devices/{installation_id}`
- **THEN** the server responds `204`, and a second identical call also responds `204`

#### Scenario: Another person's device

- **WHEN** a person calls `DELETE` with an installation ID registered only by someone else
- **THEN** the server responds `204` and the other person's device is unchanged

### Requirement: Registrations end with the grant

A device registration SHALL stop receiving notifications as soon as the OAuth grant it belongs to is revoked. This covers:

- the app's sign-out, which posts to `/oauth/revoke`
- revoking the app on the web Security page
- refresh-token reuse that revokes the token family
- deleting the user

Revocation SHALL delete the grant's devices. The sender SHALL also skip, and then delete, any device whose grant has no live token when it is about to send.

#### Scenario: Sign-out

- **WHEN** the app signs out and revokes its refresh token
- **THEN** the device registered under that grant is deleted, and no later outcome produces a push to it

#### Scenario: Revoked on the web

- **WHEN** a person revokes the Folio app under connected apps on the Security page
- **THEN** every device registered through that client for that person is deleted

#### Scenario: Missed cleanup

- **WHEN** a device row survives although its grant has no live token
- **THEN** the sender sends nothing to it and deletes it

### Requirement: Vault scoping

A device SHALL receive a notification about a document only when all of these hold:

- the device's person has a role in the document's vault at the time of sending
- the vault is not deleted
- the device's `vaults` preference includes that vault

Losing access between the outcome and the send SHALL suppress the notification.

#### Scenario: Revoked vault

- **WHEN** a person's membership in a vault is removed and a document in that vault then finishes processing
- **THEN** none of that person's devices receives a notification for it

#### Scenario: Access removed while pending

- **WHEN** a notification is pending for a device and the person loses access to the vault before the quiet window ends
- **THEN** the pending notification is dropped without sending

#### Scenario: Group membership

- **WHEN** a person can reach a vault only through a group
- **THEN** their devices receive notifications for that vault as for a direct member

#### Scenario: Vault filter

- **WHEN** a device's `vaults` preference lists only vault A
- **THEN** outcomes in vault B produce no notification on that device

### Requirement: Notification triggers

Each outcome SHALL be offered to every eligible device whose preference for it is on:

- `processed`: processing of a document finishes and the document is already reviewed
- `needs_review`: processing finishes and the document is unreviewed
- `failed`: a processing stage fails terminally, after its retries are used up

A retry-scheduled failure SHALL NOT notify. An outcome caused by a workflow or an import SHALL be treated like any other.

Each outcome SHALL be offered at most once per device, even when the outbox delivers the event more than once.

#### Scenario: Upload needs review

- **WHEN** a person uploads a document and processing finishes while it is unreviewed
- **THEN** each of their eligible devices gets one pending `needs_review` notification

#### Scenario: Reprocessing a filed document

- **WHEN** a reviewed document is reprocessed
- **THEN** the outcome is `processed`, which is off by default, so nothing is sent unless the device opted in

#### Scenario: Retry is not a failure

- **WHEN** OCR fails and a retry is scheduled
- **THEN** no notification is produced, and one `failed` notification is produced only if the last attempt also fails

#### Scenario: Duplicate delivery

- **WHEN** the outbox dispatcher delivers the same event twice
- **THEN** each device still has one pending notification for it

#### Scenario: AI provider unconfigured

- **WHEN** no AI provider is configured, so processing ends without embeddings
- **THEN** the `processed` or `needs_review` outcome is still produced when processing finishes

### Requirement: Coalescing and rate limits

The server SHALL hold a device's pending notifications until no new one has arrived for the quiet window (default 30 seconds), or for at most 5 minutes after the first one. It SHALL then send one alert that summarizes them.

The alert SHALL:

- name the count per outcome: "1 document needs review", "4 documents need review", or "3 documents need review, 1 failed"
- carry a per-vault `thread-id`
- carry an `apns-collapse-id` per vault and outcome, so a later summary replaces an earlier one

A device SHALL receive no more alerts per rolling hour than the hourly cap (default 12). Outcomes beyond the cap SHALL be folded into the next alert allowed.

#### Scenario: Bulk import

- **WHEN** 200 documents finish processing for one person within 3 minutes
- **THEN** each of their devices receives at most two alerts in that time, together counting all 200 documents

#### Scenario: Single document

- **WHEN** one document finishes and nothing else follows within the quiet window
- **THEN** the device receives one alert about one document, which opens that document

#### Scenario: Hourly cap reached

- **WHEN** a device has received the hourly cap of alerts and another outcome arrives
- **THEN** no alert is sent until the hour allows one, and that alert counts the held outcomes

#### Scenario: Reviewed before sending

- **WHEN** a document is marked reviewed while its `needs_review` notification is still pending
- **THEN** it is left out of the alert, and no alert is sent if nothing else is pending

### Requirement: Non-revealing payloads

By default an alert SHALL contain only:

- generic text built from counts and outcomes
- the vault ID
- the document ID when exactly one document is covered
- a category

It SHALL NOT contain a document title, filename, tag, correspondent, extracted text or vault name. Showing titles SHALL be a per-device opt-in (`show_titles`).

When the opt-in is on, the payload SHALL ask the device to fetch the title itself (`mutable-content`). The title SHALL still not be part of the payload, whatever transport is used. If the fetch fails or takes too long, the device SHALL show the generic text.

Alerts SHALL also carry a lock-screen placeholder that names no document. People who choose "Show Previews: When Unlocked" then see only "Folio" and the placeholder while locked.

#### Scenario: Default text

- **WHEN** one document needs review on a device without the title opt-in
- **THEN** the alert reads "1 document needs review", and neither the APNs payload nor any relay request contains the title

#### Scenario: Titles opted in

- **WHEN** the device has `show_titles` on and one document needs review
- **THEN** the payload has `mutable-content: 1` and the document ID but no title, and the device shows the title it fetched from the server

#### Scenario: Title fetch fails

- **WHEN** the device can't reach the server within the extension's time limit
- **THEN** the generic text is shown

#### Scenario: App lock

- **WHEN** App lock is on
- **THEN** the alert text stays generic unless the device opted in to titles, because App lock does not cover notifications

### Requirement: Deep links and actions

An alert covering one document SHALL open that document in its vault. An alert covering several documents SHALL open the Inbox of the vault, or the Inbox of the current vault when the documents span vaults.

The single-document category SHALL offer two actions:

- **Open**, which brings the app to the foreground
- **Mark reviewed**, which needs the device unlocked and marks the document reviewed through the existing API without bringing the app forward

If the person no longer has access, or the document is gone, opening SHALL show the Inbox.

#### Scenario: Open one document

- **WHEN** a person taps a single-document alert
- **THEN** the app opens that document in the named vault

#### Scenario: Mark reviewed from the notification

- **WHEN** a person chooses Mark reviewed on a locked device
- **THEN** the system asks them to unlock, then the document is marked reviewed and the delivered alert is removed

#### Scenario: Many documents

- **WHEN** a person taps an alert covering several documents
- **THEN** the app opens the Inbox of that vault

### Requirement: Invalid tokens

The server SHALL delete a token that APNs reports as inactive. This covers `410 Unregistered` or `ExpiredToken` when the reported time is after the token was registered, and `400 BadDeviceToken` or `DeviceTokenNotForTopic`. The server SHALL NOT retry those. It SHALL retry `429` and `5xx` responses with backoff, and SHALL keep the pending notifications meanwhile.

A credential error (`403` with a provider-token reason) SHALL stop sending, mark push as unavailable in `/settings` and in registration responses, and keep pending notifications until the credentials are fixed or the notifications expire after 24 hours.

#### Scenario: App deleted

- **WHEN** APNs answers `410 Unregistered` for a token registered before the reported timestamp
- **THEN** the token is removed from its device, and a device with no tokens left is deleted

#### Scenario: Wrong environment

- **WHEN** a development build registers a token as `production` and APNs answers `BadDeviceToken`
- **THEN** that token is removed and not retried

#### Scenario: Bad key

- **WHEN** APNs answers `403 InvalidProviderToken`
- **THEN** sending stops, `/settings` shows the error, and registration responses report `push_available: false`

### Requirement: Widget reload pushes

When a vault's Inbox count changes for a person whose device registered a widget token, the server SHALL send a widgets push with `content-changed: true` to that token. The push SHALL use push type `widgets` and the bundle ID with `.push-type.widgets` as the topic. These pushes SHALL be coalesced to at most one per device every 15 minutes, and SHALL carry no document data. Widget pushes SHALL follow the same vault scoping and revocation rules as alerts, and SHALL NOT count toward the alert cap.

#### Scenario: Inbox changes

- **WHEN** three documents enter the Inbox within a minute for a person with a registered widget token
- **THEN** one widgets push is sent to that token, with only `content-changed: true` in `aps`

### Requirement: Test notification

A signed-in device SHALL be able to ask for a test alert with `POST /api/devices/{installation_id}/test`. The server SHALL respond:

- `202` once it has queued the alert
- `409` when push is unavailable
- `429` when more than one test was asked for in the last minute

#### Scenario: Test from the app

- **WHEN** the app asks for a test on a configured server
- **THEN** the server responds `202` and the device receives an alert that reads "Notifications from Folio are working"

#### Scenario: Test without push

- **WHEN** the transport is `none`
- **THEN** the server responds `409` with a message that push is not configured on this server

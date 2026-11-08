# Spec Delta

## ADDED Requirements

### Requirement: Push notification configuration

The system SHALL accept these keys through `config.toml` and `FOLIO_PUSH__*` environment variables, and SHALL store them in the `settings` table:

- `push.transport`: `none` (the default) or `apns`
- `push.apns.team_id`
- `push.apns.key_id`
- `push.apns.topic`: default `com.cedricziel.folio`
- `push.apns.private_key`: with a `_FILE` variant
- `push.quiet_window_seconds`: default 30
- `push.max_alerts_per_hour`: default 12

Precedence SHALL be built-in defaults, config file, environment, then saved database rows. An empty database secret SHALL fall back to the configured value. Changes SHALL take effect within 5 seconds without a restart.

#### Scenario: Unconfigured server

- **WHEN** Folio starts with no push configuration
- **THEN** the effective transport is `none`, device registration works and no outbound push request is made

#### Scenario: Key from a file

- **WHEN** `FOLIO_PUSH__APNS__PRIVATE_KEY_FILE` points to a `.p8` file and the transport is `apns`
- **THEN** the key is read from the file

#### Scenario: Database wins

- **WHEN** `config.toml` sets `push.max_alerts_per_hour` to 12 and an admin saves 6
- **THEN** the cap is 6 within 5 seconds

### Requirement: Push key storage

A private key saved through `/settings` SHALL be stored encrypted. It SHALL be accepted only when the encryption secret `auth.secret_key` comes from the environment or a file, not from the database. The key SHALL never be shown, logged or returned after saving. A key that is not a PKCS#8 P-256 key SHALL be rejected.

#### Scenario: Secret only in the database

- **WHEN** `auth.secret_key` is unset, so the encryption secret lives in the database, and an admin tries to save a `.p8` key
- **THEN** the save is refused with a message to set `FOLIO_AUTH__SECRET_KEY` or to provide the key as a file, and nothing is stored

#### Scenario: Malformed key

- **WHEN** the submitted key is not a PKCS#8 P-256 key
- **THEN** the save responds with a field error, and the previous key stays in effect

### Requirement: Push test endpoint override

The system SHALL accept `push.apns.base_url` from configuration only, never from the database, so tests can point APNs requests at a local stub. When it is set, the settings page and the startup log SHALL say that pushes go to a non-Apple endpoint.

#### Scenario: Stub endpoint

- **WHEN** `FOLIO_PUSH__APNS__BASE_URL` is `http://127.0.0.1:9443`
- **THEN** pushes go to that address over cleartext HTTP/2, and `/settings` shows a warning naming it

### Requirement: Push notification settings page

The system SHALL show a "Push notifications" card on `/settings` to admins and save it with `POST /settings/push`, responding with a Turbo Stream confirmation. The card SHALL show:

- the effective transport and the source of each value
- whether a key is present
- the last credential error
- the number of registered devices

`POST /settings/push/test` SHALL send a test alert to the admin's own devices.

#### Scenario: Save APNs credentials

- **WHEN** an admin with an externally set `auth.secret_key` saves a team ID, key ID and `.p8` key
- **THEN** the card shows the key as present, and a test from the card reaches the admin's device

#### Scenario: Not an admin

- **WHEN** a non-admin posts to `/settings/push`
- **THEN** the server responds `403` and nothing changes

#### Scenario: Credential error shown

- **WHEN** APNs rejected the provider token on the last send
- **THEN** the card shows the reason and when it happened

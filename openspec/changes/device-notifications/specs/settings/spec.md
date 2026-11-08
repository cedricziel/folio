# Spec Delta

## ADDED Requirements

### Requirement: Push notification configuration

The system SHALL accept these keys through `config.toml` and `FOLIO_PUSH__*` environment variables:

- `push.transport`: `none`, `apns` or `relay`, default `none`
- `push.apns.team_id`, `push.apns.key_id` and `push.apns.topic`, with topic default `com.cedricziel.folio`
- `push.apns.private_key`, with a `_FILE` variant
- `push.relay.url`
- `push.relay.token`, with a `_FILE` variant
- `push.quiet_window_seconds`, default 30
- `push.max_alerts_per_hour`, default 12

The same keys SHALL be stored in the `settings` table. Precedence SHALL be built-in defaults, config file, environment, then saved database rows. A secret that is empty in the database SHALL fall back to the configured value.

The private key and relay token SHALL be stored encrypted in the database, and SHALL never be shown, logged or returned after saving. Changes SHALL take effect within 5 seconds without a restart.

#### Scenario: Unconfigured server

- **WHEN** Folio starts with no push configuration
- **THEN** the effective transport is `none`, device registration works and no outbound push request is made

#### Scenario: Key from a file

- **WHEN** `FOLIO_PUSH__APNS__PRIVATE_KEY_FILE` points to a `.p8` file and the transport is `apns`
- **THEN** the key is read from the file and the effective key ID and team ID come from configuration

#### Scenario: Database wins

- **WHEN** `config.toml` selects `relay` and an admin saves `apns` with a key on `/settings`
- **THEN** pushes use direct APNs within 5 seconds

#### Scenario: Malformed key

- **WHEN** the saved private key is not a valid PKCS#8 P-256 key
- **THEN** the save responds with a field error, the previous key stays in effect, and nothing is sent with the bad key

### Requirement: Push notification settings page

The system SHALL show a "Push notifications" card on `/settings` to admins and save it with `POST /settings/push`, responding with a Turbo Stream confirmation.

The card SHALL show:

- the effective transport and where each value comes from (database or configuration)
- whether a private key or relay token is present, never its value
- the last credential error, if any
- the number of registered devices

`POST /settings/push/test` SHALL send a test alert to the admin's own registered devices.

#### Scenario: Save APNs credentials

- **WHEN** an admin selects `apns`, enters a team ID, key ID and `.p8` key and saves
- **THEN** the card shows the key as present, and a test sent from the card reaches the admin's registered device

#### Scenario: Not an admin

- **WHEN** a non-admin posts to `/settings/push`
- **THEN** the server responds `403` and nothing changes

#### Scenario: Credential error shown

- **WHEN** APNs rejected the provider token on the last send
- **THEN** the card shows the error reason and the time it happened

# Spec Delta

## Purpose

Lets an operator who builds and signs their own Folio Apple app have their server's web links open in that app, by publishing an Apple app site association for the app IDs they configure. The official App Store build cannot use this, because it cannot list each self-hosted domain.

## ADDED Requirements

### Requirement: Opt-in Apple app IDs setting

The server SHALL accept a list of Apple app IDs (`TEAMID.bundle.id`: a 10-character uppercase alphanumeric Team ID, a dot, and a bundle identifier) under `apple.app_ids`. Sources are `config.toml` (`[apple] app_ids`, a TOML array), the env var `FOLIO_APPLE__APP_IDS` (comma-separated), and the `settings` table. The setting is empty by default.

#### Scenario: Configured in config.toml

- **WHEN** `config.toml` sets `[apple] app_ids = ["ABCDE12345.com.example.folio"]`
- **THEN** the server treats that app ID as configured

#### Scenario: Configured through the environment

- **WHEN** `FOLIO_APPLE__APP_IDS=ABCDE12345.com.example.folio,ABCDE12345.com.example.folio.beta` is set
- **THEN** both app IDs are configured and override `config.toml`

#### Scenario: Invalid startup value

- **WHEN** `apple.app_ids` contains `com.example.folio` (no Team ID)
- **THEN** startup fails with an error naming `apple.app_ids` and the invalid entry

### Requirement: Database value wins over startup configuration

A non-empty `apple.app_ids` value in the `settings` table SHALL replace the startup value. An empty or missing database value SHALL fall back to the startup value. Changes SHALL apply to the next association request without a restart.

#### Scenario: Database overrides config

- **WHEN** `config.toml` lists app ID A and the settings table lists app ID B
- **THEN** the association lists only B

#### Scenario: Database cleared

- **WHEN** the settings-table value is saved empty while `config.toml` lists A
- **THEN** the association lists A

### Requirement: Admins edit the app IDs at /settings

`GET /settings` SHALL show an "Apple app links" section to owners and admins with the effective app IDs and the setting's source. `POST /settings/apple` (HTML form) SHALL save one app ID per line. It SHALL redirect to `/settings?saved=apple` on success and re-render the form with 422 naming the first invalid entry.

#### Scenario: Save valid app IDs

- **WHEN** an admin posts two valid app IDs to `POST /settings/apple`
- **THEN** the response is 303 to `/settings?saved=apple` and the settings table holds both

#### Scenario: Save an invalid app ID

- **WHEN** an admin posts `not-an-app-id`
- **THEN** the response is 422, the form shows the error for that entry, and the stored value is unchanged

#### Scenario: Non-admin

- **WHEN** a member who is neither owner nor admin posts to `POST /settings/apple`
- **THEN** the response is 403, as for the other settings sections, and nothing is stored

### Requirement: Server publishes the app site association when configured

When at least one app ID is configured, `GET /.well-known/apple-app-site-association` SHALL return 200 with `Content-Type: application/json`, no redirect, and no authentication required. The body is an `applinks` document whose single `details` entry lists the configured `appIDs` and the path components below.

#### Scenario: Configured server

- **WHEN** `apple.app_ids` holds `ABCDE12345.com.example.folio` and Apple's CDN requests `GET /.well-known/apple-app-site-association` without credentials
- **THEN** the response is 200 JSON with `applinks.details[0].appIDs == ["ABCDE12345.com.example.folio"]`

#### Scenario: Caching headers

- **WHEN** the same path is requested
- **THEN** the response carries `Cache-Control: public, max-age=3600` and no `Set-Cookie`

### Requirement: Association is absent when unconfigured

When no app ID is configured, `GET /.well-known/apple-app-site-association` SHALL return 404 and SHALL NOT redirect to `/login`. The legacy path `GET /apple-app-site-association` SHALL NOT be served.

#### Scenario: Default installation

- **WHEN** a server with no `apple.app_ids` receives `GET /.well-known/apple-app-site-association`
- **THEN** the response is 404

#### Scenario: Root path

- **WHEN** any server receives `GET /apple-app-site-association`
- **THEN** it is handled like any other unknown path and no association is returned

### Requirement: Association matches only handed-over pages

The association's components SHALL claim `/vaults/*/documents/*`, `/vaults/*/saved-views/*` and the vault root `/vaults/*/`, each prefixed with the path of `server.public_url`. Listed before them, as exclusions, are sub-resources and actions: `/vaults/*/documents/*/*`, `/vaults/*/documents/search`, `/vaults/*/documents/upload`, `/vaults/*/saved-views/*/*` and `/vaults/*/saved-views/new`. Login, OAuth, settings, API, static assets and downloads SHALL NOT be claimed.

#### Scenario: Document page claimed

- **WHEN** the device evaluates `/vaults/{vault_id}/documents/{id}` against the association
- **THEN** it matches and opens the app

#### Scenario: Download excluded

- **WHEN** the device evaluates `/vaults/{vault_id}/documents/{id}/download`
- **THEN** an exclusion matches first and the link stays in the browser

#### Scenario: Sign-in pages untouched

- **WHEN** the device evaluates `/login` or `/oauth/authorize`
- **THEN** no component matches and the link stays in the browser

#### Scenario: Server under a path prefix
- **WHEN** `server.public_url` is `https://example.com/folio` and the reverse proxy forwards `/.well-known/apple-app-site-association` to Folio
- **THEN** every component pattern starts with `/folio/vaults/`

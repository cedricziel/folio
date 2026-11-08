# Proposal

## Why

Folio runs on iPhone, iPad and Mac, but nothing moves between them. A document opened on the phone has to be found again by hand on the Mac. The only continuation the app handles today is Spotlight (`InboxView.onContinueUserActivity(CSSearchableItemActionType)`). That path carries no server identity, and it is dropped while App lock is showing, because the lock screen replaces the view that installs the handler. Folio is self-hosted, so each user has their own server domain. Any hand-over has to say which server it came from, and the official App Store build can never claim those domains.

## What Changes

- **App→app Handoff**: the Apple app advertises `NSUserActivity` hand-overs for the document detail (including an open discussion sheet), the vault library (Inbox / All documents) and saved views. The activity types are declared in `NSUserActivityTypes` (XcodeGen `project.yml`). Search, upload/scan review, taxonomy and settings do not advertise.
- **Server identity in the payload**: each hand-over carries a versioned payload with the sender's server base URL and the vault, document or saved-view IDs. Its user info carries no names or content. The receiver compares canonical server URLs and routes in-app only when it is signed in to the same server and the vault is accessible. Defined outcomes for: not signed in, different server, inaccessible vault, deleted or trashed document, malformed payload, and App lock.
- **Continuation waits for App lock**: Handoff, universal-link and Spotlight continuations that arrive while the app is locked are held and delivered after unlock. They never bypass the lock. Nothing is advertised while the app is locked.
- **App→browser fallback**: each activity sets `webpageURL` to the existing vault-scoped web page on the same server (`/vaults/{vault_id}/documents/{id}`, `/vaults/{vault_id}/`, `/vaults/{vault_id}/saved-views/{id}`). Devices without the app open it in the browser. No new web routes are needed; this change pins the existing ones as stable.
- **Browser→app (Universal Links), for self-builders only**: a new opt-in server setting `apple.app_ids` (Team ID + bundle ID entries). When it is set, the server answers `GET /.well-known/apple-app-site-association` with an `applinks` document whose UUID-masked patterns match exactly those three web pages. When it is unset, the route answers 404. On the app side, the identifiers that are literals today (bundle IDs, team, app group, keychain group, signing profiles and OAuth callback scheme) move into one checked-in `Config/Folio.xcconfig`. An optional, git-ignored `Local.xcconfig` overrides them, so other teams can actually build and sign Folio, and its `FOLIO_ASSOCIATED_DOMAINS` adds the associated-domains entitlement. The official build resolves to exactly today's values, and CI checks this. **The official App Store build does not support browser→app**, because associated domains are fixed at build time and every Folio server has a different domain. Universal-link continuations go through the same resolver as Handoff.
- **Privacy**: no document, vault or saved-view names in `userInfo` or keywords. The activity title stays the document title that #392 sets for on-screen awareness. That is acceptable because Handoff delivers it only to the user's own devices on the same Apple Account, and search and public indexing are off. `isEligibleForSearch`, `isEligibleForPublicIndexing` and `isEligibleForPrediction` are false. Activities are invalidated on sign-out and when the vault changes.
- **Testing**: pure payload, server-identity, link-parsing and resolver logic lives in FolioKit with `swift test` coverage, which `fastlane test` already runs in `.github/workflows/apple.yml` on macos-26. Rust integration tests cover the AASA route and the setting.

No breaking changes. No database migration: the setting is a new key in the existing `settings` key-value table.

## Capabilities

### New Capabilities

- `apple-handoff`: continuing the same Folio screen across the user's Apple devices and into a browser. This covers which screens advertise, the payload and its privacy, server-identity matching, routing outcomes, the web fallback URL, and universal-link continuation in the app.
- `universal-links`: the server's opt-in Apple app site association, so that tapping a Folio web link on a device with a self-built Folio app opens the app. This covers the `apple.app_ids` setting, `GET /.well-known/apple-app-site-association` and its path patterns.

### Modified Capabilities

- `app-lock`: adds a requirement that system continuations (Handoff, universal links, Spotlight) arriving while locked are held until unlock and that a locked app advertises nothing.

## Impact

Hexagonal layers touched:

- **folio-core (models)**: a validated `AppleAppId` value type (`TEAMID.bundle.id`). No new port: the setting is read through the existing `SettingsRepository`.
- **folio-infra (adapters/config)**: a new typed startup section `[apple]` with `app_ids`, plus the `FOLIO_APPLE__APP_IDS` env var, validated at startup. No schema change.
- **folio-api (routes/templates)**: a new **JSON route** `GET /.well-known/apple-app-site-association`, unauthenticated and already exempt under `/.well-known/`. It is not part of `/api` and not in `openapi/openapi.json`. A new **HTML (Turbo) form** section on `/settings` saved via `POST /settings/apple`, admin-only like the other settings sections.
- **Setting location**: `apple.app_ids` lives in **both** `config.toml` (`[apple] app_ids`, env `FOLIO_APPLE__APP_IDS`) and the `settings` table (key `apple.app_ids`). A non-empty database value wins; an empty or missing database value falls back to `config.toml`. This matches the existing AI-key and MCP precedence.
- **Apple app**: `apple/Folio/project.yml` (`NSUserActivityTypes`, every identifier read from `$(FOLIO_…)` variables), new `apple/Folio/Config/Folio.xcconfig` + `Local.xcconfig.example`, `apple/scripts/` (associated-domains and build-settings checks), `apple/fastlane/Fastfile` (`generate_project`), FolioKit `ServerConfig` / `OAuthConfiguration` (app group and callback scheme from Info.plist), `apple/Folio/Sources` (`FolioApp`, `RootView`, `WorkspaceView`, `InboxView`, `DocumentDetailView`), and a new `apple/FolioKit/Sources/FolioKit/Handoff/` module with tests in `apple/FolioKit/Tests/FolioKitTests`.
- **Docs**: `apple/README.md` (self-builder Universal Links setup) and the configuration reference for `apple.app_ids`.
- **Unchanged**: the OpenAPI spec and the generated Swift client. Handoff uses existing JSON endpoints (`GET /api/vaults`, document detail, saved views).

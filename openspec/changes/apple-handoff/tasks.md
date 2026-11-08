# Tasks

Server and Apple sections are independent and can land in either order. Within each section, follow the numbering. Each task is one semantic commit, test first.

## Server

## 1. Apple app ID model and startup configuration

- [ ] 1.1 Add `folio_core::models::AppleAppId` with `FromStr` and `parse_list`. First write failing unit tests in folio-core: `ABCDE12345.com.example.folio` is accepted; `com.example.folio`, `abcde12345.x`, `ABCDE1234.x` and an empty string are rejected; comma- and newline-separated lists are split and trimmed. Commit `feat(core): validate Apple app IDs`.
- [ ] 1.2 Add `Config.apple: AppleConfig { app_ids }` in `crates/folio-infra/src/config.rs`, accepting a TOML array or a comma-separated `FOLIO_APPLE__APP_IDS`, validated in `Config::validate` and listed in the config lookup table. First write failing config tests: TOML array parsed; env string parsed and overriding TOML; an invalid entry fails startup with a message naming `apple.app_ids`; default empty. Commit `feat(infra): apple.app_ids startup setting`.

## 2. Apple app site association route

- [ ] 2.1 Create `crates/folio-api/tests/apple_links.rs`, failing first: `GET /.well-known/apple-app-site-association` returns 404 with no app IDs configured, also with auth enabled and no session (no `/login` redirect). Then add `routes/apple.rs` and merge it at the root, plus `AppState.apple_app_ids_default`. Commit `feat(api): serve apple-app-site-association when configured`.
- [ ] 2.2 Failing test: with `apple.app_ids` configured in startup config, the route returns 200, `Content-Type: application/json`, `Cache-Control: public, max-age=3600`, no `Set-Cookie`, and a body equal to the snapshot from design.md §8 (exclusions first). With `server.public_url = https://example.com/folio`, every pattern starts with `/folio/vaults/`. `GET /apple-app-site-association` is not served. Implement the body builder. Commit `feat(api): applinks components for document, vault and saved-view pages`.
- [ ] 2.3 Failing test: a non-empty `apple.app_ids` row in the settings table replaces the config value, and an empty row falls back to config, both without a restart. Implement the lookup through `SettingsRepository`. Commit `feat(api): settings table overrides apple.app_ids`.

## 3. Admin setting at /settings

- [ ] 3.1 Failing tests in `apple_links.rs`: `POST /settings/apple` with two valid IDs returns 303 to `/settings?saved=apple` and stores newline-separated text; an invalid line returns 422 naming it and leaves the stored value unchanged; a non-admin gets 403. Then `GET /settings` shows the "Apple app links" section with the effective IDs, their source and the association URL. Implement the handler and the template section, using only existing design-system tokens. Commit `feat(api): edit Apple app links in settings`.
- [ ] 3.2 Document `apple.app_ids` in `docs/configuration.md` (TOML, env and precedence) and the Universal Links operator steps in `docs/deployment.md` (public HTTPS, proxy rule for a path prefix, `?mode=developer` for private servers). Check that the TOML example in the doc parses in a config test. Commit `docs: Apple app links setting`.

## 4. Stable web URLs

- [ ] 4.1 Add regression tests to `crates/folio-api/tests/web.rs` pinning the handed-over pages: `GET /vaults/{vault_id}/documents/{id}` returns 200 when active and 404 when trashed; `GET /vaults/{vault_id}/` and `?view=all` return 200; `GET /vaults/{vault_id}/saved-views/{id}` returns 200, or 404 for another user's private view; an unauthenticated GET redirects to `/login?next=%2Fvaults%2F…`. Fix any test that fails. Commit `test(api): pin web URLs used by Handoff`.

## Apple

## 5. FolioKit Handoff core (pure, `swift test`)

- [ ] 5.1 Add `apple/FolioKit/Sources/FolioKit/Handoff/ServerIdentity.swift`. First write a failing `ServerIdentityTests` table: case, default port, trailing slash, user info and query are ignored; scheme, non-default port and path prefix are significant. Commit `feat(apple): canonical server identity`.
- [ ] 5.2 Add `HandoffPayload` (targets: document with optional discussion pane, library inbox/all, saved view) and the three activity-type constants. First write a failing `HandoffPayloadTests`: round trip; `requiredUserInfoKeys`; missing key, bad UUID, `v` ≠ "1" and an unknown type all fail to decode; the user info holds only the spec'd keys. Commit `feat(apple): versioned Handoff payload`.
- [ ] 5.3 Add `HandoffLink.webURL(for:)` and `HandoffLink.parse(_:)`. First write a failing `HandoffLinkTests`: web URLs for every target with and without a `/folio` prefix; parsing those URLs back (round trip); `…/documents/{id}/download`, `…/entries`, `/login` and a non-UUID segment are rejected. Commit `feat(apple): web URLs and universal-link parsing`.
- [ ] 5.4 Add `HandoffResolver.decide(incoming:session:)` and `HandoffDecision`. First write a failing `HandoffResolverTests` covering: same server → `.open`; signed out, expired or interrupted → `.signIn(prefill:then:)`; another server → `.otherServer`; malformed payload with a `webpageURL` → `.openInBrowser`; unparseable universal link → `.openInBrowser`; Spotlight → `.open`; `.restoring` → undecided. Commit `feat(apple): Handoff routing resolver`.
- [ ] 5.5 Add `ContinuationInbox` (`@MainActor @Observable`). First write a failing `ContinuationInboxTests`: `take` returns nil while locked or restoring; the last received item wins; `clear()` on sign-out empties it; an item is delivered exactly once. Commit `feat(apple): hold continuations until the app can route them`.
- [ ] 5.6 Add `HandoffActivity.configure(_:payload:)`. First write a failing test: given an `NSUserActivity` and a document payload, the user info matches the codec, `webpageURL` matches `HandoffLink`, the title is "Folio", and search, public-indexing and prediction eligibility are false. Commit `feat(apple): configure Handoff activities privately`.
- [ ] 5.7 Extend `WorkspaceViewModel.open` with a `pane` and add a saved-view route that reports a missing saved view. First write failing `WorkspaceTests`: an open with `.discussion` exposes a pending discussion presentation; an unknown vault keeps the current vault and sets "This vault is no longer available."; a missing saved view falls back to the vault library with an error. Commit `feat(apple): workspace routes for Handoff targets`.

## 6. App wiring

- [ ] 6.1 Declare `NSUserActivityTypes` (`com.cedricziel.folio.document`, `.library`, `.saved-view`) in `apple/Folio/project.yml`. Verify with `xcodegen generate` and `plutil -extract NSUserActivityTypes json` on the generated Info.plist. Commit `build(apple): declare Handoff activity types`.
- [ ] 6.2 Move continuation handling to `RootView`: handle the three Folio types, `NSUserActivityTypeBrowsingWeb` and `CSSearchableItemActionType` into a shared `ContinuationInbox`. Remove the `InboxView` Spotlight handler. Consume the inbox in `WorkspaceView` once `workspace.isReady`, and again when `lock.isLocked` becomes false. Clear it in `AppModel.signOut()`. First write a failing `AppLockContinuationTests` in `FolioAuthTests`: lock → receive → failed unlock leaves the item held → unlock delivers it once. Commit `feat(apple): continuations wait for App lock`.
- [ ] 6.3 Prefill sign-in from `.signIn` decisions: `OnboardingViewModel.address` takes the prefill; `AppModel.connected(to:)` re-decides and drops the route when the server differs. First write a failing `ViewModelTests` case: prefill sets the address, sign-in to the same server yields the route, and sign-in to another server drops it. Commit `feat(apple): continue Handoff after sign-in`.
- [ ] 6.4 Add the other-server alert ("This document is on {host}…", **Open in Browser** / **Cancel**) and the browser presentation (`SFSafariViewController` on iOS, `NSWorkspace` on macOS) for `.otherServer` and `.openInBrowser`. Verify with the `NavigationTests` fixture or a manual check on the iOS simulator using `xcrun simctl openurl`. Commit `feat(apple): browser fallback for foreign continuations`.
- [ ] 6.5 Advertise activities with `.userActivity(_:isActive:)` in `DocumentDetailView` (document, plus the discussion pane while the sheet is shown) and `InboxView` (library or saved view, never search text). Set `isActive` to false while locked. Verify in the `NavigationTests` fixture that the scene's current activity type and payload follow navigation, sign-out and vault switches. Commit `feat(apple): advertise Handoff for documents, libraries and saved views`.

## 7. Self-builder Universal Links

- [ ] 7.1 Add `apple/Folio/Config/Folio.xcconfig` (empty `FOLIO_ASSOCIATED_DOMAINS`, `#include? "Local.xcconfig"`), `Config/Local.xcconfig.example`, `configFiles` in `project.yml`, and git-ignore `Config/Local.xcconfig` and `Resources/Folio-Local.entitlements`. Add a CI step in `.github/workflows/apple.yml` that fails if the generated official `Folio.entitlements` contains `com.apple.developer.associated-domains`. Commit `build(apple): optional local xcconfig for self-builds`.
- [ ] 7.2 Add `apple/scripts/associated-domains.sh`, which writes `Folio-Local.entitlements` with `applinks:` entries from `FOLIO_ASSOCIATED_DOMAINS` and does nothing when it is empty. Call it from fastlane `generate_project` when `Local.xcconfig` exists. Verify by running it with `FOLIO_ASSOCIATED_DOMAINS=folio.example.com` (expect one `applinks:folio.example.com` entry) and empty (expect no file). Commit `build(apple): generate associated domains for self-builds`.
- [ ] 7.3 Document in `apple/README.md`: Handoff requirements (same Apple Account, Bluetooth and Wi-Fi, Handoff enabled); that the official App Store build supports app→app and app→browser but **not** browser→app, and why; and the self-builder checklist (Team ID + bundle ID = server `apple.app_ids`, `Local.xcconfig`, public HTTPS, `?mode=developer`). Commit `docs(apple): Handoff and Universal Links`.

## 8. Integration checks

- [ ] 8.1 Run `make format`, `make lint` and `make test`. All pass.
- [ ] 8.2 Run `cd apple && bundle exec fastlane test` (FolioKit `swift test`, iOS and macOS builds, `FolioAuthTests`). All pass, and CI's `apple.yml` on macos-26 is green.
- [ ] 8.3 Use the `verify` skill against an ephemeral instance with `FOLIO_APPLE__APP_IDS=ABCDE12345.com.example.folio`: `curl -i /.well-known/apple-app-site-association` returns 200 JSON, and without the variable it returns 404. `/settings` shows the section. Update the `verify` skill to cover the route.
- [ ] 8.4 Manual device pass, recorded in the PR: iPhone → Mac document Handoff on the same server; Handoff into a locked Mac opens only after unlock; Handoff to a device on another server offers the browser; a Mac without the app opens the web page.

## Workflow follow-up

- Archive the change (`openspec archive apple-handoff`) as the last commit on the implementation branch, so one merge carries the code and the updated specs.

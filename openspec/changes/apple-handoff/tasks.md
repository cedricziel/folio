# Tasks

Server and Apple sections are independent and can land in either order; within each, follow the numbering. Each task is one semantic commit and starts with the failing test it names.

## Server

## 1. Apple app ID model and startup configuration

- [ ] 1.1 Add `folio_core::models::AppleAppId` with `FromStr` and `parse_list`. First write failing folio-core unit tests:
  - `ABCDE12345.com.example.folio` is accepted
  - `com.example.folio`, `abcde12345.x`, `ABCDE1234.x` and an empty string are rejected
  - comma- and newline-separated lists are split and trimmed

  Commit `feat(core): validate Apple app IDs`.

- [ ] 1.2 Add `Config.apple: AppleConfig { app_ids }` in `crates/folio-infra/src/config.rs` as a TOML array. Register `FOLIO_APPLE__APP_IDS` in the env-setting table with the existing `ValueKind::List` (the comma-list parsing already used for `import_roots` and forward-auth), with no custom deserialiser. Validate the entries in `Config::validate`. First write failing config tests:
  - a TOML array is parsed
  - an env comma list is parsed and overrides TOML
  - an invalid entry fails startup with a message naming `apple.app_ids`
  - the default is empty

  Commit `feat(infra): apple.app_ids startup setting`.

## 2. Apple app site association route

- [ ] 2.1 Create `crates/folio-api/tests/apple_links.rs` with a failing test: `GET /.well-known/apple-app-site-association` returns 404 when no app IDs are configured, also with auth enabled and no session (no `/login` redirect). Then add `routes/apple.rs`, merged at the root, plus `AppState.apple_app_ids_default`. Commit `feat(api): serve apple-app-site-association when configured`.
- [ ] 2.2 Write a failing test for an `apple.app_ids` set in startup config:
  - the route returns 200 with `Content-Type: application/json`, `Cache-Control: public, max-age=3600` and no `Set-Cookie`
  - the body equals the design §8 snapshot: three components with `????????-????-????-????-????????????` masks, no `*` and no `exclude`
  - with `server.public_url = https://example.com/folio`, every pattern starts with `/folio/vaults/`
  - `GET /apple-app-site-association` is not served

  Implement the body builder. Commit `feat(api): applinks components for document, vault and saved-view pages`.

- [ ] 2.3 Write a failing test: a non-empty `apple.app_ids` row in the settings table replaces the config value on the next request without a restart, and an empty row falls back to config. Implement the lookup through `SettingsRepository`. Commit `feat(api): settings table overrides apple.app_ids`.

## 3. Admin setting at /settings

- [ ] 3.1 Write failing tests in `apple_links.rs`:
  - `POST /settings/apple` with two valid IDs returns 303 to `/settings?saved=apple` and stores newline-separated text
  - an invalid line returns 422 naming it and leaves the stored value unchanged
  - a non-admin gets 403
  - `GET /settings` shows the "Apple app links" section with the effective IDs, their source ("from settings", "from config.toml", "not configured"), the association URL, and the help text saying devices pick up changes only when Apple's CDN refreshes

  Implement the handler and the template section with existing design-system tokens only. Commit `feat(api): edit Apple app links in settings`.

- [ ] 3.2 Document `apple.app_ids` in `docs/configuration.md`: TOML, env and precedence, including the limitation that an empty `/settings` value falls back to config, so a config value can only be turned off by removing it. Document the Universal Links operator steps in `docs/deployment.md`: public HTTPS, the proxy rule for a path prefix, the CDN refresh delay, and `?mode=developer` for private servers. Write a config test that parses the doc's TOML example. Commit `docs: Apple app links setting`.

## 4. Stable web URLs

- [ ] 4.1 Add regression tests to `crates/folio-api/tests/web.rs` that pin the handed-over pages:
  - `GET /vaults/{vault_id}/documents/{id}` returns 200 for an active document and 404 for a trashed one
  - `GET /vaults/{vault_id}/` and `?view=all` return 200
  - `GET /vaults/{vault_id}/saved-views/{id}` returns 200, or 404 for another user's private view
  - an unauthenticated GET redirects to `/login?next=%2Fvaults%2F…`

  Fix any test that fails. Commit `test(api): pin web URLs used by Handoff`.

## Apple

## 5. Configurable identifiers for self-builds

- [x] 5.1 Write a failing `AppConfigurationTests` in FolioKit: the app group and OAuth callback scheme are read from Info.plist keys `FolioAppGroup` / `FolioCallbackScheme`, with today's literals as the fallback, and `redirectURI` is `<scheme>:/oauth/callback`. Then make `UserDefaultsServerConfigStore.appGroup` and `OAuthConfiguration` use those lookups. Commit `refactor(apple): read app group and callback scheme from Info.plist`.
- [x] 5.2 Add `apple/Folio/Config/Folio.xcconfig` with the official `FOLIO_*` values and `#include? "Local.xcconfig"` (design §9), plus `Config/Local.xcconfig.example`. Switch every literal in `project.yml` to `$(FOLIO_…)`: bundle IDs of all three targets, team, `CODE_SIGN_ENTITLEMENTS`, Release sign style, identities and profiles, app-group and keychain entitlements, `FolioKeychainGroup` / `FolioAppGroup` / `FolioCallbackScheme`, and `NSExtensionFileProviderDocumentGroup`. Git-ignore `Config/Local.xcconfig` and `Resources/Folio-Local.entitlements`. First add `apple/scripts/check-build-settings.sh`, which fails until this is done. It compares `xcodebuild -showBuildSettings` for Folio, FolioShare and FolioFileProvider:
  - without `Local.xcconfig`: the values equal today's literals (`com.cedricziel.folio*`, team `WPW3QSLZ2F`, the match profiles, `Resources/Folio.entitlements`)
  - with the example copied in: the overridden values, with Release resolving to `CODE_SIGN_STYLE = Automatic`, `CODE_SIGN_IDENTITY = Apple Development` and empty profile specifiers

  Run it in `.github/workflows/apple.yml` (official case only). Commit `build(apple): identifiers from Folio.xcconfig with optional local overrides`.

- [x] 5.3 Add `apple/scripts/associated-domains.sh`, which writes `Folio-Local.entitlements` with `applinks:` entries from `FOLIO_ASSOCIATED_DOMAINS` and does nothing when that is empty. In the private `generate_project` helper in `apple/fastlane/Fastfile` (also called by `lane :test` and therefore CI), call the script only `if File.exist?` the `Local.xcconfig`. Add a CI step that fails if the official generated `Folio.entitlements` contains `com.apple.developer.associated-domains`. Verify by running the script with `FOLIO_ASSOCIATED_DOMAINS=folio.example.com` (expect one `applinks:folio.example.com` entry) and with it empty (expect no file). Commit `build(apple): generate associated domains for self-builds`.

## 6. FolioKit Handoff core (pure, `swift test`)

- [x] 6.1 Add `Handoff/ServerIdentity.swift`. First write a failing `ServerIdentityTests` table:
  - ignored: host case, default port, trailing slash, user info and query
  - IDNA: `https://bücher.example` equals `https://xn--bcher-kva.example`, and also its percent-encoded spelling
  - `http://[::1]:8080/` equals `http://[::1]:8080`, and is not equal to `http://[::1]`
  - significant: scheme, a non-default port and the path prefix

  Commit `feat(apple): canonical server identity`.

- [x] 6.2 Add `HandoffPayload` and the three activity-type constants. The document target reuses #392's `DocumentActivityPayload` keys (`version` Int, `server`, `vault`, `document`) plus `pane`; library adds `list`, saved view adds `savedView`. First write a failing `HandoffPayloadTests`:
  - round trip
  - a #392 document payload decodes, and the reverse
  - `requiredUserInfoKeys`
  - a missing key, bad UUID, non-http(s) server, `version` ≠ 1 or an unknown type each fail to decode
  - user info holds only the spec'd keys

  Commit `feat(apple): versioned Handoff payload`.

- [x] 6.3 Add `HandoffLink.webURL(for:)` and `HandoffLink.parse(_:)`. First write a failing `HandoffLinkTests`:
  - web URLs for every target, with and without a `/folio` prefix
  - parsing them back (round trip)
  - `…/documents/{id}/download`, `…/entries`, `/login` and a non-UUID segment are rejected

  Commit `feat(apple): web URLs and universal-link parsing`.

- [x] 6.4 Add `HandoffResolver.decide(incoming:session:)`. First write a failing `HandoffResolverTests` with one case per row of the design §4 table:
  - `restoring` → hold
  - `signedOut` → signIn, and Spotlight → ignore
  - `signedIn` same → open, other → otherServer
  - `expired` / `interrupted` same → hold, other → otherServer
  - `unavailable` with the same server or `nil` → hold, other → otherServer
  - a malformed payload with `webpageURL`, or an unparseable link → openInBrowser

  Commit `feat(apple): Handoff routing resolver`.

- [ ] 6.5 Add `HandoffPresentation.from(_:currentServer:)`. First write a failing `HandoffPresentationTests`: every decision maps to the spec'd presentation, the other-server alert text names both hosts, and `.hold` / `.ignore` map to nothing. Commit `feat(apple): map Handoff decisions to presentations`.
- [ ] 6.6 Add `ContinuationInbox`. First write a failing `ContinuationInboxTests`:
  - `take` returns nil while locked or on `.hold`, and re-decides after a session change
  - the last item wins
  - an item is delivered to the receiving scene exactly once, and to any scene after `sceneClosed` for the receiving `sceneID`
  - `clear()` empties it
  - a held item whose session becomes `.signedOut` without `clear()` (restore finds no credentials) re-decides to `.signIn(prefill:)` instead of being discarded

  Commit `feat(apple): hold continuations until the app can route them`.

- [ ] 6.7 Add `HandoffAdvertiser.payload(for:)` and `HandoffActivity.configure(_:payload:)`. First write a failing `HandoffAdvertiserTests`:
  - a selected document wins over the library
  - `pane` is `.discussion` exactly when `showingDiscussion` is true
  - nil when locked or not signed in
  - library or saved view when nothing is selected
  - there is no search-text input

  Also write a failing configure test: user info matches the codec, `webpageURL` matches `HandoffLink`, `isEligibleForHandoff` is true, and search, public-indexing and prediction eligibility are false. For documents, configure extends #392's `DocumentActivityTracker.configure`. Commit `feat(apple): decide and configure the advertised activity`.

- [ ] 6.8 Extend `WorkspaceViewModel.open` with a `pane`, and add a saved-view route. First write failing `WorkspaceTests`:
  - `.discussion` exposes a pending discussion presentation
  - an unknown vault keeps the current vault and sets "This vault is no longer available."
  - a missing saved view falls back to the vault library with an error

  Commit `feat(apple): workspace routes for Handoff targets`.

- [ ] 6.9 Add `ReadingPosition.clamp(page:pageCount:)` and the optional `page` key in `HandoffPayload`. First write failing tests:
  - `ReadingPositionTests`: 7 of 12 → 7; 40 of 12 → 12; nil, 0 or −3 → 1; any page with a nil `pageCount` → 1
  - `HandoffPayloadTests` additions: `page` round-trips; a payload without `page` or with a string/float `page` still decodes, with `page == nil`; a #392 payload without `page` still decodes
  - `HandoffAdvertiserTests` addition: the reported page appears in the document payload, and a QuickLook (no page) state omits it

  Commit `feat(apple): carry and clamp the reading position`.

## 7. App wiring

- [ ] 7.1 Declare `NSUserActivityTypes` (`com.cedricziel.folio.document`, `.library`, `.saved-view`) in `project.yml`. Add the failing check first: a step in `apple/scripts/check-build-settings.sh` that runs `plutil -extract NSUserActivityTypes json` on the generated Info.plist and expects all three. Commit `build(apple): declare Handoff activity types`.
- [ ] 7.2 Give `AppModel` the single `ContinuationInbox`. In `RootView`, handle the three Folio types, `NSUserActivityTypeBrowsingWeb` and `CSSearchableItemActionType`, each into `inbox.receive(_:scene:)` with a per-window `@State private var sceneID = UUID()`, plus `.onDisappear { inbox.sceneClosed(sceneID) }`. Add `.handlesExternalEvents(preferring: ["*"], allowing: ["*"])` to the `WindowGroup` content, and remove the `InboxView` Spotlight handler. Take items in the receiving window's `WorkspaceView` when it is ready, on unlock and on session change. Clear the inbox in `AppModel.signOut()`. First write a failing `AppLockContinuationTests` in `FolioAuthTests`: lock → receive → a failed unlock keeps the item held → unlock delivers it once to the receiving scene. Commit `feat(apple): continuations wait for App lock and reach one window`.
- [ ] 7.3 Prefill sign-in from `.prefillSignIn`: `OnboardingViewModel.address` takes the URL, and `AppModel.connected(to:)` re-decides. First write a failing `ViewModelTests` case: the prefill sets the address, sign-in to the same server yields the route, and sign-in to another server drops it. Commit `feat(apple): continue Handoff after sign-in`.
- [ ] 7.4 Render `HandoffPresentation`: the alert with **Open in Browser** / **Cancel**, `SFSafariViewController` on iOS and `NSWorkspace` on macOS. The failing test is `HandoffPresentationTests` from 6.5, extended with a case for the button set per presentation. The view only switches over the value. Commit `feat(apple): browser fallback for foreign continuations`.
- [ ] 7.5 Wire advertising. `DocumentDetailView` keeps one `.userActivity` (#392's `.documentActivity`, or a new one if Handoff lands first), with `isActive = payload != nil` and `pane` from `showingDiscussion`; the discussion sheet gets no modifier. `InboxView` gets one `.userActivity` for library and saved-view types, with `isActive = session.selection == nil && payload != nil`. Both build the payload with `HandoffAdvertiser.payload(for:)`. The failing test is `HandoffAdvertiserTests` from 6.7, extended with a split-view case: a selection plus a library makes the library inactive. Commit `feat(apple): advertise Handoff for documents, libraries and saved views`.

- [ ] 7.6 Add `PDFPreview` (PDFKit `PDFView` on iOS and macOS) for PDF documents in `DocumentDetailView`, keeping `QuickLookPreview` for other types. It reports the 1-based current page on `.PDFViewPageChanged` and opens once at `ReadingPosition.clamp(page:pageCount:)` of the loaded document. First add `Sources/PDFPreview.swift` to the `FolioAuthTests` sources in `project.yml` (as `AppLock.swift` is), then write a failing test there (macOS, which can host `PDFView`): a 3-page fixture opened with page 2 shows page 2; with page 9 it shows page 3; with nil it shows page 1; a page change publishes the new number. Commit `feat(apple): PDF preview keeps the reading position`.

## 8. Self-builder docs

- [ ] 8.1 Write `apple/README.md` "Handoff and Universal Links". Cover:
  - Handoff requirements (same Apple Account, Bluetooth and Wi-Fi, Handoff enabled)
  - that the official App Store build supports app→app and app→browser but **not** browser→app, and why
  - an honest self-builder checklist:
    - copy `Local.xcconfig.example`
    - register your own bundle IDs, app group and Associated Domains capability
    - set `FOLIO_DEVELOPMENT_TEAM.FOLIO_BUNDLE_ID` in the server's `apple.app_ids`
    - run `xcodegen generate && apple/scripts/associated-domains.sh`
    - check with `apple/scripts/check-build-settings.sh`
    - expect public HTTPS and a CDN delay, or use `?mode=developer`

  Verify that the checklist commands run as written against `Local.xcconfig.example`. Commit `docs(apple): Handoff and Universal Links`.

## 9. Integration checks

- [ ] 9.1 Run `make format`, `make lint` and `make test`. All pass.
- [ ] 9.2 Run `cd apple && bundle exec fastlane test` (FolioKit `swift test`, iOS and macOS builds, `FolioAuthTests`) and `apple/scripts/check-build-settings.sh`. All pass, and CI's `apple.yml` on macos-26 is green.
- [ ] 9.3 Use the `verify` skill against an ephemeral instance with `FOLIO_APPLE__APP_IDS=ABCDE12345.com.example.folio`:
  - `curl -i /.well-known/apple-app-site-association` returns 200 with the masked components, and 404 without the variable
  - `/settings` shows the section with the CDN note

  Update the `verify` skill to cover the route.

- [ ] 9.4 Do a manual device pass and record it in the PR:
  - iPhone → Mac document Handoff on the same server
  - Handoff into a Mac with two Folio windows opens in the frontmost one only
  - Handoff into a locked Mac opens only after unlock
  - Handoff to a device on another server offers the browser
  - a Mac without the app opens the web page
  - a PDF read at page N continues at page N, and a non-PDF opens at the start
- [ ] 9.5 Record device and platform coverage in the implementation PR, as #175's last acceptance criterion requires. Add a table of the sender → receiver combinations actually tested: iPhone, iPad and Mac, with OS versions, for document, discussion, library, saved view and page. List every combination left unverified, such as iPad → iPad, Universal Links on iPad, or macOS without Apple Silicon. The PR description contains the table before it is marked ready.

## Workflow follow-up

- Archive the change (`openspec archive apple-handoff`) as the last commit on the implementation branch, so one merge carries the code and the updated specs.

# Folio for Apple platforms

Native SwiftUI app for iOS, iPadOS and macOS.

```
apple/
  FolioKit/   Swift package: generated API client, design tokens, models, view models
  Folio/      App and share extension; the Xcode project is generated from project.yml
```

## Run it

Requirements: Xcode 26.6 (the CI test and release baseline), [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

```sh
make apple          # from the repo root: test FolioKit, generate the project, build for macOS
open apple/Folio/Folio.xcodeproj
```

Pick the `Folio` scheme and run on a simulator or on My Mac. On first launch, enter the address of a running Folio server (for example `http://localhost:3000`). The scanner needs a physical iPhone or iPad.

Use `xcrun swift …` and `xcrun xcodebuild …` rather than bare `swift`, so the toolchain matches the active SDK. Xcode asks you to trust the OpenAPI generator plugin on first build; command-line builds pass `-skipPackagePluginValidation`.

Release jobs use the `macos-26` runner and select `/Applications/Xcode_26.6.app/Contents/Developer` through `DEVELOPER_DIR`. Pull requests also run the `test` job on the `xcode-27` runner with Xcode 27.1 (iOS 27.1 SDK): the build, FolioKit tests and NavigationTests on iPhone and iPhone Duo, so the OS 27 SDK stays buildable before releases switch (#373, #198). The Thumbnail and Classification UI lanes run on 26.6 only. Every Apple job logs the runner OS, Xcode build, Swift compiler and installed SDK versions. The runner OS and SDK versions do not set device requirements: deployment minimums remain iOS 18 and macOS 15. The package uses Swift tools 6.0; the app uses Swift 6 language mode, while File Provider targets use Swift 5 mode.

Both iOS and macOS uploads in [the November 2, 2026 TestFlight run](https://github.com/cedricziel/folio/actions/runs/41461724950) succeeded with Xcode 26.6. Those release logs did not report the Swift compiler and SDK versions separately; new runs do.

## How it fits together

- **API client.** `FolioKit/Sources/FolioAPI/openapi.json` plus `openapi-generator-config.yaml` feed the swift-openapi-generator build plugin. The spec is a symlink to the server's `openapi/openapi.json`. `LiveFolioService` wraps the client behind the `FolioService` protocol, which the view models and tests use. Uploads and downloads go through `URLSession` directly because the generated client cannot set per-part content types.
- **Sign-in.** OAuth 2.1 authorization code with PKCE (S256) through `ASWebAuthenticationSession`. The app discovers the server from `/.well-known/oauth-authorization-server`, registers itself once per server (dynamic client registration, public client, redirect `com.cedricziel.folio:/oauth/callback`), and asks for `documents:read documents:write` on the API resource named by the server. Tokens live in one keychain item in the shared access group, so the share and File Provider extensions use and refresh the same sign-in; a 401 triggers one refresh (the refresh token rotates) and a retry.
- **Design tokens.** `design/tokens.json` is the source. `make apple-tokens` regenerates `FolioTheme.swift` and the `Colors.xcassets` asset catalog; a test fails when the committed output drifts from the tokens.
- **Vaults.** The library sidebar discovers accessible vaults and starts with the server's default. Switching vaults clears searches and navigation, then scopes documents, taxonomies and previews to the selected vault. Upload and scan review sheets have a separate destination picker, so filing into another vault leaves the library where it was. On iOS 26, an upload batch started in the app continues as a continued-processing task when Folio leaves the foreground: the system shows its progress and lets the person stop it. Each file keeps one `Idempotency-Key` across retries, so resuming a stopped or failed batch never stores a file twice. Other platforms and older systems upload in the foreground as before. Viewers can read documents; contributors can upload; document deletion is offered to editors and owners. Servers without vault discovery retain the default library.
- **Share extension** (iOS and macOS). Takes PDFs, images and files from any app, shows destination, title and tag pickers, and uploads through FolioKit. Tags come from the selected destination and are available to editors and owners. Untagged uploads go through a background `URLSession`, so they finish after the sheet closes; tagged uploads run in the foreground because tags can only be attached once the document exists. Each request retains its chosen vault throughout the upload.
- **File Provider extension** (iOS and macOS). Folio appears in Files and Finder with one stable folder per accessible vault. Each vault contains Inbox, All documents, Tags, Correspondents and Types. The tree is independent of the app's selected library. Documents download from their containing vault; creating a file in a writable vault's Inbox uploads there. Viewer folders are read-only, and the provider root accepts no uploads. Rename, move and delete are refused. Change enumeration compares persisted snapshots for each folder and the working set, including document aliases and taxonomy folders. Vault and document alias identifiers include the vault UUID, so vault or taxonomy renames retain their identifiers. Revocation removes the vault tree and its aliases on refresh. Missing snapshots request a full enumeration. When upgrading from the old unscoped tree, Files and Finder must enumerate again; old unqualified identifiers are rejected rather than rebound to another vault. Servers without vault discovery retain the legacy tree. Thumbnails come from the server's document thumbnails. Resolving a single item reuses its folder's listing for up to 30 seconds within the extension process, and concurrent lookups share one request; enumerations always ask the server.
- **Shortcuts and Siri.** App Intents live in `FolioKit/Sources/FolioKit/Intents`, on top of `DocumentCatalog` (`Shortcuts/`), which holds the logic and knows nothing about AppIntents, so a watchOS or visionOS target can reuse it. Intents: Scan a document, Upload to Folio, Search documents, Find documents, Get the latest document, Open a document, Add and Remove a tag, Get a document's file, and on OS 27 Add and Remove a tag on many documents at once (`BulkTagIntents.swift`; the documents arrive as an `EntityCollection` of identifiers and are never fetched). Entities: Document, Tag, Correspondent, Document type and Vault. Upload, Search, Find and Get the latest document take an optional vault and otherwise use the server's default vault; Find falls back to the vault of its tag, correspondent or type. Tag, correspondent and type identifiers carry their vault (`<vault>/<id>`; bare UUIDs from older Shortcuts resolve in the default vault), pickers list them from every vault, and items from different vaults are refused. Tag changes touch only documents that need them, and on OS 26 they register with the system's Undo, which reverts only those documents. On OS 27 documents and vaults report whether the vault is shared (`OwnershipProvidingEntity`, from the `shared` flag of `GET /api/vaults`; someone else's vault always counts as shared), so the system confirms changes to them. `FolioFeatures.vaults` can switch the vault entity off. The Scan intent opens the app's review flow, where you choose a destination without changing the library. They read the server address and sign-in from the App Group and shared keychain, so they work while the app is closed; failures become plain sentences (`ShortcutError`). `Folio/Sources/FolioShortcuts.swift` declares the Siri phrases and pulls the package's intents into the app's metadata. Opening a document in the app records it in Spotlight. Each window publishes its open document as a `com.cedricziel.folio.document` activity. On OS 26 the activity also carries the document's entity identifier, so Siri and Apple Intelligence know which document is on screen. Builds with the Xcode 27 SDK also identify each list row's document (iOS 18.4 and macOS 15.4 and later); the modifier isn't available to the app target with older SDKs, so those builds skip it. The activity holds only server, vault and document identifiers and no title. It stays out of Spotlight, public indexing and Siri suggestions. It is cleared when the document closes or is deleted, including a redone trash, and on sign-out, expiry, a server change or an interactive sign-in. An interrupted session withdraws it until the same server comes back. Document entity identifiers include the vault (the Spotlight link format; UUID-only identifiers resolve in the default vault), so intents reach the right vault even when they run outside the app. `DocumentEntity` is `Transferable`: the original file under its own type and filename, the recognized text when there is any, and a link to the document in the web app. The detail view shares and copies it, and rows drag it out. Transfers download through the signed-in session and reuse the open preview. They keep at most a few recent downloads for ten minutes and delete them on sign-out. Spotlight links retain the document's vault, and legacy UUID-only links open the server's default vault. Keyboard commands (File, View, Find) are in `Commands.swift`; the bindings are listed in `docs/shortcuts.md`.
- **Widgets and the Scan control** (optional, off by default). A WidgetKit extension shows the Inbox count and the latest document from a snapshot the app writes to the App Group. A Control Center control runs the Scan intent. Generate the project with `FOLIO_WIDGETS=YES` to build it in; [docs/apple-widgets.md](../docs/apple-widgets.md) lists the portal steps.
- **Shared state.** The app and extensions share the server address through the `group.com.cedricziel.folio` App Group and the bearer token through a keychain access group. Both need a signing team with that group registered before they work on a device.

## App lock

On Mac, choose **Folio > Settings** or press Command-comma. On iPhone and iPad,
open the Privacy menu in the workspace toolbar and choose **App lock settings**.
App lock is off by default and applies to this installation. Enabling or disabling
it requires system authentication. Face ID or Touch ID is used when available,
with the device passcode or Mac password as fallback.

Choose **Immediately**, **After 1 minute** (the default), or **After 5 minutes**
for the time away before another unlock is required. Folio covers its windows
while inactive, even within that grace period. Launching Folio, a device-lock
notification, or **Lock now** requires authentication again. On Mac and iPad,
Command-Shift-L locks Folio. Cancelling authentication leaves the app locked.

App lock protects the Folio interface. Files, Finder, Shortcuts, Spotlight
results, and the share extension keep their existing access. It does not encrypt
cached files or change server permissions. App lock preferences stay outside the
shared App Group and do not alter the shared sign-in.

## Handoff and Universal Links

Folio hands the screen you are on to your other Apple devices: an open document (with its discussion, and the page for PDFs), Inbox or All documents of a vault, or a saved view. Handoff needs both devices signed in to the same Apple Account, with Bluetooth and Wi-Fi on and Handoff enabled (iOS: Settings > General > AirPlay & Continuity; macOS: System Settings > General > AirDrop & Handoff). The receiving app opens the target only when it is signed in to the same server. Otherwise it offers the server's web page. A device without Folio gets the web page directly, so a Mac without the app opens the document in its browser.

There are three directions:

- **App → app** (Handoff): works in every build, including the App Store build.
- **App → browser**: every activity carries the web page on the sender's server, so it works in every build.
- **Browser → app** (Universal Links): only in builds you sign yourself. The domains a link may open are compiled into the app's signed entitlements, and every Folio server has its own domain, so the App Store build cannot list them.

### Universal Links in a self-build

All identifiers come from `Folio/Config/Folio.xcconfig`, which holds the official values and includes an optional, git-ignored `Folio/Config/Local.xcconfig`. To build under your own team:

1. Copy the example and edit it: `cp apple/Folio/Config/Local.xcconfig.example apple/Folio/Config/Local.xcconfig`. Use plain values, not `$(...)` references. Set your Team ID, a bundle ID you own and an App Group. Set `FOLIO_ASSOCIATED_DOMAINS` to your server's host (several hosts separated by spaces). Keep `FOLIO_APP_ENTITLEMENTS = Resources/Folio-Local.entitlements`, or drop both lines if you only want Handoff.
2. In the Apple Developer portal, register `FOLIO_BUNDLE_ID` and its `.share` and `.fileprovider` extensions, create the App Group, assign it to all three, and enable Associated Domains on the app's identifier.
3. On the server, set `apple.app_ids` to `FOLIO_DEVELOPMENT_TEAM.FOLIO_BUNDLE_ID` (for example `ABCDE12345.com.example.folio`) in `config.toml`, `FOLIO_APPLE__APP_IDS` or the Apple app links section of `/settings`. The server then answers `/.well-known/apple-app-site-association`; see `docs/deployment.md`.
4. Generate the project and the entitlements: `cd apple/Folio && xcodegen generate && ../scripts/associated-domains.sh`. `bundle exec fastlane test` does the same when a `Local.xcconfig` exists.
5. Check the resolved identifiers: `apple/scripts/check-build-settings.sh --local`.

Apple's CDN fetches the association file from your server, so the server must be reachable over public HTTPS, and a change can take a while to reach devices. For a server on a private network, use a development-signed build with `FOLIO_ASSOCIATED_DOMAINS = folio.lan?mode=developer` and turn on Associated Domains Development in the device's developer settings; the device then fetches the file directly. Handoff and the web fallback work without any of this.

## Tests

```sh
cd apple/FolioKit && xcrun swift test
```

## Fastlane and TestFlight

```sh
cd apple
bundle install
bundle exec fastlane bootstrap   # once: bundle ids with App Groups, then match appstore (not readonly)
bundle exec fastlane test        # FolioKit tests, then builds for iOS Simulator and macOS
bundle exec fastlane beta        # match -> build number -> build -> TestFlight (iOS)
bundle exec fastlane beta_mac    # same for macOS
```

Pull requests that touch `apple/`, `openapi/`, `design/tokens.json` or the Apple workflow run the test lane. The release workflow calls `beta` and `beta_mac` only when release-please creates a release. You can also dispatch the Apple workflow manually, with an optional app version. The macOS upload is allowed to fail until the Mac installer certificate exists. The build number is the latest TestFlight build plus one.

The lanes need `ASC_KEY_ID`, `ASC_ISSUER_ID` and `ASC_KEY_CONTENT` (the raw `.p8`, or its base64) for the App Store Connect API key, and `MATCH_PASSWORD`. Match clones `git@github.com:cedricziel/certificates.git` over ssh (`MATCH_GIT_URL` overrides it). CI additionally uses `MATCH_DEPLOY_KEY` and `MATCH_KEYCHAIN_PASSWORD`. The team id is in `fastlane/Appfile`.

`beta` registers missing bundle ids (`com.cedricziel.folio`, `.share`, `.fileprovider`) when `CI` is not set. An API key cannot create the App Group or the App Store Connect app record; `bootstrap` prints what to create by hand. Fastlane runs `xcrun xcodebuild` under the active Xcode, so select the right one with `xcode-select` or `DEVELOPER_DIR`; per the machine rules, never bare `swift`.

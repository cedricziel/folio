# Folio for Apple platforms

Native SwiftUI app for iOS, iPadOS and macOS.

```
apple/
  FolioKit/   Swift package: generated API client, design tokens, models, view models
  Folio/      App and share extension; the Xcode project is generated from project.yml
```

## Run it

Requirements: Xcode 16 or newer (built with 27), [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

```sh
make apple          # from the repo root: test FolioKit, generate the project, build for macOS
open apple/Folio/Folio.xcodeproj
```

Pick the `Folio` scheme and run on a simulator or on My Mac. On first launch, enter the address of a running Folio server (for example `http://localhost:3000`). The scanner needs a physical iPhone or iPad.

Use `xcrun swift …` and `xcrun xcodebuild …` rather than bare `swift`, so the toolchain matches the active SDK. Xcode asks you to trust the OpenAPI generator plugin on first build; command-line builds pass `-skipPackagePluginValidation`.

## How it fits together

- **API client.** `FolioKit/Sources/FolioAPI/openapi.json` plus `openapi-generator-config.yaml` feed the swift-openapi-generator build plugin. The spec is a symlink to the server's `openapi/openapi.json`. `LiveFolioService` wraps the client behind the `FolioService` protocol, which the view models and tests use. Uploads and downloads go through `URLSession` directly because the generated client cannot set per-part content types.
- **Sign-in.** OAuth 2.1 authorization code with PKCE (S256) through `ASWebAuthenticationSession`. The app discovers the server from `/.well-known/oauth-authorization-server`, registers itself once per server (dynamic client registration, public client, redirect `com.cedricziel.folio:/oauth/callback`), and asks for `documents:read documents:write` on the API resource named by the server. Tokens live in one keychain item in the shared access group, so the share and File Provider extensions use and refresh the same sign-in; a 401 triggers one refresh (the refresh token rotates) and a retry.
- **Design tokens.** `design/tokens.json` is the source. `make apple-tokens` regenerates `FolioTheme.swift` and the `Colors.xcassets` asset catalog; a test fails when the committed output drifts from the tokens.
- **Vaults.** The library sidebar discovers accessible vaults and starts with the server's default. Switching vaults clears searches and navigation, then scopes documents, taxonomies and previews to the selected vault. Upload and scan review sheets have a separate destination picker, so filing into another vault leaves the library where it was. Viewers can read documents; contributors can upload; document deletion is offered to editors and owners. Servers without vault discovery retain the default library.
- **Share extension** (iOS and macOS). Takes PDFs, images and files from any app, shows destination, title and tag pickers, and uploads through FolioKit. Tags come from the selected destination and are available to editors and owners. Untagged uploads go through a background `URLSession`, so they finish after the sheet closes; tagged uploads run in the foreground because tags can only be attached once the document exists. Each request retains its chosen vault throughout the upload.
- **File Provider extension** (iOS and macOS). Folio appears in Files and Finder with one stable folder per accessible vault. Each vault contains Inbox, All documents, Tags, Correspondents and Types. The tree is independent of the app's selected library. Documents download from their containing vault; creating a file in a writable vault's Inbox uploads there. Viewer folders are read-only, and the provider root accepts no uploads. Rename, move and delete are refused. Change enumeration compares persisted snapshots for each folder and the working set, including document aliases and taxonomy folders. Vault and document alias identifiers include the vault UUID, so vault or taxonomy renames retain their identifiers. Revocation removes the vault tree and its aliases on refresh. Missing snapshots request a full enumeration. When upgrading from the old unscoped tree, Files and Finder must enumerate again; old unqualified identifiers are rejected rather than rebound to another vault. Servers without vault discovery retain the legacy tree. No thumbnails: the API has no thumbnail endpoint.
- **Shortcuts and Siri.** App Intents live in `FolioKit/Sources/FolioKit/Intents`, on top of `DocumentCatalog` (`Shortcuts/`), which holds the logic and knows nothing about AppIntents, so a watchOS or visionOS target can reuse it. Intents: Scan a document, Upload to Folio, Search documents, Find documents, Get the latest document, Open a document, Add and Remove a tag, Get a document's file. Entities: Document, Tag, Correspondent, Document type, and an inactive Vault stub. Service-backed intents continue to use the server's default vault. The Scan intent opens the app's review flow, where you choose a destination without changing the library. The vault feature flag remains disabled until service-backed intents support explicit vault selection. They read the server address and sign-in from the App Group and shared keychain, so they work while the app is closed; failures become plain sentences (`ShortcutError`). `Folio/Sources/FolioShortcuts.swift` declares the Siri phrases and pulls the package's intents into the app's metadata. Opening a document in the app records it in Spotlight. Spotlight links retain the document's vault, and legacy UUID-only links open the server's default vault. Keyboard commands (File, View, Find) are in `Commands.swift`; the bindings are listed in `docs/shortcuts.md`.
- **Shared state.** The app and extensions share the server address through the `group.com.cedricziel.folio` App Group and the bearer token through a keychain access group. Both need a signing team with that group registered before they work on a device.

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

Every push to `main` that touches `apple/`, `openapi/` or `design/tokens.json` runs `beta` and `beta_mac` in CI (`beta_mac` is allowed to fail until the Mac installer certificate exists). The build number is the latest TestFlight build plus one.

The lanes need `ASC_KEY_ID`, `ASC_ISSUER_ID` and `ASC_KEY_CONTENT` (the raw `.p8`, or its base64) for the App Store Connect API key, and `MATCH_PASSWORD`. Match clones `git@github.com:cedricziel/certificates.git` over ssh (`MATCH_GIT_URL` overrides it). CI additionally uses `MATCH_DEPLOY_KEY` and `MATCH_KEYCHAIN_PASSWORD`. The team id is in `fastlane/Appfile`.

`beta` registers missing bundle ids (`com.cedricziel.folio`, `.share`, `.fileprovider`) when `CI` is not set. An API key cannot create the App Group or the App Store Connect app record; `bootstrap` prints what to create by hand. Fastlane runs `xcrun xcodebuild` under the active Xcode, so select the right one with `xcode-select` or `DEVELOPER_DIR`; per the machine rules, never bare `swift`.

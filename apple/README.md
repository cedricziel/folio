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
- **Auth.** The API has none yet. `BearerTokenProvider` is wired into both the generated client and the raw requests, so a Keychain-backed provider can be added without touching callers.
- **Design tokens.** `design/tokens.json` is the source. `make apple-tokens` regenerates `FolioTheme.swift` and the `Colors.xcassets` asset catalog; a test fails when the committed output drifts from the tokens.
- **Share extension.** Reads the server address from the `group.com.cedricziel.folio` App Group, so both targets need a signing team with that group before it works on a device.

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

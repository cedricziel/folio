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

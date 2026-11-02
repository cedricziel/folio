---
name: verify
description: Verify Folio Apple changes with isolated package and macOS authentication and File Provider regression tests.
---

# Verify Apple changes

From the repository root, run `make apple-project`.
Create `verify_dir` with `mktemp -d /private/tmp/folio-verify.XXXXXX`.
Remove that directory after verification, including on failure.

Run the package suite for document status mapping, view models, and API adapters:

```sh
CLANG_MODULE_CACHE_PATH="$verify_dir/clang" xcrun swift test \
  --package-path apple/FolioKit --scratch-path "$verify_dir/package" \
  --cache-path "$verify_dir/cache" --config-path "$verify_dir/config" \
  --security-path "$verify_dir/security" --disable-keychain --disable-netrc
```

The API adapter tests use an ephemeral URLSession with a local protocol stub.

Build the app and run the macOS authentication and File Provider tests:

```sh
xcrun xcodebuild test -project apple/Folio/Folio.xcodeproj -scheme Folio \
  -destination 'platform=macOS' -derivedDataPath "$verify_dir/xcode" \
  -clonedSourcePackagesDirPath "$verify_dir/xcode-packages" \
  -packageCachePath "$verify_dir/xcode-cache" \
  -skipPackagePluginValidation -skipMacroValidation \
  -only-testing:FolioAuthTests -only-testing:FolioProviderTests CODE_SIGNING_ALLOWED=NO
```

The tests exercise authentication completion from a background queue without
launching the app, opening a browser, or accessing credentials.
Do not launch the app against existing App Group or keychain state.

## Apple File Provider

Run `xcrun swift test --package-path apple/FolioKit --scratch-path /tmp/folio-provider-build`.
`LiveFolioServiceTests` covers retained download contents and concurrent downloads into a supplied staging directory. Use a task-specific scratch path when another worker is testing.

Generate the project with `xcodegen generate --spec apple/Folio/project.yml`. Build the extension through the app scheme:

```sh
xcrun xcodebuild build -project apple/Folio/Folio.xcodeproj -scheme Folio \
  -destination 'generic/platform=iOS Simulator' -derivedDataPath /tmp/folio-provider-xcode \
  -clonedSourcePackagesDirPath /tmp/folio-provider-xcode-packages \
  -skipPackagePluginValidation -skipMacroValidation CODE_SIGNING_ALLOWED=NO
```

Lint changed Swift files with `xcrun swift-format lint --strict`. A package test or unsigned build does not prove system materialization. For Files or Finder checks, use a new simulator or isolated test account, a separate test domain, and a throwaway server. Verify download bytes and reopening, then remove the domain, simulator, and server. Never register a test domain in the user's signed-in environment.
`FolioProviderTests` exercises upload completion using a fake service and temporary files. It does not register a File Provider domain or access shared credentials. `FolioCatalogTests` checks that Inbox retains documents while processing is pending.

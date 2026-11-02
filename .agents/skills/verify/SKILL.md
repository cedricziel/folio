---
name: verify
description: Verify Folio Apple changes with isolated package and macOS authentication regression tests.
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

Build the app and run the macOS authentication tests:

```sh
xcrun xcodebuild test -project apple/Folio/Folio.xcodeproj -scheme Folio \
  -destination 'platform=macOS' -derivedDataPath "$verify_dir/xcode" \
  -clonedSourcePackagesDirPath "$verify_dir/xcode-packages" \
  -packageCachePath "$verify_dir/xcode-cache" \
  -skipPackagePluginValidation -skipMacroValidation \
  -only-testing:FolioAuthTests CODE_SIGNING_ALLOWED=NO
```

The tests exercise authentication completion from a background queue without
launching the app, opening a browser, or accessing credentials.
Do not launch the app against existing App Group or keychain state.

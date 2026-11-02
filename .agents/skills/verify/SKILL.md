---
name: verify
description: Verify Folio MCP and Apple changes with isolated protocol, package, macOS authentication, and File Provider regression tests.
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
`WorkspaceTests` covers first-run Inbox selection, independent destination queries and navigation paths, and external document links opening the unfiltered library. It uses a mock service and does not read account configuration.
`DocumentDetailViewModelTests` covers opening the next document from a cancelled
navigation task, retaining a downloaded preview on reappearance, and retrying a
failed download without restarting the app. For device confirmation, open one
file, return to the list, open a different file, and reopen the first.

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

`EnumerationTests` checks folder deletions, working-set aliases after restart, classification changes, taxonomy renames, unchanged snapshots, expired anchors, and a consistent initial snapshot. It stores fingerprints under a fresh temporary directory and removes it after each test. Production retains at most 32 snapshots per domain/container cache directory.

`DownloadTests` passes the fixture PDF through `fetchContents`, opens it with PDFKit, reopens it, and verifies that missing server contents produce `noSuchItem`. `ProviderErrorTests` covers supported error domains. For device-only failures, filter Console for subsystem `com.cedricziel.folio.fileprovider` and category `transfer`; reproduce one open and inspect the phase, HTTP status, and mapped error code. These diagnostics exclude filenames, credentials, and response bodies. Callback tests do not establish that a reported device failure is fixed.

## Verify MCP changes

Run `RUSTC_WRAPPER= cargo test -p folio-mcp` for protocol, tool, HTTP, and OAuth
regressions. The skills suite uses in-memory transports and temporary fixture
storage. HTTP tests bind loopback port 0, use test identities, and stop with their
Tokio runtime. They do not start the app or access user archives or credentials.

The skills tests cover modern discovery, direct lookup, manifest digests and
byte sizes, frontmatter equality, unknown URIs, invalid cursors, legacy resource
reads, and document-scope separation. Do not launch a server against existing data.

Before a PR, also run `RUSTC_WRAPPER= make format`, `RUSTC_WRAPPER= make lint`, and
`RUSTC_WRAPPER= make test`. An empty wrapper bypasses a local sccache service when
the sandbox cannot access it.

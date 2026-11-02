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

The skills tests cover discovery, direct lookup, manifest digests and
byte sizes, frontmatter equality, unknown URIs, invalid cursors, standard resource
reads, and document-scope separation. OAuth tests require the resource-specific
metadata URL `/.well-known/oauth-protected-resource/mcp`.
Do not launch a server against existing data.

## Verify configuration, storage, and provisioning

Run `RUSTC_WRAPPER= cargo test -p folio-infra config::tests --lib` for canonical
environment names, secret files, and ignored obsolete configuration names.
The `storage_contract` integration suite covers canonical object keys and prefix
deletion. API `routes` tests cover canonical previews and ignore old preview keys.
The `vault_isolation` suite checks that provisioning does not claim ownerless
archives and that retained SQL migrations still preserve existing rows.

Before a PR, also run `RUSTC_WRAPPER= make format`, `RUSTC_WRAPPER= make lint`, and
`RUSTC_WRAPPER= make test`. An empty wrapper bypasses a local sccache service when
the sandbox cannot access it.

## Verify document repository changes

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test repository_contract`
with Docker available. The shared contract uses disposable SQLite databases and
PostgreSQL containers; it covers bounded pagination, stable ordering, cursor
binding, and batched tags with duplicate IDs and vault isolation. Containers are
removed after the tests. Do not point these tests at an existing archive.

For paged HTTP browsing, run `cargo test -p folio-api --test vaults browse`.
The route tests cover bounded continuations, filter/tag enrichment, malformed
queries, legacy list compatibility, invisible vaults, and membership revocation
between pages. Search coverage verifies title/content matches outside the first
unfiltered page, AND semantics for tags, and rejection of cursors after text
changes. The shared route harness owns a temporary SQLite directory and
removes it when dropped. Regenerate OpenAPI with `make openapi`; the Apple schema
input is a symlink to that file, so run the isolated Swift package suite above
after schema changes.

When temporary build storage is constrained, run Rust and Apple verification
sequentially. Use `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` consistently
for the Rust run and a task-specific `CARGO_TARGET_DIR`; remove that target after
verification before starting the Apple build. These settings omit debug symbols,
not tests.

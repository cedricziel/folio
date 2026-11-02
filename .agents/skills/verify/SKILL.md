---
name: verify
description: Verify Folio MCP and Apple changes with isolated protocol, package, macOS authentication, and File Provider regression tests.
---

# Verify Apple changes

From the repository root, run `make apple-project`.
Create `verify_dir` with `mktemp -d /private/tmp/folio-verify.XXXXXX`.
Remove that directory after verification, including on failure.
On a host with limited disk space, run suites sequentially and remove each
completed suite's scratch build before starting the next. Keep logs until all
checks finish.

Run the package suite for document status mapping, view models, and API adapters:

```sh
CLANG_MODULE_CACHE_PATH="$verify_dir/clang" xcrun swift test \
  --package-path apple/FolioKit --scratch-path "$verify_dir/package" \
  --cache-path "$verify_dir/cache" --config-path "$verify_dir/config" \
  --security-path "$verify_dir/security" --disable-keychain --disable-netrc
```

The API adapter tests use an ephemeral URLSession with a local protocol stub.
`DocumentImportTests` covers mixed PDF and image imports, unsupported files, directories, missing files, and remote URL rejection using temporary files. File-type resolution requires access to macOS type services. For a manual drop check, use an isolated account or simulator and a throwaway server, then drop multiple PDFs and images into the document list, including when it is empty. Check the hover indicator, upload progress, and refreshed list.
`e2e/tests/documents.spec.ts` covers dropping two files onto the web page, the hover indicator, and completion of both uploads. Run it only against the disposable instance started by the E2E setup.
`WorkspaceTests` covers first-run Inbox selection, independent destination queries and navigation paths, and external document links opening the unfiltered library. It uses a mock service and does not read account configuration.
`PagingTests` covers continuation deduplication, selection retention, Inbox lookahead,
cancellation, stale queries, retry, vault consistency, and direct document lookup.
`LiveFolioServiceTests` verifies scoped browse queries and direct document requests.
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

## Apple session recovery

The package suite covers Keychain read, write, and delete failures through injected
operations; it does not access the user’s Keychain. `SessionRestorationTests`
checks temporary failures, retained server addresses, expiry, and interrupted
refreshes. `OAuthRecoveryTests` checks concurrent sessions, failed token persistence,
stale writes after sign-out or another sign-in, offline retry, and lost responses.
`OAuthRefreshLockTests` uses temporary files to verify exclusion, cancellation,
and lock release after failure.

On an isolated macOS test account, check startup with unavailable credentials,
retry after access recovers, and reauthentication after expiry. Confirm that
activation retries restoration without returning to the server selector. Package
tests do not establish the visual behavior of these recovery screens.

## Apple App lock

`FolioAuthTests/AppLockTests` uses unique temporary UserDefaults suites, fake
authentication, and an injected clock. It covers persisted launch locking,
setting authentication, timeout boundaries, inactive privacy state, and stale
authentication results after backgrounding or device lock. It never invokes
LocalAuthentication or accesses the user's app preferences.

Before release, use an isolated simulator or test account to check the Privacy
menu, Lock now, every timeout, multiple windows, and inactive snapshots with a
preview or sheet open. Check Face ID or Touch ID and credential fallback on a
test device. Cancel a prompt, lock the device during a prompt, and return after
both short and long absences. macOS screen lock also uses the distributed
`com.apple.screenIsLocked` notification, so verify it on each supported macOS
release. These device and visual checks are not covered by the hostless tests.

## Apple inbox navigation

`apple/NavigationTests/project.yml` builds the production workspace, inbox, and
detail views with a fake service. Its UI test taps a document, returns to the
list, and opens another document. Model tests alone do not verify row taps.

Generate the fixture outside the checkout with
`xcodegen generate --spec apple/NavigationTests/project.yml --project "$verify_dir"`.
Create a fresh iPhone simulator with an installed iOS runtime and save its ID as
`verify_device`. Run:

```sh
xcrun xcodebuild test -project "$verify_dir/InboxNavigation.xcodeproj" -scheme Fixture \
  -destination "platform=iOS Simulator,id=$verify_device" \
  -derivedDataPath "$verify_dir/build" -clonedSourcePackagesDirPath "$verify_dir/packages" \
  -packageCachePath "$verify_dir/cache" -skipPackagePluginValidation -skipMacroValidation \
  -parallel-testing-enabled NO -test-timeouts-enabled YES \
  -maximum-test-execution-time-allowance 45 CODE_SIGNING_ALLOWED=NO ONLY_ACTIVE_ARCH=YES
```

Bound runner startup separately; XCTest's execution timeout does not cover it.
On every exit, shut down and delete only `verify_device`, then remove `verify_dir`.
An accessibility initialization failure before the test taps a row is an
infrastructure failure, not a navigation regression or a passing check.

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

## Verify vault selection

Run `cargo test -p folio-api --test vaults` for discovery, explicit request scope,
role enforcement, revoked membership, scoped redirects, and CSRF protection.
Run the isolated Playwright fixture with `e2e/tests/vaults.spec.ts` for desktop
and mobile navigation, independent upload destinations, viewer controls, and
late search responses after switching vaults. Use the fixture's temporary
database and server, never an existing archive.

The Apple package suite covers immutable scoped services, independent upload
destinations, and external document links. File Provider tests must cover stable
vault-qualified identifiers, scoped downloads and Inbox uploads, read-only
capabilities, membership revocation, and transient failures that retain entries.
Verify that vault discovery 404 remains an error, unloaded catalogs grant no
write capabilities, and UUID-only Spotlight and unscoped provider identifiers
are rejected. Keep current default-vault document Intent coverage separate from
external identifier parsing.
Run both the package suite and native provider tests above after these changes.
These tests do not establish system integration in Files or Finder.

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

## Verify document trash

Run `cargo test -p folio-api --test trash --test vaults --test live` for default
soft deletion, retained originals, restore, explicit purge, permissions, and
vault-scoped events. Run `cargo test -p folio-infra --lib queue::worker::tests`
for cleanup retries, indefinite retention, and processing races. The repository
contract above also verifies transactional purge enqueueing on both databases.

Build `folio-app`, then run `cd e2e && npx playwright test trash.spec.ts documents.spec.ts shortcuts.spec.ts`.
The setup script starts an instance in a temporary directory with its own port,
database, storage, and test configuration; teardown removes it. Do not set
`E2E_BASE_URL` to an existing user instance. Verify that Move to Trash preserves
the document, Restore returns it, and Delete forever requires confirmation and
eventually removes its pending row. No automatic retention expiry is supported.
Include `vaults.spec.ts` and the API `vaults` suite when library selection changes:
Trash links, forms, event refreshes, and redirects must retain the selected vault,
and viewer/contributor restrictions must also apply to trashed documents.

For Linux visual checks on macOS, a disposable pinned Playwright browser-server
container can connect to the temporary macOS app through
`PW_TEST_CONNECT_EXPOSE_NETWORK=<loopback>`. Verify the remote browser reports
Linux, then use a temporary harness that removes only the host-platform skip in
`visual.spec.ts`. Keep the same tour fixtures, viewport, masks, and screenshot
paths. Recheck without updating snapshots, and remove the container and harness.

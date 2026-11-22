# Apple regression checks


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
`ProcessingSummaryTests` checks stage labels, empty OCR completion, skipped work,
and the distinction between a scheduled retry and terminal failure.
`LiveFolioServiceTests` checks the generated API mapping, and
`DocumentDetailViewModelTests` checks that list updates and metadata saves retain
the stage summaries already shown on the detail screen.
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

## Apple OAuth scope negotiation

Run the package suite with `--filter OAuthRequestTests` using the isolated
package settings above. The requests include `vaults:manage` only when discovery
advertises it. Metadata without that scope keeps registration and authorization
within document scopes, including authorization with an existing client ID.
These tests use constructed requests and do not open a browser or access Keychain.

## Apple session recovery

The package suite covers Keychain read, write, and delete failures through injected
operations; it does not access the user’s Keychain. `SessionRestorationTests`
checks temporary failures, retained server addresses, expiry, and interrupted
refreshes. `OAuthRecoveryTests` checks concurrent sessions, failed token persistence,
stale writes after sign-out or another sign-in, offline retry, and lost responses.
`OAuthRefreshLockTests` uses temporary files to verify exclusion, cancellation,
and lock release after failure.
`OAuthTransientRefreshTests` checks credential recovery after transient token
errors, definitive refusals, and multiple callers sharing a stalled refresh.

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

Allow three minutes per test for cold simulator launch and automation setup.
Keep the per-element waits bounded independently.

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
  -default-test-execution-time-allowance 180 -maximum-test-execution-time-allowance 180 \
  CODE_SIGNING_ALLOWED=NO ONLY_ACTIVE_ARCH=YES
```

Bound runner startup separately; XCTest's execution timeout does not cover it.
On every exit, shut down and delete only `verify_device`, then remove `verify_dir`.
An accessibility initialization failure before the test taps a row is an
infrastructure failure, not a navigation regression or a passing check.

## Apple review and recovery

The package suite includes `UploadViewModelTests` for partial-batch retries,
original-vault retention, and independent batches. `DocumentTrashHistoryTests`
checks native Undo/Redo, failed deletion and restoration, and ordered asynchronous
operations. `TokenGeneratorTests`, `TokenContrastTests`, and `TokenDriftTests`
check high-contrast asset generation, foreground/background contrast, and generated
asset drift.

The isolated navigation fixture also covers scan dismissal confirmation and
retaining the list beside a preview in a wide iPad window. Run the compact
navigation test on an iPhone and the wide-window test on an iPad. A test-runner
launch failure before a test starts does not verify these behaviors.

For layout investigation, launch the fixture with `--onboarding-layout` or
`--share-layout`. These modes use a short viewport, the largest Dynamic Type size,
and mock data. Confirm that all controls are reachable before changing production
layout. The fixture does not establish keyboard avoidance or actual share-extension
hosting behavior; check those separately in an isolated simulator.

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

`DownloadTests` passes the fixture PDF through `fetchContents`, opens it with PDFKit, reopens it, and verifies that missing server contents produce `noSuchItem`. It also cancels a stalled transfer through the returned `Progress`, checks that the request stops and Files receives `NSUserCancelledError`, then opens a fresh copy. `ProviderErrorTests` covers supported error domains. It checks that certificate failures use the File Provider connection code and preserve the cause. For device-only failures, filter Console for subsystem `com.cedricziel.folio.fileprovider` and category `transfer`; reproduce one open and inspect the phase, HTTP status, and mapped error code. These diagnostics exclude filenames, credentials, and response bodies. Callback tests do not establish that a reported device failure is fixed.

## Verify Apple saved views

Run the isolated Swift package suite above for saved-view query mapping, ownership
fields, unchanged-query omission on rename, unsupported versions, paging resets,
cross-vault rejection, and late responses after switching vaults. Run the macOS
build/authentication/provider checks for the production views.
The navigation fixture's `testSavedViewCanBeSavedEditedAndDeleted` saves the
current view, reopens its editor, persists a rename, and deletes it. It retains a
saved-view screenshot for inspection. Use a fresh simulator and the bounded
procedure above; compilation or an accessibility timeout before the first tap
does not verify the controls.

## Apple source lifecycle

Run the isolated package suite with `--filter 'SourceLifecycleTests|SourceLifecycleServiceTests|SourceActivityTests'`.
It covers explicit vault scoping, configured-source restrictions, masked settings,
trash and restore conflicts, retained outcomes, and failed requests.
Run `SourceLifecycleUITests` in the ClassificationTests fixture with temporary
simulator state. Its `--source-lifecycle` launch mode uses in-memory services
and exercises owner controls, read-only access, retry, and conflicts.

## Apple workflow lifecycle

Run the isolated package suite with `--filter 'WorkflowLifecycleTests|WorkflowLifecycleServiceTests'`.
These cases cover explicit vault scope, lifecycle metadata, retained runs,
owner controls, conflicts, retry, and stale asynchronous results.
Run `WorkflowLifecycleUITests` in ClassificationTests. The `--workflow-lifecycle`
fixture uses in-memory services and records screenshots for trash, retained
history, restored workflows, conflict recovery, read-only controls, and retry.

## Verify Apple vault management

Use the isolated package procedure above for `ManagementRequestTests`,
`VaultManagementTests`, and `WorkspaceTests`. The adapter tests cover query strings,
server base paths, explicit vault scope, missing-scope guidance, and empty grant
responses. The model tests cover retained recovery metadata, rejected revocations,
member-role reloads, and library state after access or account-default changes.

`VaultLifecycleUITests` in `apple/ClassificationTests` launches the production vault
management view with an in-memory service through `--vault-lifecycle`. It checks
owner metadata, member controls, history, viewer restrictions, and recovery from
an empty catalog. Inspect its retained screenshots after running the isolated UI
fixture. The fixture does not access a server, app credentials, or the Keychain.

Before merging native changes, run the package tests and unsigned macOS and iOS
Simulator builds. Static parsing and formatting do not verify Swift types or UI
behavior. Use a fresh simulator or isolated test account with a disposable server
for the following checks:

- As an owner, create and rename a vault, choose an account default, grant user and
  group access, change a role, and revoke a grant. Check the retained history.
- Confirm that a rejected last-owner change keeps the current grant visible and
  shows the server error. Choose another default before deleting a vault.
- As a viewer, inspect lifecycle metadata and confirm that owner controls and
  owner history are absent.
- Delete and restore a vault. Check that creation attribution and history remain.
  With no active vault, open management, restore a retained vault, and return to
  its library. Confirm that no unscoped document or saved-view request occurs.
- Refresh after the selected vault is revoked or its role changes. Check that
  stale results and selection clear. A changed default must preserve an accessible
  selected vault and its query.

Inspect the affected screens in light and dark appearances and at large Dynamic
Type sizes. Confirm that owner actions, recovery controls, and server errors remain
reachable. Never use the signed-in app profile or the user's Keychain.

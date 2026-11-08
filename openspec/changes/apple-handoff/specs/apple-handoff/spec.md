# Spec Delta

## Purpose

Lets a user continue the Folio screen they are looking at on another of their Apple devices, or in a browser on a device without the app, without leaving the server they are signed in to.

## ADDED Requirements

### Requirement: Screens that offer Handoff

Each unlocked window SHALL advertise at most one Handoff activity: the selected document (with its discussion pane when that is open), or, when no document is selected, the vault library (Inbox or All documents) or saved view on screen. Search, upload or scan review, classifications and settings SHALL NOT advertise one.

#### Scenario: Viewing a document

- **WHEN** the user has a document's detail open on an iPhone signed in to a server
- **THEN** a nearby iPad or Mac signed in to the same iCloud account offers to continue that document in Folio

#### Scenario: Viewing the discussion

- **WHEN** the user opens a document's discussion
- **THEN** the window's single document activity now also asks the receiver to open the discussion, and closing the discussion removes that request

#### Scenario: Browsing a vault or saved view

- **WHEN** the user is on the Inbox, All documents or a saved view of a vault and no document is selected
- **THEN** the advertised activity identifies that vault and that list or saved view

#### Scenario: Split view with a selected document
- **WHEN** an iPad or Mac window shows the document list and a selected document's detail side by side
- **THEN** that window advertises only the selected document, and advertises the list again once the selection is cleared

#### Scenario: Searching

- **WHEN** the user is typing or reviewing a search
- **THEN** no activity carries the search text and the activity for the underlying list stays as it was

### Requirement: Activity types are declared

The Apple app SHALL declare each activity type it advertises or accepts in its `NSUserActivityTypes`: `com.cedricziel.folio.document`, `com.cedricziel.folio.library` and `com.cedricziel.folio.saved-view`. It SHALL ignore activities of any other custom type.

#### Scenario: Declared type arrives

- **WHEN** another device hands over a `com.cedricziel.folio.document` activity
- **THEN** the app is offered for continuation and receives it

#### Scenario: Unknown type arrives

- **WHEN** an activity with an undeclared Folio type arrives
- **THEN** the app does not navigate

### Requirement: Payload identifies server and target

Each activity's user info SHALL carry `version` (integer `1`), `server` (the sender's server base URL), and `vault`. A document activity adds `document` and an optional `pane` = `discussion`. A library activity adds `list` = `inbox` or `all`. A saved-view activity adds `savedView`. All IDs are lowercase UUID strings.

#### Scenario: Round trip

- **WHEN** a payload is encoded into user info and decoded on another device
- **THEN** the decoded server, vault, target and pane equal the originals

#### Scenario: Malformed or newer payload

- **WHEN** user info lacks a required key, holds an invalid UUID, a URL that is not http(s) with a host, or a `version` other than `1`
- **THEN** decoding fails, the app does not navigate, and it opens the activity's web page URL if one is present

### Requirement: Handoff reveals no document content

Activity user info and keywords SHALL NOT carry document, vault or saved-view names, document content, search text or credentials. The title MAY be the visible document's title, which only the user's own devices see, and is never used for routing. Activities SHALL NOT be eligible for search, public indexing or prediction.

#### Scenario: Inspecting an advertised activity

- **WHEN** the app advertises an activity for a document titled "Tax return 2025"
- **THEN** its user info and keywords do not contain that title, and its search, public-indexing and prediction eligibility are all false

### Requirement: Server identity is compared canonically

The app SHALL treat two server URLs as the same server when their canonical forms are equal. Canonical form: lowercased scheme; host lowercased and, if internationalised, in its punycode (IDNA) form, IPv6 literals kept bracketed; default port (80 for http, 443 for https) removed; user info, query and fragment removed; trailing slashes trimmed from a case-sensitive path.

#### Scenario: Equivalent spellings

- **WHEN** the payload names `https://Folio.Example.com:443/` and the receiver is signed in to `https://folio.example.com`
- **THEN** the servers match

#### Scenario: Different scheme, port or path

- **WHEN** the two URLs differ in scheme, in a non-default port, or in the path prefix (e.g. `/folio` vs `/`)
- **THEN** the servers do not match

#### Scenario: Internationalised host
- **WHEN** the payload names `https://bücher.example` and the receiver is signed in to `https://xn--bcher-kva.example`
- **THEN** the servers match

#### Scenario: IPv6 literal with a port
- **WHEN** the payload names `http://[::1]:8080/` and the receiver is signed in to `http://[::1]:8080`
- **THEN** the servers match, and `http://[::1]` does not match either

### Requirement: Continuation on the same server opens the target

When the receiver is signed in to the payload's server, it SHALL select the payload's vault and open the target. A document opens its detail (with the discussion presented when requested), a library opens Inbox or All documents, and a saved view opens that saved view.

#### Scenario: Continue a document

- **WHEN** a document activity arrives and the receiver is signed in to the same server with access to the vault
- **THEN** the app selects that vault and shows the document's detail

#### Scenario: Continue a discussion

- **WHEN** a document activity with the `discussion` pane arrives
- **THEN** the app shows the document's detail with its discussion presented

#### Scenario: Continue a saved view that no longer exists

- **WHEN** a saved-view activity names a saved view the receiver cannot load
- **THEN** the app opens that vault's library and reports that the saved view is unavailable

### Requirement: Inaccessible vault or document is reported

If the vault is not in the receiver's vault list, the app SHALL report that the vault is no longer available and stay where it was. If the document cannot be loaded, because it was deleted, trashed or access was revoked, the app SHALL show its document-unavailable state in that vault.

#### Scenario: Vault access revoked

- **WHEN** a document activity names a vault the receiver's account can no longer see
- **THEN** the app shows "This vault is no longer available." and keeps its current vault

#### Scenario: Document trashed

- **WHEN** a document activity names a document that has since been moved to Trash
- **THEN** the app opens the vault and shows the document as unavailable without offering to restore it

### Requirement: Continuation never changes the account
The app SHALL NOT sign out, switch servers or discard a stored sign-in because of a continuation. When its session (signed in, expired, interrupted or unavailable) names another server than the payload, it SHALL keep that session and offer to open the activity's web page URL in a browser.

#### Scenario: Receiver on another server
- **WHEN** a document activity from server A arrives on a device signed in to server B
- **THEN** the app stays signed in to B, explains the document is on A's host, and offers "Open in Browser" for the activity's web page URL

#### Scenario: Expired session on another server
- **WHEN** a document activity from server A arrives while the app's session for server B has expired
- **THEN** the app offers "Open in Browser" and leaves the expired session for B as it was

### Requirement: Signed-out receiver is offered sign-in
When the app is signed out, a Handoff or universal-link continuation SHALL show sign-in prefilled with the payload's server and open the target after sign-in to that server. A Spotlight continuation while signed out SHALL be ignored.

#### Scenario: Receiver signed out
- **WHEN** a document activity from `https://folio.example.com` arrives on a signed-out device
- **THEN** the sign-in screen shows that server address and, once the user signs in to it, the document opens

#### Scenario: Sign-in to a different server instead
- **WHEN** the user edits the prefilled address and signs in to another server
- **THEN** the held continuation is discarded and the app opens normally

#### Scenario: Spotlight result while signed out
- **WHEN** a Folio Spotlight result is opened on a signed-out device
- **THEN** the app shows its normal sign-in screen and does not navigate after sign-in

### Requirement: Continuation waits for the same server's session
While the app is restoring its session, or its session for the payload's server is expired, interrupted or unavailable, it SHALL hold the continuation and open the target once that session becomes signed in. Signing out discards it.

#### Scenario: Cold launch
- **WHEN** a continuation launches the app and session restoration is still running
- **THEN** the target opens once restoration reports signed in to the same server

#### Scenario: Expired session on the same server
- **WHEN** a document activity arrives while the session for that server has expired
- **THEN** the app keeps the continuation through reconnecting and opens the document once signed in again

#### Scenario: Server unreachable
- **WHEN** a continuation arrives while the session is unavailable because the server cannot be reached, and the user later signs out instead of retrying
- **THEN** the continuation is discarded

### Requirement: Continuation is handled once in an existing window
A continuation SHALL be handled exactly once by the app, in an existing window (the frontmost one when several are open), without opening a new window.

#### Scenario: Mac with two windows
- **WHEN** a document activity arrives on a Mac with two Folio windows open
- **THEN** only the frontmost window navigates to the document, the other window is unchanged, and no third window opens

#### Scenario: iPad with two scenes
- **WHEN** the same arrives on an iPad with two Folio scenes in Split View
- **THEN** the scene the system activates handles it and the other scene is unchanged

### Requirement: Activities fall back to the web page

Each activity SHALL set its web page URL to the same target on the sender's server: `{server}/vaults/{vault_id}/documents/{document_id}` for a document or discussion, `{server}/vaults/{vault_id}/` (Inbox) or `{server}/vaults/{vault_id}/?view=all` for a library, and `{server}/vaults/{vault_id}/saved-views/{saved_view_id}` for a saved view. `{server}` keeps any path prefix.

#### Scenario: Device without the app

- **WHEN** a document activity is handed to a Mac without Folio installed
- **THEN** the Mac offers to open `{server}/vaults/{vault_id}/documents/{document_id}` in the browser

#### Scenario: Browser is not signed in

- **WHEN** that page is opened in a browser without a Folio session
- **THEN** the server sends it to `/login` and returns it to the same page after sign-in

### Requirement: Stable web URLs for handed-over targets

The server SHALL keep serving the HTML pages that activities link to: `GET /vaults/{vault_id}/documents/{id}` (document detail), `GET /vaults/{vault_id}/` and `GET /vaults/{vault_id}/?view=all` (Inbox and All documents), and `GET /vaults/{vault_id}/saved-views/{id}` (saved view). Each answers 200 for an accessible target and 404 for a missing or inaccessible one. An unauthenticated GET redirects to `/login?next=<path>`.

#### Scenario: Accessible document

- **WHEN** a signed-in member requests `GET /vaults/{vault_id}/documents/{id}` for an active document in that vault
- **THEN** the response is 200 with the document detail page

#### Scenario: Trashed document

- **WHEN** the same URL names a document in Trash
- **THEN** the response is 404

#### Scenario: Not signed in

- **WHEN** a request without a session asks for `GET /vaults/{vault_id}/saved-views/{id}`
- **THEN** the response redirects to `/login?next=%2Fvaults%2F{vault_id}%2Fsaved-views%2F{id}`

### Requirement: Activities follow the session

The app SHALL stop advertising an activity when it signs out, when the selected vault changes away from the activity's vault, and when the advertised screen closes. After a vault switch, only the new vault's screen is advertised.

#### Scenario: Sign out

- **WHEN** the user signs out while a document activity is advertised
- **THEN** no Folio activity remains advertised

#### Scenario: Vault switch

- **WHEN** the user switches from vault A to vault B
- **THEN** activities naming vault A are no longer offered and the library activity names vault B

### Requirement: Universal links route like Handoff

A self-built app with associated domains SHALL accept universal links for the web URLs above and route them through the same rules as Handoff. It derives the server from the link's scheme, host, port and the path before `/vaults/`. A link it cannot parse SHALL open in a browser view inside the app instead of navigating.

#### Scenario: Tapping a document link

- **WHEN** a link `https://folio.example.com/vaults/{vault_id}/documents/{id}` opens a self-built app signed in to `https://folio.example.com`
- **THEN** the app opens that document

#### Scenario: Link under a path prefix

- **WHEN** a link `https://example.com/folio/vaults/{vault_id}/documents/{id}` arrives
- **THEN** the server is `https://example.com/folio` for matching

#### Scenario: Link to an unsupported page

- **WHEN** a universal link reaches the app with a path it does not route, for example from an association a device cached before the server changed it
- **THEN** the app presents the URL in an in-app browser view and does not navigate

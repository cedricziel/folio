# Folio roadmap

Draft, 2026-11-14. Written against v0.17.0.

This roadmap collects the November 2026 analysis of the server, the web UI and the Apple apps into one plan we can set goals from. It lists the jobs users hire Folio for, the bar we hold the experience to, the bugs that break trust today, and the features that would make Folio the best place to keep paperwork.

Where a finding came from matters, so each item carries a marker.

- **(seen)** observed in a running ephemeral instance
- **(code)** read in the source, not exercised
- **(suspected)** inferred, needs a device or a load test to confirm

The use cases are inferred from the code, docs and issues. Nobody has interviewed users yet. Treat the jobs below as a hypothesis to test.

## Where Folio stands

Capture and plumbing are strong. Scanning, the share extension, folder and IMAP ingestion, Shortcuts, idempotent uploads, live processing updates, vault-scoped authorization and a large test suite all work.

Filing and finding are weak. Folio calls itself AI-native, but the only AI in the codebase is OCR and embeddings. There is no LLM port and no suggestion code. A user titles, dates and files every document by hand, one page visit at a time. Text search is `LIKE` with no full-text index.

Closing that gap is the centre of this roadmap.

## Jobs to be done

| #   | Job           | Today  | What great looks like                                                                             |
| --- | ------------- | ------ | ------------------------------------------------------------------------------------------------- |
| J1  | Get paper in  | Strong | No upload is ever lost, on any network. A file Folio cannot read says so in plain words.          |
| J2  | File it       | Manual | Folio proposes title, sender, type, date and tags. The user confirms a batch from the keyboard.   |
| J3  | Find it again | Weak   | One search box handles words, typos, tags, dates and meaning, with a results page and filters.    |
| J4  | Act on it     | Thin   | Due dates and reminders. Questions about the archive answered with citations.                     |
| J5  | Trust it      | Mixed  | Backup, restore and full export are documented and tested. Lists never show stale or wrong state. |

Who we think the users are: a household keeping invoices, contracts, tax and insurance papers. A freelancer keeping business records. A family sharing some vaults and keeping others private. Someone moving an existing archive off Paperless-ngx.

## The experience bar

These are the rules that make Folio pleasant instead of merely functional. Every change in this roadmap has to meet them, and a PR that breaks one needs a reason.

1. **Folio does the work, the user decides.** Suggestions are proposals. Folio never writes a guess into a document without the user confirming it.
2. **Every action answers back.** A save, a move to trash or a retry shows a result within 200 ms, and destructive actions offer undo.
3. **The user never loses their place.** Vault, filters, scroll position and an unfinished scan survive navigation, app lock and a trip to settings.
4. **Errors name the cause and the next step.** "Couldn't reach the server" is not enough when the cause is a certificate, a blocked `http://` address or an expired session.
5. **No machinery on screen.** People have names, times are readable, and identifiers stay out of the UI.
6. **The keyboard goes everywhere.** Triage, search, editing and navigation all work without a pointer on web, iPad and Mac.
7. **State is always current.** If a document finished processing, every open list shows it without a manual refresh.
8. **It speaks the user's language.** German and English first, with locale-aware dates and plurals.
9. **Accessible by default.** WCAG 2.2 AA on web with zero serious axe violations, VoiceOver labels and Dynamic Type on Apple.
10. **Fast enough to forget.** List and search responses under 300 ms at 10,000 documents.

## Goals

Proposed targets. The numbers are a starting point for discussion, not measurements.

| Goal                        | Measure                                                      | Target                                   |
| --------------------------- | ------------------------------------------------------------ | ---------------------------------------- |
| G1 Filing is nearly free    | Median time from upload to filed, 20-document batch          | Under 60 seconds for the batch           |
| G2 Suggestions are right    | Share of suggested fields accepted without edits             | 80% on a fixed evaluation set            |
| G3 Search finds it          | Known-item searches with the document in the top 5           | 95% on a fixed query set, typos included |
| G4 Nothing is lost          | Uploads lost, and archives unrecoverable after restore drill | Zero                                     |
| G5 Nothing is stale         | Lists needing a manual refresh after processing              | Zero on web and Apple                    |
| G6 Safe by default          | Open high-severity security findings                         | Zero                                     |
| G7 Scales to a real archive | p95 list and search latency at 10,000 and 100,000 documents  | 300 ms and 1 s                           |
| G8 Less work in progress    | Open OpenSpec changes                                        | 6 or fewer at any time                   |

## Now: trust and triage

The theme is to stop losing the user's place, data and confidence. Nothing here needs a design phase.

### Bugs

| ID  | Bug                                                                                                       | Where                                                   | Source        |
| --- | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------------- |
| B1  | Vault selector resets to the default vault after visiting Settings                                        | web, `base.html:2002`                                   | seen          |
| B2  | Titles cannot be edited on the web, so uploads keep raw filenames                                         | web, `document_view.html:13`                            | seen          |
| B3  | Back link from a document always goes to Inbox, dropping filters and search                               | web, `document_view.html:10`                            | seen          |
| B4  | Raw UUIDs and nanosecond timestamps shown as people and times                                             | web, `vaults.html`, `entries.html`                      | seen          |
| B5  | A `.txt` upload is accepted, then reported as "Completed with no text"                                    | web upload                                              | seen          |
| B6  | Edits reload silently, trash has no toast or undo                                                         | web                                                     | seen          |
| B7  | Sidebar navigation on iPhone can stop responding, likely the rest of #209                                 | Apple, `WorkspaceView.swift:35-38`                      | suspected     |
| B8  | Lists never refresh on push, foreground or timer                                                          | Apple, `InboxView.swift`, `PushNotifications.swift:116` | code          |
| B9  | App lock discards navigation, search, a pending upload review and an unfiled scan                         | Apple, `FolioApp.swift:311`                             | code          |
| B10 | A 5xx during token refresh forces a full re-sign-in                                                       | Apple, `OAuthSession.swift:163`                         | code          |
| B11 | Every connection failure reads "Couldn't reach the server"                                                | Apple, `LiveFolioService.swift:725`                     | code          |
| B12 | Share extension blames a missing server address for an expired session or no network                      | Apple, `ShareViewController.swift:36`                   | code          |
| B13 | Tag, correspondent and type changes use form posts outside the API spec and infer success from a redirect | Apple and `routes/documents.rs:785`                     | code          |
| B14 | An unknown vault role fails the whole vault list                                                          | Apple, `Vault.swift:4`                                  | code          |
| B15 | PDF fails to open in the iOS Files provider                                                               | #177                                                    | issue         |
| B16 | Document download failures                                                                                | #300                                                    | issue         |
| B17 | Live lists drift from active filters                                                                      | #125                                                    | issue         |
| B18 | Live event streams outlive graceful shutdown                                                              | #151                                                    | issue         |
| B19 | Destructive actions confirm differently by mouse and keyboard, and some have no confirmation              | web, `vaults.html`, `sources.html`                      | seen and code |

### Security

| ID  | Finding                                                                                 | Where                                          | Source          |
| --- | --------------------------------------------------------------------------------------- | ---------------------------------------------- | --------------- |
| S1  | With `auth.required=false`, permissive CORS lets any website read and write the archive | `routes/mod.rs:131`, `routes/auth.rs:473`      | code, rechecked |
| S2  | A `documents:write` token can create workflows whose webhooks reach private addresses   | `routes/auth.rs:437`, `models/workflow.rs:489` | code            |
| S3  | Login throttling keys on the TCP peer, so a reverse proxy puts every user in one bucket | `routes/auth.rs:1100`                          | code            |
| S4  | The setup token is logged at `warn!` and reaches OTLP log export                        | `folio-app/src/main.rs:452`                    | code            |
| S5  | Main app pages send no CSP or frame protection                                          | `routes/auth.rs:667`                           | code            |

### Processing safety

| ID  | Item                                                                                          | Where                       | Source          |
| --- | --------------------------------------------------------------------------------------------- | --------------------------- | --------------- |
| P1  | AI HTTP clients have no timeout, and the serial worker stalls behind one hung provider (#159) | `infra/ai/pipeline.rs:57`   | code, rechecked |
| P2  | Completed jobs are never deleted because `JobQueue::cleanup` has no caller                    | `infra/queue/mod.rs:350`    | code, rechecked |
| P3  | SQLite runs without WAL or a busy timeout (#461)                                              | `database/sqlite/mod.rs:57` | code, rechecked |
| P4  | SQLite dequeue panics the worker on a malformed row                                           | `queue/sqlite.rs:190`       | code            |
| P5  | One embedding model per document across batches (#162)                                        | worker                      | issue           |
| P6  | Mixed digital and scanned PDFs lose the scanned pages' text (#161)                            | OCR                         | issue           |

### Triage features

| ID  | Feature                                                          | Jobs |
| --- | ---------------------------------------------------------------- | ---- |
| F1  | Inline title editing on the document page and in lists           | J2   |
| F2  | Bulk select with bulk tag, type, sender, trash and mark reviewed | J2   |
| F3  | Keyboard triage in the inbox: accept, next, previous, trash      | J2   |
| F4  | Sort and list or grid toggle on every list, not only saved views | J3   |
| F5  | "Empty trash"                                                    | J5   |
| F6  | Toasts with undo for edits and trash on web                      | J5   |
| F7  | Apple lists refresh on push and on foreground                    | J5   |
| F8  | Vault members shown and added by name, with a picker             | J5   |
| F9  | Autocomplete combobox for correspondents, types and tags (#92)   | J2   |

Exit for this phase: every B, S and P item above is closed or has a dated decision not to fix, and G5 and G6 are met.

## Next: Folio does the filing

The theme is to make the AI-native claim true and make search dependable. These need design before code.

| ID  | Feature                                                                        | Notes                                                                                                                                                          | Jobs |
| --- | ------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- |
| F10 | LLM port beside OCR and embeddings                                             | Opt-in per vault. Ollama as the local default, cloud providers optional, same provider model as OCR.                                                           | J2   |
| F11 | Suggested title, sender, type, document date, amount and due date              | Runs after text extraction. Stored as suggestions, shown in the inbox, never written silently. Builds on `typed-document-metadata` and `document-date-review`. | J2   |
| F12 | One-keystroke accept in the triage view                                        | Depends on F3 and F11.                                                                                                                                         | J2   |
| F13 | Full-text indexes, FTS5 on SQLite and `tsvector` on Postgres                   | Replaces `LIKE`. Typo tolerance and tag-name matches.                                                                                                          | J3   |
| F14 | Hybrid ranking with filters applied before limiting (#137)                     | Fuses text and vector results. Stops swallowing semantic-search errors.                                                                                        | J3   |
| F15 | Search results page with filters, scoped to the current destination            | The palette stays for quick jumps.                                                                                                                             | J3   |
| F16 | Typed pgvector column with an ANN index, bounded scan on SQLite (#140)         | Needed for G7.                                                                                                                                                 | J3   |
| F17 | Backup and restore, documented and tested with a restore drill (#90)           |                                                                                                                                                                | J5   |
| F18 | Full export of files and metadata in an open format                            |                                                                                                                                                                | J5   |
| F19 | Finish the Paperless import, 19 of 48 tasks done                               | The main way new users arrive with an archive.                                                                                                                 | J1   |
| F20 | German and English on web and Apple                                            | String catalogs, locale-aware dates and plurals.                                                                                                               | all  |
| F21 | Streaming uploads and bounded OCR rendering                                    | Removes whole-file reads, caps pages, protects the share extension from memory kills.                                                                          | J1   |
| F22 | Cancel an upload in progress, refresh the token for delayed background uploads | Apple.                                                                                                                                                         | J1   |
| F23 | Specific connection errors and an in-place sign-in recovery                    | Apple. Covers certificate, blocked `http://`, timeout and expired session.                                                                                     | J5   |
| F24 | Document inspector on iPad and Mac, wrapping tags                              | Replaces the fixed 220pt panel.                                                                                                                                | J2   |
| F25 | Saved views finished: row layout, styled form, listed in the sidebar           |                                                                                                                                                                | J3   |
| F26 | Document viewer with zoom and open-original                                    |                                                                                                                                                                | J3   |
| F27 | Settings split into sections with one save per section                         |                                                                                                                                                                | J5   |

Exit for this phase: G1, G2, G3 and G4 are met on the evaluation sets, and a new user can import a Paperless archive without help.

## Later: act on it

| ID  | Feature                                                                            | Notes                                                 | Jobs     |
| --- | ---------------------------------------------------------------------------------- | ----------------------------------------------------- | -------- |
| F28 | Ask your archive, answers with citations                                           | Web and MCP. Needs F10 and F14.                       | J4       |
| F29 | Due dates with reminders, and export to Reminders or Calendar (#368)               | Uses the due date from F11.                           | J4       |
| F30 | Workflow actions beyond webhooks                                                   | Notify, set a due date, move to a vault.              | J4       |
| F31 | Share a document by expiring link                                                  | No share-link code exists today.                      | J4       |
| F32 | Duplicate and near-duplicate detection surfaced in the inbox                       | SHA-256 dedup exists, near-duplicates do not.         | J2       |
| F33 | Offline reading cache on Apple                                                     | No document cache today.                              | J3       |
| F34 | Apple system UI: Live Activities, widgets, interactive snippets (#351, #362, #363) | Resume after the Next phase.                          | J1       |
| F35 | Apple Intelligence and Siri tools (#350, #354, #357)                               | Resume after F28, which gives them something to call. | J4       |
| F36 | Voice notes, on-device translation, PaperKit markup (#370, #369, #367)             |                                                       | J4       |
| F37 | watchOS and visionOS evaluation (#372)                                             |                                                       | none yet |

## Engineering foundations

These run alongside the phases and make the feature work cheaper.

| ID  | Item                                                                                                  | Source          |
| --- | ----------------------------------------------------------------------------------------------------- | --------------- |
| E1  | Share query logic between the SQLite and Postgres adapters, about 10,000 duplicated lines each        | code            |
| E2  | Split `worker.rs` (2,794 lines) and remove the second OCR path                                        | code            |
| E3  | Remove or adopt core's unused `IngestService`                                                         | code, rechecked |
| E4  | Run the API, MCP and e2e suites against Postgres as well as SQLite                                    | code            |
| E5  | Make the e2e job a required check                                                                     | code            |
| E6  | Move pgvector DDL from startup into migrations, and align index differences between the two databases | code            |
| E7  | Bound document-list and live-counter database work (#152)                                             | issue           |
| E8  | Add a compile-time drift check for the handwritten Apple decoders, or generate them                   | code            |
| E9  | Add an iPhone sidebar path to `NavigationTests`                                                       | code            |
| E10 | Extend visual baselines from 3 screens to the full `screens.json` set                                 | code            |
| E11 | Describe SSE events in OpenAPI (#32)                                                                  | issue           |
| E12 | Fix span export in tests under SQLite (#395)                                                          | issue           |
| E13 | Pin the toolchain so `build.sh` works, since `mail-parser` needs rustc 1.98                           | seen            |
| E14 | Evaluation sets for G2 and G3, checked in and run in CI                                               | new             |

## Housekeeping

- Close or archive stale OpenSpec work. `integration-access` is complete and not archived. Five specs still carry a placeholder purpose.
- Merge the two `verify` skills in `.claude` and `.agents` into one. Done 2026-11-14 in #476.
- Update `docs/deployment.md`, which pins image `0.1.0`, and `docs/database.md`, which names a table the code no longer uses.
- Stop duplicate changelog lines for a commit and its PR.

## What we pause

Nineteen OpenSpec changes are open and the last five releases went mostly to Apple platform depth. Capture is already the strongest job, so more of it has the lowest return right now.

Paused until the Next phase exits: the OS 27 epics beyond what the SDK requires (#373 stays), widgets and Live Activities, Visual Intelligence, Finder search, watchOS and visionOS.

Kept: anything that fixes a bug in the tables above, and `apple-handoff`, `pluggable-processing-pipeline`, `vault-library-switching` and `workflow-mutation-dedup`, which are each within a few tasks of done.

## How we work on it

- Each roadmap item becomes one small PR, or one OpenSpec change when it needs design.
- Design, root-cause analysis and review go to Opus. Implementation goes to Sonnet. Mechanical edits go to Haiku. The top model orchestrates and writes the syntheses.
- A failing test comes first for every behavior change.
- Every UI change runs the `verify` skill against an ephemeral instance before its PR.
- We review this file at each minor release and move items between phases with a dated note.

## Open decisions

1. Should suggested filing default to a local model, with cloud providers opt-in per vault? This roadmap assumes yes.
2. Are the G1 to G8 targets right, and who owns each?
3. Which users do we interview to test the jobs in this document?
4. Is German and English the right first pair of languages?
5. Do the paused Apple epics stay paused if the OS 27 release date forces some of them?

## Not yet checked

Real OCR with Tesseract, semantic search with embeddings on, OIDC and passkeys, multi-user roles on the web, mail and Paperless sources end to end, and everything in the Apple apps that needs a running device. One valid PDF uploaded from a phone-sized browser ended in "Couldn't process this file" during the sweep, and nobody has reproduced it yet.

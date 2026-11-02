# Document workspace implementation plan

Status: implementation started on 2026-11-02. Contracts [#192](https://github.com/cedricziel/folio/pull/192) and pagination [#193](https://github.com/cedricziel/folio/pull/193) are merged. Native navigation [#195](https://github.com/cedricziel/folio/pull/195) is merged.

## Scope and outcome

Build the approved native Apple workspace: Inbox, All documents, Recently added, Correspondents, Tags, Document types, and Saved views. Use one document browser with list and grid presentations, persistent filters, optional preview, editable metadata, and bulk actions. Support macOS and adaptive iPad and iPhone navigation. Extend the shared backend without redesigning the web application or MCP tools.

Deliver this in six milestones. Each milestone produces a usable increment. Keep only two to four dependent PRs open at once, merge bottom-up, and start subsequent work from the merged base. The table numbers are planning IDs, not GitHub issue or PR numbers. Target 150–400 handwritten changed lines per PR and investigate splits above 500. Contract tests and generated OpenAPI changes belong with the behavior they validate. Line budgets are estimates, not reasons to omit tests.

## Verified starting point

- `apple/Folio/Sources/InboxView.swift` implements a two-column document list and reader. Menu commands select Inbox or All documents. `DocumentListViewModel.scope` defaults to `.all`.
- `DocumentDetailView.swift` displays read-only metadata. `DocumentRowView.swift` uses a generic paper placeholder.
- `apple/FolioKit/Sources/FolioKit/Networking/FolioService.swift` supports query search, structured filters, taxonomy reads, and tag assignment. The app does not expose most of these operations.
- `crates/folio-api/src/routes/documents.rs` provides list/get/upload/delete and search. Classification writes are form posts with redirects. Lists return enriched metadata, while search has a different response shape. The Apple search adapter requests at most 50 results.
- `crates/folio-api/src/routes/{tags,correspondents,document_types}.rs` already provide JSON CRUD. Extend these rather than creating a second taxonomy service.
- `crates/folio-api/src/routes/pages.rs` already serves thumbnails and supports reprocessing for the web. Add documented client-facing access rather than a new rendering pipeline.
- Both Rust `Document.status()` and Swift `Document.status` infer review completion from classification. They also differ in their treatment of embedding progress. Neither represents an explicit human review decision.
- Database repositories already accept `VaultId`. The HTTP router currently uses default-vault resolution and unscoped collection URLs. The active `add-vaults-and-access` OpenSpec change plans explicit vault URLs and access enforcement. Do not mistake that proposal for completed routing.
- The active `pluggable-processing-pipeline` change overlaps processing state and search. Coordinate contracts before changing these areas.

## Product rules

### Navigation and presentation

The sidebar contains Library, Browse by, and Saved views groups. Inbox is the first-run destination. Later launches restore the last workspace unless the user chooses Always open Inbox. Recently added means all documents ordered by arrival time, grouped by day; it has no hidden time cutoff.

List and grid are presentations of the same query and selection. List emphasizes title, correspondent, date, and exceptional status. Grid uses real first-page thumbnails. Keep the reader closed until selection requires it. Inbox review can keep the reader and inspector open. Narrow layouts navigate from results to detail and restore results on Back.

Keep upload, scan, settings, and account access discoverable in the new shell. Respect the existing platform-specific scanner availability. Account and server changes clear cached data and restored selection. When explicit vault switching lands, apply the same rule to vault changes.

### Query behavior

Search defaults to the current destination and offers Search all documents within the current vault. Scope, search, filters, sorting, and presentation remain visible and survive detail navigation. Switching presentation does not refetch unnecessarily.

Filters support correspondent, document type, all selected tags, explicit unassigned values, added-date range, document-date range, review state, and processing state. Dates use inclusive lower and exclusive upper bounds. Convert local date selections to explicit boundaries. Keep document date separate from arrival time; do not guess it from upload time.

Use a typed, versioned query model shared by results and saved views. Reject unknown filter versions instead of silently broadening results. First ship deterministic title/content text search with filters and pagination. Preserve the existing hybrid search endpoint. Add filtered semantic search as a separate increment; never filter a globally limited top-50 response afterward.

### Review behavior

Store review state independently of OCR, embeddings, and metadata completeness. New imports are unreviewed even when a workflow classifies them. Mark reviewed is explicit and reversible. Metadata edits and reprocessing do not reset review state. A reviewed document with a processing failure still appears in the Inbox failure group.

Inbox membership is unreviewed OR processing active OR processing failed. Groups are mutually exclusive: failed first, then active processing, then ready to review. An embedding failure is labeled as a search-indexing problem, not an unreadable document. Define the mapping against the processing pipeline contract once; match it in Rust, Swift, the web inbox, and File Provider catalog behavior.

For upgrade compatibility, preserve the server's old Filed membership once: mark those existing records reviewed with migration provenance. Leave other records unreviewed. Do not invent a human reviewer or historical review time. This avoids flooding a mature archive while keeping new imports explicit. Record and test this compromise before the migration lands.

### Taxonomy and saved views

Taxonomy entries open filtered documents. Management is an explicit action on that entry. Directories offer search, counts, and unassigned groups. Rename preserves IDs. Delete removes assignments, never documents. Merge moves assignments to a chosen target in the same vault and deduplicates tags atomically.

Saved views are private to a user within a vault for the first release. They store a versioned query, name, and preferred presentation. Device-local state stores sidebar pins, scroll position, selected document, and window layout. Do not add these preferences to global server settings or config.toml. Sharing saved views is deferred.

## Backend contracts to settle in the first PR

These are proposed contracts, not claims about current endpoints. Use the explicit-vault route base once the access work lands. Do not introduce unguarded temporary routes to get ahead of it.

| Contract | Proposed shape | Required behavior |
| --- | --- | --- |
| Browse | `GET /api/vaults/{vault_id}/documents` with query, filter, sort, limit, and cursor parameters | Enriched document summaries, `next_cursor`, bounded pages; preserve existing callers through additive fields or an explicit compatibility adapter |
| Edit metadata | `PATCH /api/documents/{id}` | Title, nullable correspondent/type/date, and explicit tag operations; omitted means unchanged, null means clear; return updated document |
| Review | `PUT /api/documents/{id}/review` | Idempotent reviewed/unreviewed transition; reviewer and timestamp for human transitions; return updated state |
| Preview | `GET /api/documents/{id}/thumbnail` | Authenticated bytes, pending/unavailable states, cache validators, access checks identical to the original file |
| Retry | `POST /api/documents/{id}/reprocess` | Reuse the existing processing service; 202 with job identity; deduplicate active retries |
| Directory | Existing vault-scoped taxonomy collections | Bounded name search, stable ordering, document counts, unassigned totals; no per-entry document fetches |
| Merge | `POST /api/vaults/{vault_id}/{taxonomy}/{source_id}/merge` with target ID | One transaction; reject cross-vault or self merge; update saved-view references, including views the caller cannot read, without exposing their contents |
| Saved views | `GET/POST /api/vaults/{vault_id}/saved-views`, `GET/PATCH/DELETE .../{id}` | User-owned, schema-versioned query; access rechecked on execution; deleted filter references invalidate the view rather than remove the filter silently |
| Bulk classify | `POST /api/vaults/{vault_id}/documents/bulk` | At most 100 explicit IDs; per-item authorization and outcome; no implicit select-all across unloaded pages |

Normal reads and edits return 200; creations return 201; deletes return 204. Specify 400 validation, 401 unauthenticated, 404 inaccessible or missing resources, 403 insufficient role within an accessible vault, and 409 edit conflicts in OpenAPI. Use a document metadata revision for conditional edits so background processing does not cause unrelated metadata conflicts. Never trust cursor contents or saved-view IDs as authorization.

Use keyset pagination with a stable ID tie-breaker for chronological and title sorts. Bind cursors to vault, filters, and sort; reject reuse against a different query. Document concurrent-update semantics: a changing library is not a frozen snapshot; refresh restarts the query, and the client deduplicates IDs. For relevance-ranked search, choose a bounded result snapshot or a deterministic continuation strategy in its own design before promising pagination.

Batch-load tags and related names for a page. Return counts only where the UI needs them; do not run an exact global count on every keystroke. Directory counts and Inbox counts must be vault-scoped. Thumbnail caches must be scoped by account, server, vault, document, and content version, and cleared on sign-out.

## Small PR sequence

Each row includes its first regression test or acceptance check. Use `codex/workspace-<id>-<slug>` branches. A dependency means the prerequisite must land or be the branch base. Existing access and processing work are external dependencies, not new PRs in this plan.

### Milestone A: A usable library browser

| ID | PR scope | Depends on | First test / completion evidence |
| --- | --- | --- | --- |
| 01 | Document the workspace OpenSpec proposal, query/review contracts, compatibility policy, and overlaps with active changes. No implementation. | None | Validate the new change; scenarios name methods, paths, errors, migration rules, and old-client behavior. |
| 02 | Add typed paged browse queries and SQLite/PostgreSQL repository implementations for chronological/title sorting and existing filters. Keep legacy list methods usable. | 01 | Shared repository contract traverses tied timestamps across pages without omission; cursor mismatch and cross-vault tests fail first. |
| 03 | Expose paged JSON browse results and batch-enriched summaries. Update OpenAPI and the Apple schema input. | 02; access route contract | API test returns page two with correct tags/names and bounded enrichment queries; existing list callers still work. |
| 04 | Introduce the Apple workspace navigation and reusable browser state. Ship sidebar destinations backed by available operations; retain upload, scan, and account controls. | 01 | Navigation/restoration test keeps selected document and query when returning from detail; narrow layout screenshot. |
| 05 | Add Apple incremental loading through the paged API, cancellation, refresh, and stable selection. | 03, 04 | A slow old query cannot overwrite a newer query; loading the next page does not duplicate rows or lose selection. |
| 06 | Add text search plus existing filters to the paged backend query. Keep hybrid search unchanged. | 03 | A match outside the first unfiltered page is returned; combined tags use AND; unconfigured AI still returns literal matches. |
| 07 | Expose Apple filter controls, active-filter chips, sorting, Recently added, and scoped search. | 05, 06 | Search and filters compose; clearing one chip preserves others; added dates and timezone boundaries behave correctly. |

Slice 03 is delivered as 03a, batched tag enrichment using existing routes, then 03b, paged browsing with per-request access checks. Slice 03b integrates read authorization through the existing access policy and adds `/api/documents/browse` for default-vault clients. Vault management and migration of other routes remain in the access plan.

Milestone exit: users can navigate and retrieve more than 50 documents with combined search and filters. No classification work is needed to benefit from this release.

### Milestone B: Browse visually and find classifications

| ID | PR scope | Depends on | First test / completion evidence |
| --- | --- | --- | --- |
| 08 | Expose the existing thumbnail service through a documented authenticated API. | 03 | Pending, missing, ready, conditional request, and inaccessible-document cases; no public cache of private files. |
| 09 | Add Apple grid mode, bounded thumbnail loading, and per-destination presentation preference. | 05, 08 | List/grid retains selected IDs and filters; loading cancels offscreen requests; narrow and large-text screenshots. |
| 10 | Add bounded taxonomy directory queries with counts and explicit unassigned filters in both repositories and the API. | 03 | Counts match filtered document sets, stay within the vault, and do not require one query per entry. |
| 11 | Add Apple Correspondents, Tags, and Document types directories and filtered drilldown. | 07, 10 | Selecting an entry scopes results; Back restores directory search; renamed entries keep ID-based links. |

Milestone exit: users can recognize documents visually or navigate by sender, topic, and document type.

### Milestone C: Complete the Inbox review loop

| ID | PR scope | Depends on | First test / completion evidence |
| --- | --- | --- | --- |
| 12 | Add explicit review persistence and upgrade backfill in both databases. Keep the old derived status until consumers migrate. | 01; processing state contract | Old Filed records retain membership with migration provenance; new auto-classified uploads remain unreviewed. |
| 13 | Add authorized review transitions, query predicates/counts, and change events. | 03, 12; access enforcement | Review/unreview is idempotent; reviewer attribution is correct; processing failures remain visible independently. |
| 14 | Add JSON metadata editing with nullable fields, tag operations, conditional revisions, and existing-service reuse. | 03; access enforcement | Omitted versus null fields, stale revisions, cross-vault assignments, and forbidden edits fail correctly. |
| 15 | Add the Apple editable metadata inspector using the JSON API. | 11, 14 | Failed saves preserve edits, conflicts offer reload/retry, and changing metadata does not mark reviewed. |
| 16 | Adopt explicit review state across Apple Inbox, shared Rust status, web Inbox, and File Provider catalog. Add next-document review and undo. | 13, 15 | Common state fixtures agree across consumers; review advances only after success; undo restores membership; refresh does not lose the queue position. |
| 17 | Expose JSON processing retry and Apple failure recovery. Reuse the pipeline job service. | 16; processing retry contract | Repeated retry queues one active job; embedding failure copy differs from OCR failure; review state survives reprocessing. |

Milestone exit: import → review → correct → mark reviewed → next document works, including failure and undo paths. New automatic classification never silently completes review.

### Milestone D: Maintain the vocabulary

| ID | PR scope | Depends on | First test / completion evidence |
| --- | --- | --- | --- |
| 18 | Connect Apple taxonomy create/rename/delete to existing CRUD with explicit affected-document behavior. | 11, 15 | Duplicate names are actionable; deleting a tag preserves documents; canceled edits have no effect. |
| 19 | Add transactional taxonomy merge in repositories and API, one taxonomy kind at a time if the diff grows. | 10, 18 | Source assignments move to target, duplicate tags collapse, concurrent changes roll back safely, and cross-vault merge fails. |
| 20 | Add Apple merge selection and confirmation showing source, target, and affected document count. | 19 | Cancel leaves data intact; success refreshes directories and open documents; stale counts are not treated as an authorization guarantee. |

Milestone exit: duplicate names and inconsistent classification can be repaired without moving or deleting documents.

### Milestone E: Repeatable workflows

| ID | PR scope | Depends on | First test / completion evidence |
| --- | --- | --- | --- |
| 21 | Add user/vault-owned saved-view persistence, JSON CRUD, and schema validation. Extend taxonomy merge/delete to maintain or invalidate references atomically. | 07, 19; user/vault identity | A different user cannot read the view; revoked vault access blocks execution; unknown versions and deleted references never broaden results. |
| 22 | Add Apple Save view, rename/delete, sidebar pins, and per-view layout. Finish launch preference and account-scoped restoration. | 09, 21 | Saved query round-trips every filter; deletion removes pins; reopening restores layout; sign-out cannot restore another account's data. |
| 23 | Add bounded bulk-classification API using explicit IDs and per-document outcomes. | 13, 14 | Mixed successes/failures are reported accurately; unauthorized IDs are not changed; tag add/remove leaves unrelated tags intact. |
| 24 | Add Apple multi-selection, bulk classification/review, and selected-file export. | 22, 23 | Selection survives list/grid; only selected IDs change; partial failure retains failed selections; exported bytes match originals. |

Milestone exit: users can revisit useful queries and process groups of documents. Bulk export initially uses existing downloads with bounded concurrency; large archive jobs are deferred.

### Milestone F: Dates, semantic retrieval, and scale

| ID | PR scope | Depends on | First test / completion evidence |
| --- | --- | --- | --- |
| 25 | Add nullable document date in both databases, API editing, filtering, and sort order. Existing rows remain null. | 14 | Null dates stay distinct from arrival dates; date boundaries and null sorting match across databases. |
| 26 | Expose document date editing, date-kind filters, and column choice in Apple views. | 22, 25 | Date edits do not change Recently added order; saved views preserve date kind and null semantics. |
| 27 | Add filtered semantic retrieval in both search adapters with an explicit stable continuation contract. Preserve text-only fallback. | 06; processing/search model contract | Filters apply before limiting/ranking; no cross-vault hits; model unavailable/stale cases work; continuation does not silently drop lower-ranked matching documents. |
| 28 | Integrate semantic mode into Apple search and saved queries, with consistent summary metadata and clear fallback state. | 22, 27 | Search mode persists; filters and tags survive search; fallback is visible and does not clear the query. |
| 29 | Close measured scale and accessibility gaps; verify end-to-end workflows on isolated fixtures. Split actual fixes by cause. | 24, 26, 28 | Large-library retrieval, queue review, taxonomy repair, and saved-view navigation pass; VoiceOver and keyboard paths work at narrow and large-text sizes. |

Milestone exit: document dates and semantic retrieval work with the same library model. The final PR is an audit and measured fixes, not a place to defer tests from earlier PRs.

## Migration and compatibility sequence

Add new numbered migrations to both `migrations/sqlite/` and `migrations/postgres/`. Allocate sequence numbers at implementation time because other changes are active. Planned suffixes are `document_review`, `document_metadata_revision`, `saved_views`, and `document_date`; query indexes belong in the relevant query migration.

Deploy additive backend fields before requiring them in clients. Update `openapi/openapi.json` and `apple/FolioKit/Sources/FolioAPI/openapi.json` together; the Swift client is generated by the package plugin. Use capability negotiation or a documented minimum server version for each new feature. Do not treat an arbitrary 404 as proof of an old server, since it can mean inaccessible data. Keep old form routes for web compatibility. Do not change existing MCP tool response contracts accidentally.

The new access route base is a deliberate dependency. Rebase route PRs onto that work once its URLs and authorization are settled. The visual shell can land independently against current services. A future vault picker belongs with the existing vault effort, but browser queries, saved views, pins, and caches must carry vault identity from the start.

Roll back the client or hide a capability without dropping stored review decisions or saved views. Do not automatically reverse data migrations. Prove restoration from a throwaway database backup during migration verification.

## Verification and release gates

Write behavior tests before implementation where practical. Extend the shared repository contract suite for SQLite and PostgreSQL; require container-backed tests in CI with `FOLIO_REQUIRE_DOCKER=1` so unavailable infrastructure cannot count as coverage. Exercise route authorization and API errors in `crates/folio-api/tests`, and update OpenAPI snapshot checks for every contract change.

Use the project `.agents/skills/verify/SKILL.md` for Apple package, macOS, and File Provider checks. Add isolated UI fixtures before the first visual PR so screenshots and keyboard checks never open the user's signed-in archive. Include iOS Simulator builds for shared SwiftUI changes. Update the verify skill with the new test commands when they exist.

For scale, seed an isolated library with at least 10,000 documents and 1,000 taxonomy entries, repeated dates, missing assignments, failed processing, and long names. Check bounded page sizes and thumbnail concurrency, query plans for indexes, cancellation, and memory as pages accumulate. Record timing and memory on a named fixture and machine; choose numeric budgets from that baseline rather than inventing latency promises now.

Before each PR, run relevant tests and formatting, the project verify workflow, and required `make format`, `make lint`, and `make test` checks. Use `RUSTC_WRAPPER=` when the local sccache service is unavailable. Generate OpenAPI with `make openapi`. Validate changed OpenSpec artifacts with `openspec validate <change-name> --strict`. Capture before/after evidence for visible changes using isolated test data.

Each PR description uses the repository's Problem, Approach, Tests, Evidence, Security Impact, and Risk format, with observed results. Mark external dependencies explicitly. Merge only after required CI and review gates pass. Archive an OpenSpec change with the final implementation slice it governs, not while dependent behavior is still missing.

## Delivery order and boundaries

Start with 01, then run the library backend sequence 02–03 alongside the independent shell 04. Converge at 05, add 06–07, and release milestone A. Keep subsequent stacks short instead of opening a 29-layer chain. Milestones B and C can be prepared independently after A, but changes to shared view models need a single integration owner.

The existing local sidebar-width and row-wrapping edits are a separate bug-fix candidate. Review their diff and verification failures independently; do not hide them in the planning or workspace-shell PR.

Deferred scope: a full web redesign, shared saved views, folder hierarchy, automatic document-date extraction, select-all matching results, server-side bulk export jobs, and new processing infrastructure. None is required to ship the approved workspace. Review these defaults at PR 01, especially legacy review backfill, private saved views, and text-first search delivery.

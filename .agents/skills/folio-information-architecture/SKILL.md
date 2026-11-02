---
name: folio-information-architecture
description: Apply Folio's information architecture and UX hierarchy when building or changing screens, navigation, visual prominence, document browsing, search and filters, taxonomy, saved views, review flows, or vault selection. Use for Apple and web UI work and backend contracts that shape these interactions. Excludes infrastructure-only changes and cosmetic styling that leaves hierarchy unchanged.
---

# Folio information architecture

Keep finding and filing documents central. Place new features within the existing document and vault model before adding destinations.

## Preserve the desired UX hierarchy

This is the product's desired hierarchy for Apple and web interfaces. Apply it when changing the relevant UI; it does not claim that every current screen already follows it.

| Level               | Content                              | Treatment                                                                                   |
| ------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------- |
| Context at the top  | Selected vault                       | Establish which library the user is browsing before showing its destinations.               |
| Main destinations   | Inbox, All documents                 | Give these the strongest navigation prominence beneath the vault.                           |
| Browse documents    | Tags, Correspondents, Document types | Group these as ways into filtered documents, subordinate to the main destinations.          |
| Secondary utilities | Settings and management              | Keep these discoverable but visually separated and less prominent than document navigation. |

Opening a tag, correspondent, or document type leads to its documents. Put editing and managing that classification behind an explicit action. Browsing must not require entering management first.

Preserve this relative order and prominence in compact layouts as well as sidebars. Settings and management must not displace Inbox or All documents from primary navigation. The utility group does not imply that all settings belong to the selected vault; retain each setting's actual scope.

Place additional views, including Recently added and Saved views, without giving them equal primary prominence by default. The two main destinations remain Inbox and All documents. When a source plan lists more destinations, use it for behavior and use this hierarchy for their prominence.

## Establish the relevant contract

Read the sources needed for the requested change. Paths below are relative to this skill.

- [Design system](../../../design/README.md) defines product voice, terminology, and visual hierarchy.
- [Document workspace plan](../../../docs/document-workspace-plan.md), especially Product rules, defines the intended Apple navigation, query behavior, review, taxonomy, and saved views.
- [Workspace specification](../../../openspec/changes/document-workspace/specs/document-workspace/spec.md) supplies acceptance scenarios for that work.
- [Vault library specification](../../../openspec/changes/vault-library-switching/specs/vault-library/spec.md) defines library selection, independent upload destinations, and Files and Finder organization.
- [Trash specification](../../../openspec/specs/document-trash/spec.md) defines document removal and recovery.

Check implementation before claiming a capability exists. The workspace plan describes incremental delivery, including unfinished features. Inspect [WorkspaceViewModel.swift](../../../apple/FolioKit/Sources/FolioKit/ViewModels/WorkspaceViewModel.swift) and [WorkspaceView.swift](../../../apple/Folio/Sources/WorkspaceView.swift) for Apple navigation. Inspect [web templates](../../../crates/folio-api/templates) and [page routes](../../../crates/folio-api/src/routes/pages.rs) for web behavior.

If an active change has been archived, find its successor under `openspec/specs` or `openspec/changes/archive`. Resolve disagreements between plans, specifications, and implementation explicitly. Do not silently treat a proposal as shipped behavior. The Apple workspace plan does not authorize a web redesign.

## Place content and actions

Use these decisions from the linked plans when implementing their scope:

- The selected vault contains the library. Show it above library destinations. One accessible vault needs its name, without a redundant selector.
- The workspace plan names Library, Browse by, and Saved views groups for Apple. Apply the desired UX hierarchy above when placing these groups and their entries. Add destinations as their backing behavior becomes available.
- Inbox is for attention and review. All documents is the archive. Recently added orders by arrival time and groups by day, without a hidden time cutoff.
- A taxonomy entry opens filtered documents. Make managing the entry a separate explicit action. Renaming preserves identity; removing a classification does not delete its documents.
- Saved views store a named, versioned query and preferred presentation. Their initial ownership is one user within one vault. Pins, scroll position, selection, and window layout are device-local state.
- Keep upload, supported scan actions, settings, and account access discoverable. Upload destination is independent of the library being browsed and is captured when the upload starts.
- Use the existing trash contract for removal and recovery. Do not conflate removing a tag, moving a document to Trash, and permanently deleting it.

## Preserve the user's context

- List and grid present the same query and selection. They are not separate libraries.
- Search starts within the current destination. An explicit Search all documents action stays within the selected vault.
- Keep scope, search, filters, sort, and presentation visible and preserve them through detail navigation. Narrow layouts return from detail to the previous results.
- Distinguish document date from arrival time. Do not substitute upload time for an unknown document date.
- Clear stale results and selection when account, server, or vault changes. A late response from the old context must not populate the new library.
- Do not broaden a query silently when a saved filter is invalid or unsupported. Follow the query contract for errors and recovery.

## Keep states and labels distinct

The intended workspace review model separates a person's review decision from processing and metadata completeness. Imports remain unreviewed until an explicit review action, even if automation classifies them. Inbox membership is unreviewed OR processing active OR processing failed. Its groups prioritize failed, then active processing, then ready to review.

Check whether the requested implementation has the explicit review contract before applying that model. Do not infer a completed human review from populated metadata. An embedding failure is a search-indexing problem, not proof that a document is unreadable.

Use the design system's field labels: From for correspondent, Type for document type, and Archive no. for ASN. Preserve directory names such as Correspondents where the navigation plan uses them. Present suggestions as suggestions and status as words, not color alone.

## Check the changed journey

Before implementation, name the entry point, vault scope, destination or action, and return path for the requested feature. Reuse an existing destination when it serves that journey. Raise unresolved product choices before inventing a new hierarchy.

For navigation changes, check that the vault comes first, Inbox and All documents remain primary, taxonomy opens documents, and settings and management remain secondary. Check the same hierarchy in the affected compact layout.

Verify the affected journey with realistic data and the project's verify skill. Relevant cases include opening a taxonomy result, searching within a destination, returning from detail, switching vaults during a pending request, and uploading to another vault. Check empty results and unavailable access where affected. Match verification to the change; this skill does not require a full navigation audit for every edit.

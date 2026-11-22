## Why

An open filtered list can show unrelated uploads, retain rows that stop matching, or miss rows that start matching after a document changes.

## What Changes

- Refresh open Inbox, All documents, filtered, and saved-view results using the current vault and page criteria when document events arrive.
- Keep row membership, empty states, and vault counts aligned while preserving open editors and the current URL.

## Capabilities

### Modified Capabilities

- `realtime-events`: Browser document lists reconcile against their active view after a document change.

## Impact

Affects server-rendered web lists and their browser stream handling. The event API and saved-view query contract remain unchanged.

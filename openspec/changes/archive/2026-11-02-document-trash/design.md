# Design

The document row carries `deleted_at` and `purge_requested_at`. Ordinary queries and mutations exclude trashed rows; dedicated repository operations list, restore, request purge, and finalize purge. Trash preserves original files, derived files, metadata, tags, and search data. Restoring makes the document visible again and resumes interrupted processing.

A purge request requires an already trashed document and records its intent and job in one transaction. Restore conflicts after that request. The worker removes only the canonical document storage prefix and deletes the row after storage succeeds. Failed work is retryable. Repair schedules only explicit purge intents, never documents selected by age. Processing work skips trashed documents; purge waits for active processing to stop.

Trashed and Restored are explicit outbox events. Deleted describes final permanent deletion. Web lists and Trash react to these events. Authorization uses the selected vault and document deletion permissions for trash, restore, and purge.

The JSON API adds GET `/api/documents/trash`, POST `/api/documents/{id}/restore`, and DELETE `/api/documents/{id}/purge`. DELETE `/api/documents/{id}` moves to Trash. Regenerate `openapi/openapi.json` and the Apple client schema copy. Existing generated delete calls retain their signature and now trash documents.

HTML routes are GET `/trash`, POST `/documents/{id}/restore`, and POST `/documents/{id}/delete-forever`. Existing delete forms move to Trash. Permanent deletion requires a separate confirmation and pending rows expose no restore or delete action. Use existing visual tokens and list controls.

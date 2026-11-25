## 1. Producers and persistence

- [x] Carry a stable, opaque item key through folder and email polling (#269).
- [x] Add SQLite and PostgreSQL outcome and source health tables. Commit the outcome, fenced ledger transition, and shared outbox event together (#293).
- [x] Wire observed ingestion and scheduler health through the merged repository. Verify polling replay, failed and cancelled attempts, outcome commit recovery, synchronous veto, and sanitized payloads (#309).
- [x] Recheck managed source trash in the same transaction as claims, completion, and health changes. Verify the shared scope contract on SQLite and PostgreSQL (#502).

## 2. Source activity

- [x] Expose bounded, vault-scoped outcome history in JSON and web views. Regenerate OpenAPI and verify scope, pagination, and inaccessible-document links (#309).
- [x] Connect the Apple source list and activity view to the source and generated outcome APIs. Verify loading, retry, pagination, and document navigation (#316).

## 3. Workflow integration

- [x] Use the shared domain envelope and outbox (#265 and #266) and typed trigger registry (#280).
- [x] Define all six producer triggers and verify their condition fields and allowed actions with the shared registry (#523).
- [x] Consume the six owner-only outcome and health events through the shared workflow engine. Recheck current authorization and source lifecycle, use the captured event data, and deduplicate replay without collapsing distinct attempts (#536).
- [x] Expose the six triggers and their bounded conditions in the web and Apple builders. Reject SkipImport and document actions for these triggers (#536, #572, and #623).
- [x] Verify producer-to-workflow delivery, retry, replay, lineage, and authorization, then merge through required checks (#536).

## 4. Delivery

- [x] Verify the new migrations on SQLite and PostgreSQL, including upgrades with existing ledger rows and no historical backfill (#293).
- [x] Verify producer and UI PRs #309 and #316 merged with server, end-to-end, and Apple CI passing.
- [x] Verify retained history and lifecycle authorization, then archive the completed outcome requirements (#502 and #518).

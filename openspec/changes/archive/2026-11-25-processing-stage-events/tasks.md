## 1. Contracts

- [x] Use the shared domain event envelope, workflow trigger catalog, and revision dispatch contracts. Register all 15 processing outcomes with bounded condition fields (#265, #499, #533, and #625).
- [x] Preserve enqueue-time vault and lineage. Restore system attribution and reject moved work before execution (#288).

## 2. Atomic processing

- [x] Persist stage runs and commit OCR, embedding, and thumbnail transitions with their outbox events. Verify rollback, duplicate completion, stale claims, and lease recovery on SQLite and PostgreSQL (#274, #307, #517, and #534).
- [x] Distinguish retry, terminal failure, and skip. Cover disabled providers, empty OCR, unsupported thumbnails, and incomplete embedding batches (#268, #270, and #517).
- [x] Bind dispatch, results, and reads to the current revision. Exercise replacement, restore, receipt replay, and stale processing on SQLite, PostgreSQL, and Chromium (#530, #534, #540, #544, #558, and #625).

## 3. Workflows and clients

- [x] Verify OCR producer-to-workflow delivery, immutable event context, origin suppression, replay deduplication, and action authorization on SQLite and PostgreSQL (#541 and #579).
- [x] Expose read-only current stage summaries in JSON, HTML, OpenAPI, and Apple. Verify retry, terminal failure, skip, and preview fencing (#281, #509, #524, and #534).
- [x] Expose the processing trigger catalog and typed filters in both workflow builders. Verify root and nested editing in Chromium (#572, #598, #617, and #620).
- [x] Verify typed root and nested filter save and reopen on an isolated iPhone 18 Pro simulator with Xcode 27.2. Verify the tested fixture and test on main (#630, head 5bb44436, merge 0084e182).

## 4. Delivery

- [x] Verify formatting, lint, and workspace tests on main after revision dispatch activation. Confirm the publication and processing contracts on both databases (CI run 45515545954).
- [x] Verify current revision stage IDs after replacement and restoration and root and nested processing filter editing in hosted Chromium (main CI run 45515545954). The separate browser fixture repairs are tracked in #631.
- [x] Validate and archive the completed processing requirements.

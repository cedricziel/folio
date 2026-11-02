## 1. Contracts
- [ ] Reconcile this design with merged foundation, revision, and workflow contracts.

## 2. Atomic processing
- [x] Commit OCR state and legacy completion events atomically; verify `ocr_completion_contract` on both databases and `retry_after_empty_ocr_completion_does_not_repeat_ocr_or_its_event`. Merged in PR #274, commit 3aa4d1f.
- [x] Reject incomplete or malformed embedding batches before storage/completion; prove with `missing_embedding_vectors_never_publish_completion` and `embedding_batch`. Merged in PR #268, commit b40a353.
- [x] Add attempt transition model after failing `processing_attempts` tests for retry, terminal outcomes, stale attempts, duplicate completion, persisted-state restoration, and counter exhaustion. Core suite passes; merged in PR #270, commit 2353a4d. Database integration remains pending.
- [ ] Write failing shared repository tests for rollback, duplicate completion, lease expiry, stale claims, revision supersession, trash, and vault move; implement fenced transitions in both databases.
- [ ] Write failing worker tests for retry versus terminal failure, disabled providers, empty OCR, and thumbnail skips; replace split state/event writes.

## 3. Workflow and clients
- [ ] In progress: expose existing OCR and embedding status in web and Apple detail, distinguishing empty completion, skipped work, and last-attempt failure; verify `ProcessingSummaryTests`, the document-detail router regression, and the document lifecycle UI test.
- [ ] Write failing workflow tests for selected stage outcomes, deduplication, immutable context, and revoked authorization; register triggers in the shared registry.
- [ ] Write failing API and Apple adapter tests for stage summaries; update JSON/HTML detail, OpenAPI, Swift client, and native stage state/error UI.
- [ ] Verify web and native trigger selection and stage state behavior using isolated fixtures and shared tokens.

## 4. Ship
- [ ] Run project verify, make format, make lint, make test, and changed OpenSpec validation.
- [ ] Archive completed specs, open reviewable PRs, resolve CI/review/conflicts, and verify every PR merged.
- [ ] Disable monitoring and archive this chat only after merged completion.

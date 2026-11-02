## 1. Contracts
- [ ] Reconcile this design with merged foundation, revision, and workflow contracts.

## 2. Atomic processing
- [x] Reject incomplete or malformed embedding batches before storage/completion; prove with `missing_embedding_vectors_never_publish_completion` and `embedding_batch`. Merged in PR #268, commit b40a353.
- [x] Add attempt transition model after failing `processing_attempts` tests for retry, terminal outcomes, stale attempts, duplicate completion, persisted-state restoration, and counter exhaustion. Core suite passes; database integration remains pending.
- [ ] In progress: write failing shared repository tests for rollback, duplicate completion, lease expiry, stale claims, revision supersession, trash, and vault move; implement fenced transitions in both databases.
- [ ] Write failing worker tests for retry versus terminal failure, disabled providers, empty OCR, and thumbnail skips; replace split state/event writes.

## 3. Workflow and clients
- [ ] Write failing workflow tests for selected stage outcomes, deduplication, immutable context, and revoked authorization; register triggers in the shared registry.
- [ ] Write failing API and Apple adapter tests for stage summaries; update JSON/HTML detail, OpenAPI, Swift client, and native stage state/error UI.
- [ ] Verify web and native trigger selection and stage state behavior using isolated fixtures and shared tokens.

## 4. Ship
- [ ] Run project verify, make format, make lint, make test, and changed OpenSpec validation.
- [ ] Archive completed specs, open reviewable PRs, resolve CI/review/conflicts, and verify every PR merged.
- [ ] Disable monitoring and archive this chat only after merged completion.

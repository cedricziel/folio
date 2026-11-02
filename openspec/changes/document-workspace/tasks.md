## 1. Contracts
- [x] 1.1 Record the approved scope, delivery plan, compatibility policy, and dependency boundaries.
- [x] 1.2 Validate the document-workspace change.

## 2. Paged repository queries
- [x] 2.1 Add failing contract tests for bounded pages, tied sort values, filters, and query-bound continuations.
- [x] 2.2 Add typed page/query models and the repository port while supporting the current list routes.
- [x] 2.3 Implement SQLite and PostgreSQL keyset queries and run shared contracts.

## 3. Library delivery
- [x] 3.0 Batch document tag enrichment with shared vault-isolation and duplicate-ID regression tests.
- [x] 3.1 Implement authorized paged JSON API and default-vault adapter with access-revocation, pagination, and filter regression tests.
- [x] 3.2 Implement native workspace navigation with failing navigation/restoration tests first.
- [ ] 3.3 Implement incremental loading, cancellation, and query composition with failing view-model tests first.
- [x] 3.4a Expose literal text search combined with paged filters and query-bound continuations.
- [ ] 3.4b Implement native text search and filter controls.

## 4. Subsequent milestones
- [ ] 4.1 Expand executable tasks for milestones B–F from the implementation plan before each milestone starts.

## 5. Verification
- [ ] 5.1 Run scoped tests, project verify checks, and strict OpenSpec validation for each slice.
- [ ] 5.2 Run make format, make lint, and make test before shipping implementation PRs.

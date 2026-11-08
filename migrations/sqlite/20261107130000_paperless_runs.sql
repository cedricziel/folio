-- Paperless import runs, their items, the sync ledger, the taxonomy map and
-- the per-vault sync state. Timestamps are RFC 3339 UTC with a fixed number of
-- fraction digits, so text comparison orders them.

CREATE TABLE import_runs (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('paperless_export', 'paperless_source')),
    "trigger" TEXT NOT NULL CHECK ("trigger" IN ('manual', 'schedule', 'initial')),
    -- Configured sources have no ingestion_sources row, so no foreign key.
    source_id TEXT,
    source_path TEXT,
    dry_run INTEGER NOT NULL DEFAULT 0 CHECK (dry_run IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'completed_with_errors', 'failed', 'cancelled')),
    cursor_after TEXT,
    confirm_trash INTEGER NOT NULL DEFAULT 0 CHECK (confirm_trash IN (0, 1)),
    plan_complete INTEGER NOT NULL DEFAULT 0 CHECK (plan_complete IN (0, 1)),
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    driver_restarts INTEGER NOT NULL DEFAULT 0 CHECK (driver_restarts >= 0),
    params TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(params)),
    summary TEXT CHECK (summary IS NULL OR json_valid(summary)),
    error TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);
CREATE UNIQUE INDEX import_runs_one_active ON import_runs(vault_id) WHERE status IN ('queued', 'running');
CREATE INDEX import_runs_vault_created ON import_runs(vault_id, created_at DESC, id DESC);

CREATE TABLE import_items (
    run_id TEXT NOT NULL REFERENCES import_runs(id) ON DELETE CASCADE,
    paperless_id INTEGER NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('create', 'update', 'trash', 'restore')),
    document_id TEXT,
    title TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'in_progress', 'done', 'skipped', 'failed', 'would_create', 'would_update', 'would_trash', 'would_restore')),
    reason TEXT,
    warnings TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(warnings)),
    payload TEXT CHECK (payload IS NULL OR json_valid(payload)),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    claim TEXT,
    claimed_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (run_id, paperless_id)
);
CREATE INDEX import_items_run_status ON import_items(run_id, status, paperless_id);
-- Claims skip finished items without reading them.
CREATE INDEX import_items_claimable ON import_items(run_id, paperless_id)
    WHERE status IN ('pending', 'in_progress');

CREATE TABLE import_staging (
    run_id TEXT NOT NULL REFERENCES import_runs(id) ON DELETE CASCADE,
    paperless_document_id INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('note', 'custom_field_instance')),
    record TEXT NOT NULL CHECK (json_valid(record))
);
CREATE INDEX import_staging_document ON import_staging(run_id, paperless_document_id, kind);

-- The sync ledger. No foreign key to documents: a purged document keeps its
-- row as `purged_in_folio`, so sync never re-creates it.
CREATE TABLE paperless_documents (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    paperless_id INTEGER NOT NULL,
    document_id TEXT NOT NULL,
    paperless_modified TEXT NOT NULL,
    checksum TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    needs_reapply INTEGER NOT NULL DEFAULT 0 CHECK (needs_reapply IN (0, 1)),
    trashed_in_paperless INTEGER NOT NULL DEFAULT 0 CHECK (trashed_in_paperless IN (0, 1)),
    folio_state TEXT NOT NULL DEFAULT 'live' CHECK (folio_state IN ('live', 'trashed_by_user', 'purged_in_folio')),
    last_synced_at TEXT NOT NULL,
    last_run_id TEXT,
    PRIMARY KEY (vault_id, paperless_id),
    UNIQUE (vault_id, document_id)
);

CREATE TABLE paperless_taxonomy (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('tag', 'correspondent', 'document_type', 'custom_field')),
    paperless_id INTEGER NOT NULL,
    entity_id TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (vault_id, kind, paperless_id)
);

CREATE TABLE paperless_sync_state (
    vault_id TEXT PRIMARY KEY REFERENCES vaults(id) ON DELETE CASCADE,
    source_id TEXT,
    library_sample TEXT CHECK (library_sample IS NULL OR json_valid(library_sample)),
    cursor_modified TEXT,
    last_poll_at TEXT,
    last_full_reconcile_at TEXT,
    once_done INTEGER NOT NULL DEFAULT 0 CHECK (once_done IN (0, 1)),
    retry_ids TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(retry_ids)),
    parked_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(parked_ids)),
    updated_at TEXT NOT NULL
);

-- SQLite can't alter a CHECK, so ingest_items and source_health are rebuilt.
-- ingest_outcomes references ingest_items with ON DELETE CASCADE, and foreign
-- keys stay on inside the migration transaction, so dropping ingest_items
-- would empty it; it is rebuilt alongside and dropped first.
CREATE TABLE ingest_items_new (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    source_managed INTEGER NOT NULL CHECK (source_managed IN (0, 1)),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('folder', 'email', 'paperless')),
    locator_key TEXT NOT NULL CHECK (length(locator_key) = 64),
    attempt_id TEXT NOT NULL UNIQUE,
    started_at TEXT NOT NULL,
    lease_expires_at TEXT NOT NULL,
    decision_fingerprint TEXT NOT NULL CHECK (length(decision_fingerprint) = 64),
    state TEXT NOT NULL CHECK (state IN ('processing', 'imported', 'skipped', 'duplicate', 'failed')),
    UNIQUE (vault_id, source_id, locator_key),
    UNIQUE (vault_id, source_id, id)
);
INSERT INTO ingest_items_new SELECT id, vault_id, source_id, source_managed, source_kind, locator_key,
       attempt_id, started_at, lease_expires_at, decision_fingerprint, state
  FROM ingest_items;

CREATE TABLE ingest_outcomes_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vault_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL UNIQUE,
    event_id TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('imported', 'skipped', 'duplicate', 'failed')),
    -- Keep the historical identity after purge; reads must recheck current access.
    document_id TEXT,
    CHECK ((outcome = 'imported' AND document_id IS NOT NULL)
        OR (outcome != 'imported' AND document_id IS NULL)),
    FOREIGN KEY (vault_id, source_id, item_id)
        REFERENCES ingest_items_new(vault_id, source_id, id) ON DELETE CASCADE
);
INSERT INTO ingest_outcomes_new SELECT id, vault_id, source_id, item_id, attempt_id, event_id,
       occurred_at, outcome, document_id
  FROM ingest_outcomes;

DROP TABLE ingest_outcomes;
DROP TABLE ingest_items;
-- Renaming also rewrites the reference in ingest_outcomes_new.
ALTER TABLE ingest_items_new RENAME TO ingest_items;
ALTER TABLE ingest_outcomes_new RENAME TO ingest_outcomes;
CREATE INDEX idx_ingest_outcomes_source ON ingest_outcomes(vault_id, source_id, id DESC);

CREATE TABLE source_health_new (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    source_managed INTEGER NOT NULL CHECK (source_managed IN (0, 1)),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('folder', 'email', 'paperless')),
    failed INTEGER NOT NULL CHECK (failed IN (0, 1)),
    PRIMARY KEY (vault_id, source_id)
);
INSERT INTO source_health_new SELECT vault_id, source_id, source_managed, source_kind, failed
  FROM source_health;
DROP TABLE source_health;
ALTER TABLE source_health_new RENAME TO source_health;

CREATE UNIQUE INDEX ingestion_sources_one_paperless ON ingestion_sources(vault_id)
    WHERE kind = 'paperless' AND deleted_at IS NULL;

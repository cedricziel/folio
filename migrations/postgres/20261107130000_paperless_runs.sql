-- Paperless import runs, their items, the sync ledger, the taxonomy map and
-- the per-vault sync state.

CREATE TABLE import_runs (
    id UUID PRIMARY KEY,
    vault_id UUID NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('paperless_export', 'paperless_source')),
    "trigger" TEXT NOT NULL CHECK ("trigger" IN ('manual', 'schedule', 'initial')),
    -- Configured sources have no ingestion_sources row, so no foreign key.
    source_id UUID,
    source_path TEXT,
    dry_run BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'completed_with_errors', 'failed', 'cancelled')),
    cursor_after TIMESTAMPTZ,
    confirm_trash BOOLEAN NOT NULL DEFAULT FALSE,
    plan_complete BOOLEAN NOT NULL DEFAULT FALSE,
    cancel_requested BOOLEAN NOT NULL DEFAULT FALSE,
    driver_restarts INTEGER NOT NULL DEFAULT 0 CHECK (driver_restarts >= 0),
    params JSONB NOT NULL DEFAULT '{}',
    summary JSONB,
    error TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX import_runs_one_active ON import_runs(vault_id) WHERE status IN ('queued', 'running');
CREATE INDEX import_runs_vault_created ON import_runs(vault_id, created_at DESC, id DESC);

CREATE TABLE import_items (
    run_id UUID NOT NULL REFERENCES import_runs(id) ON DELETE CASCADE,
    paperless_id BIGINT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('create', 'update', 'trash', 'restore')),
    document_id UUID,
    title TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'in_progress', 'done', 'skipped', 'failed', 'would_create', 'would_update', 'would_trash', 'would_restore')),
    reason TEXT,
    warnings JSONB NOT NULL DEFAULT '[]',
    payload JSONB,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    claim UUID,
    claimed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (run_id, paperless_id)
);
CREATE INDEX import_items_run_status ON import_items(run_id, status, paperless_id);
-- Claims skip finished items without reading them.
CREATE INDEX import_items_claimable ON import_items(run_id, paperless_id)
    WHERE status IN ('pending', 'in_progress');

CREATE TABLE import_staging (
    run_id UUID NOT NULL REFERENCES import_runs(id) ON DELETE CASCADE,
    paperless_document_id BIGINT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('note', 'custom_field_instance')),
    record JSONB NOT NULL
);
CREATE INDEX import_staging_document ON import_staging(run_id, paperless_document_id, kind);

-- The sync ledger. No foreign key to documents: a purged document keeps its
-- row as `purged_in_folio`, so sync never re-creates it.
CREATE TABLE paperless_documents (
    vault_id UUID NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    paperless_id BIGINT NOT NULL,
    document_id UUID NOT NULL,
    paperless_modified TIMESTAMPTZ NOT NULL,
    checksum TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    needs_reapply BOOLEAN NOT NULL DEFAULT FALSE,
    trashed_in_paperless BOOLEAN NOT NULL DEFAULT FALSE,
    folio_state TEXT NOT NULL DEFAULT 'live' CHECK (folio_state IN ('live', 'trashed_by_user', 'purged_in_folio')),
    last_synced_at TIMESTAMPTZ NOT NULL,
    last_run_id UUID,
    PRIMARY KEY (vault_id, paperless_id),
    UNIQUE (vault_id, document_id)
);

CREATE TABLE paperless_taxonomy (
    vault_id UUID NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('tag', 'correspondent', 'document_type', 'custom_field')),
    paperless_id BIGINT NOT NULL,
    entity_id UUID NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (vault_id, kind, paperless_id)
);

CREATE TABLE paperless_sync_state (
    vault_id UUID PRIMARY KEY REFERENCES vaults(id) ON DELETE CASCADE,
    source_id UUID,
    library_sample JSONB,
    cursor_modified TIMESTAMPTZ,
    last_poll_at TIMESTAMPTZ,
    last_full_reconcile_at TIMESTAMPTZ,
    once_done BOOLEAN NOT NULL DEFAULT FALSE,
    retry_ids JSONB NOT NULL DEFAULT '{}',
    parked_ids JSONB NOT NULL DEFAULT '[]',
    updated_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE ingest_items DROP CONSTRAINT ingest_items_source_kind_check;
ALTER TABLE ingest_items ADD CONSTRAINT ingest_items_source_kind_check
    CHECK (source_kind IN ('folder', 'email', 'paperless'));
ALTER TABLE source_health DROP CONSTRAINT source_health_source_kind_check;
ALTER TABLE source_health ADD CONSTRAINT source_health_source_kind_check
    CHECK (source_kind IN ('folder', 'email', 'paperless'));

CREATE UNIQUE INDEX ingestion_sources_one_paperless ON ingestion_sources(vault_id)
    WHERE kind = 'paperless' AND deleted_at IS NULL;

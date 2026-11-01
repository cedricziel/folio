-- Automatic ingestion: where documents come from and what was already imported (PostgreSQL)

ALTER TABLE documents ADD COLUMN source_type TEXT NOT NULL DEFAULT 'upload';
ALTER TABLE documents ADD COLUMN source_meta TEXT;

CREATE INDEX idx_documents_source ON documents(source_type);

-- Sources created in the UI; sources from the configuration are not stored here.
CREATE TABLE ingestion_sources (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    kind TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

-- Keyed by source id, for managed and configured sources alike.
CREATE TABLE ingestion_status (
    source_id UUID PRIMARY KEY,
    last_run_at TIMESTAMPTZ,
    last_error TEXT,
    last_imported INTEGER NOT NULL DEFAULT 0,
    last_skipped INTEGER NOT NULL DEFAULT 0,
    last_failed INTEGER NOT NULL DEFAULT 0
);

-- One row per handled content hash or mail message; inserts use ON CONFLICT DO NOTHING.
CREATE TABLE ingest_ledger (
    key TEXT PRIMARY KEY,
    source_id UUID NOT NULL,
    document_id UUID,
    outcome TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

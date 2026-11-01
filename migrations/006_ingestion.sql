-- Automatic ingestion: where documents come from and what was already imported

ALTER TABLE documents ADD COLUMN source_type TEXT NOT NULL DEFAULT 'upload';
ALTER TABLE documents ADD COLUMN source_meta TEXT;

CREATE INDEX idx_documents_source ON documents(source_type);

-- Sources created in the UI; sources from the configuration are not stored here.
CREATE TABLE ingestion_sources (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Keyed by source id, for managed and configured sources alike.
CREATE TABLE ingestion_status (
    source_id TEXT PRIMARY KEY,
    last_run_at TEXT,
    last_error TEXT,
    last_imported INTEGER NOT NULL DEFAULT 0,
    last_skipped INTEGER NOT NULL DEFAULT 0,
    last_failed INTEGER NOT NULL DEFAULT 0
);

-- One row per handled content hash or mail message; inserts use ON CONFLICT DO NOTHING.
CREATE TABLE ingest_ledger (
    key TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    document_id TEXT,
    outcome TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE ingestion_sources_lifecycle (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    created_by TEXT,
    updated_by TEXT
);
INSERT INTO ingestion_sources_lifecycle (id, vault_id, name, enabled, kind, config, created_at, updated_at)
SELECT id, vault_id, name, enabled, kind, config, created_at, updated_at FROM ingestion_sources;
DROP TABLE ingestion_sources;
ALTER TABLE ingestion_sources_lifecycle RENAME TO ingestion_sources;
CREATE UNIQUE INDEX ingestion_sources_live_name ON ingestion_sources(vault_id, name) WHERE deleted_at IS NULL;

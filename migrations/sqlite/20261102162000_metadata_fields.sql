CREATE TABLE metadata_fields (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'long_text', 'boolean', 'integer', 'decimal', 'date', 'money', 'select', 'url', 'document_link')),
    is_required BOOLEAN NOT NULL DEFAULT FALSE,
    is_unique BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    created_by TEXT,
    updated_by TEXT,
    UNIQUE (vault_id, id),
    UNIQUE (vault_id, id, kind)
);
CREATE UNIQUE INDEX metadata_fields_live_name ON metadata_fields(vault_id, name_key) WHERE deleted_at IS NULL;

CREATE TABLE metadata_options (
    id TEXT PRIMARY KEY NOT NULL,
    vault_id TEXT NOT NULL,
    field_id TEXT NOT NULL,
    field_kind TEXT NOT NULL DEFAULT 'select' CHECK (field_kind = 'select'),
    label TEXT NOT NULL,
    label_key TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position BETWEEN 0 AND 4294967295),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    created_by TEXT,
    updated_by TEXT,
    UNIQUE (vault_id, field_id, id),
    FOREIGN KEY (vault_id, field_id, field_kind) REFERENCES metadata_fields(vault_id, id, kind) ON DELETE CASCADE
);
CREATE UNIQUE INDEX metadata_options_live_label ON metadata_options(vault_id, field_id, label_key) WHERE deleted_at IS NULL;

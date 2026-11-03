CREATE TABLE source_health (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    source_managed INTEGER NOT NULL CHECK (source_managed IN (0, 1)),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('folder', 'email')),
    failed INTEGER NOT NULL CHECK (failed IN (0, 1)),
    PRIMARY KEY (vault_id, source_id)
);

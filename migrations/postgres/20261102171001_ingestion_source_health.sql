CREATE TABLE source_health (
    vault_id UUID NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    source_id UUID NOT NULL,
    source_managed BOOLEAN NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('folder', 'email')),
    failed BOOLEAN NOT NULL,
    PRIMARY KEY (vault_id, source_id)
);

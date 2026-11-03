-- Configured sources have no ingestion_sources row, so source_id has no foreign key.
CREATE TABLE ingest_items (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    source_managed INTEGER NOT NULL CHECK (source_managed IN (0, 1)),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('folder', 'email')),
    locator_key TEXT NOT NULL CHECK (length(locator_key) = 64),
    attempt_id TEXT NOT NULL UNIQUE,
    started_at TEXT NOT NULL,
    lease_expires_at TEXT NOT NULL,
    decision_fingerprint TEXT NOT NULL CHECK (length(decision_fingerprint) = 64),
    state TEXT NOT NULL CHECK (state IN ('processing', 'imported', 'skipped', 'duplicate', 'failed')),
    UNIQUE (vault_id, source_id, locator_key),
    UNIQUE (vault_id, source_id, id)
);

CREATE TABLE ingest_outcomes (
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
        REFERENCES ingest_items(vault_id, source_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_ingest_outcomes_source ON ingest_outcomes(vault_id, source_id, id DESC);

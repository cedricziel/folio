-- no-transaction
-- Vaults: containers for documents, with per-vault roles granted to users and
-- groups. Every document, vocabulary entry, workflow and ingestion source
-- belongs to exactly one vault.
--
-- SQLite cannot drop a UNIQUE constraint, so the vocabulary tables are rebuilt.
-- Dropping a table that other tables reference runs their ON DELETE actions
-- even with deferred constraints, so foreign keys are switched off for the
-- rebuild (a pragma that is a no-op inside a transaction, hence the
-- `no-transaction` marker above and the explicit BEGIN/COMMIT below).

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE vaults (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

ALTER TABLE users ADD COLUMN default_vault_id TEXT REFERENCES vaults(id) ON DELETE SET NULL;
ALTER TABLE users ADD COLUMN vaults_provisioned INTEGER NOT NULL DEFAULT 0;

CREATE TABLE vault_members (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    principal_type TEXT NOT NULL CHECK (principal_type IN ('user', 'group')),
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    group_id TEXT REFERENCES auth_groups(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'contributor', 'viewer')),
    granted_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    granted_at TEXT NOT NULL,
    CHECK (
        (principal_type = 'user' AND user_id IS NOT NULL AND group_id IS NULL)
        OR (principal_type = 'group' AND group_id IS NOT NULL AND user_id IS NULL)
    )
);

CREATE UNIQUE INDEX vault_members_user_idx ON vault_members (vault_id, user_id)
    WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX vault_members_group_idx ON vault_members (vault_id, group_id)
    WHERE group_id IS NOT NULL;
CREATE INDEX vault_members_user_lookup_idx ON vault_members (user_id);
CREATE INDEX vault_members_group_lookup_idx ON vault_members (group_id);

-- No foreign keys: the history must outlive deleted vaults and users.
CREATE TABLE audit_events (
    id TEXT PRIMARY KEY,
    at TEXT NOT NULL,
    actor_user_id TEXT,
    action TEXT NOT NULL,
    vault_id TEXT,
    target_type TEXT,
    target_id TEXT,
    details TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX audit_events_vault_idx ON audit_events (vault_id, at DESC);

-- Existing data moves into `Archive`, but only when there is any.
INSERT INTO vaults (id, name, created_by, created_at, updated_at)
SELECT
    lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2)
          || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)), 2)
          || '-' || hex(randomblob(6))),
    'Archive',
    (SELECT id FROM users WHERE is_admin = 1 OR is_owner = 1
      ORDER BY is_owner DESC, created_at, id LIMIT 1),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE EXISTS (SELECT 1 FROM documents)
   OR EXISTS (SELECT 1 FROM tags)
   OR EXISTS (SELECT 1 FROM correspondents)
   OR EXISTS (SELECT 1 FROM document_types)
   OR EXISTS (SELECT 1 FROM workflows)
   OR EXISTS (SELECT 1 FROM ingestion_sources)
   OR EXISTS (SELECT 1 FROM ingest_ledger);

INSERT INTO vault_members (vault_id, principal_type, user_id, role, granted_at)
SELECT id, 'user', created_by, 'owner', created_at FROM vaults WHERE created_by IS NOT NULL;

UPDATE users SET default_vault_id = (SELECT id FROM vaults WHERE name = 'Archive')
 WHERE id = (SELECT created_by FROM vaults WHERE name = 'Archive');

-- Documents. The empty default makes a forgotten vault a foreign key error
-- instead of a silent orphan (SQLite cannot add a NOT NULL reference otherwise).
ALTER TABLE documents ADD COLUMN vault_id TEXT NOT NULL DEFAULT '' REFERENCES vaults(id);
ALTER TABLE documents ADD COLUMN uploaded_by TEXT REFERENCES users(id) ON DELETE SET NULL;
UPDATE documents SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
CREATE INDEX idx_documents_vault_created ON documents (vault_id, created_at);

-- Vocabularies: unique per vault instead of per instance.
CREATE TABLE correspondents_new (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (vault_id, name)
);
INSERT INTO correspondents_new (id, vault_id, name, created_at)
SELECT id, (SELECT id FROM vaults WHERE name = 'Archive'), name, created_at FROM correspondents;
DROP TABLE correspondents;
ALTER TABLE correspondents_new RENAME TO correspondents;

CREATE TABLE document_types_new (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (vault_id, name)
);
INSERT INTO document_types_new (id, vault_id, name, created_at)
SELECT id, (SELECT id FROM vaults WHERE name = 'Archive'), name, created_at FROM document_types;
DROP TABLE document_types;
ALTER TABLE document_types_new RENAME TO document_types;

CREATE TABLE tags_new (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    color TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (vault_id, name)
);
INSERT INTO tags_new (id, vault_id, name, color, created_at)
SELECT id, (SELECT id FROM vaults WHERE name = 'Archive'), name, color, created_at FROM tags;
DROP TABLE tags;
ALTER TABLE tags_new RENAME TO tags;

-- Workflows.
ALTER TABLE workflows ADD COLUMN vault_id TEXT NOT NULL DEFAULT '' REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE workflows SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
DROP INDEX idx_workflows_trigger;
CREATE INDEX idx_workflows_trigger ON workflows (vault_id, trigger_event, enabled);

-- Ingestion sources and the ledger of what they imported.
CREATE TABLE ingestion_sources_new (
    id TEXT PRIMARY KEY,
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (vault_id, name)
);
INSERT INTO ingestion_sources_new (id, vault_id, name, enabled, kind, config, created_at, updated_at)
SELECT id, (SELECT id FROM vaults WHERE name = 'Archive'), name, enabled, kind, config, created_at, updated_at
  FROM ingestion_sources;
DROP TABLE ingestion_sources;
ALTER TABLE ingestion_sources_new RENAME TO ingestion_sources;

CREATE TABLE ingest_ledger_new (
    vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    source_id TEXT NOT NULL,
    document_id TEXT,
    outcome TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (vault_id, key)
);
INSERT INTO ingest_ledger_new (vault_id, key, source_id, document_id, outcome, created_at)
SELECT (SELECT id FROM vaults WHERE name = 'Archive'), key, source_id, document_id, outcome, created_at
  FROM ingest_ledger;
DROP TABLE ingest_ledger;
ALTER TABLE ingest_ledger_new RENAME TO ingest_ledger;

COMMIT;

PRAGMA foreign_keys = ON;

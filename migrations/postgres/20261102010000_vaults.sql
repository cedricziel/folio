-- Vaults: containers for documents, with per-vault roles granted to users and
-- groups. Every document, vocabulary entry, workflow and ingestion source
-- belongs to exactly one vault.

CREATE TABLE vaults (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE users ADD COLUMN default_vault_id UUID REFERENCES vaults(id) ON DELETE SET NULL;

CREATE TABLE vault_members (
    vault_id UUID NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    principal_type TEXT NOT NULL CHECK (principal_type IN ('user', 'group')),
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    group_id TEXT REFERENCES auth_groups(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'contributor', 'viewer')),
    granted_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    granted_at TIMESTAMPTZ NOT NULL,
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
    id UUID PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL,
    actor_user_id TEXT,
    action TEXT NOT NULL,
    vault_id UUID,
    target_type TEXT,
    target_id TEXT,
    details JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX audit_events_vault_idx ON audit_events (vault_id, at DESC);

-- Existing data moves into `Archive`, but only when there is any.
INSERT INTO vaults (id, name, created_by, created_at, updated_at)
SELECT
    gen_random_uuid(),
    'Archive',
    (SELECT id FROM users WHERE is_admin OR is_owner ORDER BY is_owner DESC, created_at, id LIMIT 1),
    now(),
    now()
WHERE EXISTS (SELECT 1 FROM documents)
   OR EXISTS (SELECT 1 FROM tags)
   OR EXISTS (SELECT 1 FROM correspondents)
   OR EXISTS (SELECT 1 FROM document_types)
   OR EXISTS (SELECT 1 FROM workflows)
   OR EXISTS (SELECT 1 FROM ingestion_sources);

INSERT INTO vault_members (vault_id, principal_type, user_id, role, granted_at)
SELECT id, 'user', created_by, 'owner', created_at FROM vaults WHERE created_by IS NOT NULL;

UPDATE users SET default_vault_id = (SELECT id FROM vaults WHERE name = 'Archive')
 WHERE id = (SELECT created_by FROM vaults WHERE name = 'Archive');

ALTER TABLE documents ADD COLUMN vault_id UUID REFERENCES vaults(id);
ALTER TABLE documents ADD COLUMN uploaded_by TEXT REFERENCES users(id) ON DELETE SET NULL;
UPDATE documents SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE documents ALTER COLUMN vault_id SET NOT NULL;
CREATE INDEX idx_documents_vault_created ON documents (vault_id, created_at);

-- Vocabularies: unique per vault instead of per instance.
ALTER TABLE correspondents ADD COLUMN vault_id UUID REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE correspondents SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE correspondents ALTER COLUMN vault_id SET NOT NULL;
ALTER TABLE correspondents DROP CONSTRAINT correspondents_name_key;
ALTER TABLE correspondents ADD CONSTRAINT correspondents_vault_name_key UNIQUE (vault_id, name);

ALTER TABLE document_types ADD COLUMN vault_id UUID REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE document_types SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE document_types ALTER COLUMN vault_id SET NOT NULL;
ALTER TABLE document_types DROP CONSTRAINT document_types_name_key;
ALTER TABLE document_types ADD CONSTRAINT document_types_vault_name_key UNIQUE (vault_id, name);

ALTER TABLE tags ADD COLUMN vault_id UUID REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE tags SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE tags ALTER COLUMN vault_id SET NOT NULL;
ALTER TABLE tags DROP CONSTRAINT tags_name_key;
ALTER TABLE tags ADD CONSTRAINT tags_vault_name_key UNIQUE (vault_id, name);

-- Workflows.
ALTER TABLE workflows ADD COLUMN vault_id UUID REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE workflows SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE workflows ALTER COLUMN vault_id SET NOT NULL;
DROP INDEX idx_workflows_trigger;
CREATE INDEX idx_workflows_trigger ON workflows (vault_id, trigger_event, enabled);

-- Ingestion sources and the ledger of what they imported.
ALTER TABLE ingestion_sources ADD COLUMN vault_id UUID REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE ingestion_sources SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE ingestion_sources ALTER COLUMN vault_id SET NOT NULL;
ALTER TABLE ingestion_sources DROP CONSTRAINT ingestion_sources_name_key;
ALTER TABLE ingestion_sources ADD CONSTRAINT ingestion_sources_vault_name_key UNIQUE (vault_id, name);

ALTER TABLE ingest_ledger ADD COLUMN vault_id UUID REFERENCES vaults(id) ON DELETE CASCADE;
UPDATE ingest_ledger SET vault_id = (SELECT id FROM vaults WHERE name = 'Archive');
ALTER TABLE ingest_ledger ALTER COLUMN vault_id SET NOT NULL;
ALTER TABLE ingest_ledger DROP CONSTRAINT ingest_ledger_pkey;
ALTER TABLE ingest_ledger ADD PRIMARY KEY (vault_id, key);

CREATE TABLE saved_views (
    id UUID PRIMARY KEY,
    vault_id UUID NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'shared')),
    query_json TEXT NOT NULL,
    presentation TEXT NOT NULL CHECK (presentation IN ('list', 'grid')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX saved_views_live_name ON saved_views (vault_id, owner_id, name)
    WHERE deleted_at IS NULL;
CREATE INDEX saved_views_visible ON saved_views (vault_id, visibility, owner_id, id)
    WHERE deleted_at IS NULL;

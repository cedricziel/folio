-- External identities (forward auth, later OIDC), admin flag and groups.
-- Group names live in two namespaces (`source` is 'local' or 'external') so an
-- identity provider group never matches a local group of the same name.

ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN display_name TEXT;
ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN auth_source TEXT NOT NULL DEFAULT 'local';
ALTER TABLE users ADD COLUMN external_id TEXT;

-- Accounts created before this migration came from /setup: owners.
UPDATE users SET is_admin = 1;

CREATE UNIQUE INDEX users_external_identity_idx ON users (auth_source, external_id);

CREATE TABLE auth_groups (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (source, name)
);

CREATE TABLE auth_group_members (
    group_id TEXT NOT NULL REFERENCES auth_groups(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX auth_group_members_user_idx ON auth_group_members (user_id);

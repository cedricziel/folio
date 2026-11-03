ALTER TABLE vaults ADD COLUMN updated_by TEXT REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE vaults ADD COLUMN deleted_at TEXT;
CREATE INDEX idx_vaults_active ON vaults (created_at, id) WHERE deleted_at IS NULL;

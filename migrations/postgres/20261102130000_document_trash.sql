ALTER TABLE documents ADD COLUMN deleted_at TIMESTAMPTZ;
ALTER TABLE documents ADD COLUMN purge_requested_at TIMESTAMPTZ;
CREATE INDEX idx_documents_trash ON documents (vault_id, deleted_at, id) WHERE deleted_at IS NOT NULL;
CREATE INDEX idx_documents_purge_requested ON documents (purge_requested_at) WHERE purge_requested_at IS NOT NULL;

CREATE INDEX idx_documents_browse_created ON documents (vault_id, created_at, id);
CREATE INDEX idx_documents_browse_title ON documents (vault_id, title COLLATE "C", id);

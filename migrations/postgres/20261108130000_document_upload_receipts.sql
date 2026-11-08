-- Lets a client retry a document upload with the same Idempotency-Key without creating a second document.
-- document_id stays NULL while the first request is still storing the file.
CREATE TABLE document_upload_receipts (
    vault_id UUID NOT NULL REFERENCES vaults (id) ON DELETE CASCADE,
    actor_scope TEXT NOT NULL,
    idempotency_key UUID NOT NULL,
    request_fingerprint TEXT NOT NULL,
    document_id UUID,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (vault_id, actor_scope, idempotency_key)
);

CREATE INDEX idx_document_upload_receipts_created_at ON document_upload_receipts (created_at);

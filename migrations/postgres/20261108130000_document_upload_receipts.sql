-- Lets a client retry a document upload with the same Idempotency-Key without creating a second document.
-- The document id is reserved when the key is first claimed, so every attempt for one key imports the same
-- document. claim_token identifies the attempt that holds the key; it renews last_active_at while it imports, and a
-- claim not renewed for a while may be taken over. document_created records that an attempt created the reserved
-- document, so a retry after it was deleted is told so instead of creating it again.
CREATE TABLE document_upload_receipts (
    vault_id UUID NOT NULL REFERENCES vaults (id) ON DELETE CASCADE,
    actor_scope TEXT NOT NULL,
    idempotency_key UUID NOT NULL,
    request_fingerprint TEXT NOT NULL,
    document_id UUID NOT NULL,
    document_created BOOLEAN NOT NULL DEFAULT FALSE,
    claim_token UUID NOT NULL,
    last_active_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (vault_id, actor_scope, idempotency_key)
);

CREATE INDEX idx_document_upload_receipts_last_active_at ON document_upload_receipts (last_active_at);

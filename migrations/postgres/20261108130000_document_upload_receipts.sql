-- Lets a client retry a document upload with the same Idempotency-Key without creating a second document.
-- The document id is reserved when the key is first claimed, so an attempt that was interrupted after storing
-- the document is resumed under the same id. claim_token and claimed_at identify the attempt that holds the key;
-- completed_at is set once that attempt has stored the document.
CREATE TABLE document_upload_receipts (
    vault_id UUID NOT NULL REFERENCES vaults (id) ON DELETE CASCADE,
    actor_scope TEXT NOT NULL,
    idempotency_key UUID NOT NULL,
    request_fingerprint TEXT NOT NULL,
    document_id UUID NOT NULL,
    claim_token UUID NOT NULL,
    claimed_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (vault_id, actor_scope, idempotency_key)
);

CREATE INDEX idx_document_upload_receipts_created_at ON document_upload_receipts (created_at);

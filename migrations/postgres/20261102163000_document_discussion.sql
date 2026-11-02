CREATE UNIQUE INDEX idx_documents_id_vault ON documents (id, vault_id);

CREATE TABLE document_entries (
    id UUID PRIMARY KEY,
    document_id UUID NOT NULL,
    vault_id UUID NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('note', 'comment')),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 16384),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    created_by TEXT,
    updated_by TEXT,
    UNIQUE (id, document_id, vault_id),
    FOREIGN KEY (document_id, vault_id) REFERENCES documents (id, vault_id)
        ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX idx_document_entries_page
    ON document_entries (vault_id, document_id, kind, created_at, id)
    WHERE deleted_at IS NULL;

CREATE TABLE document_entry_creation_receipts (
    vault_id UUID NOT NULL,
    document_id UUID NOT NULL,
    actor_scope TEXT NOT NULL,
    idempotency_key UUID NOT NULL,
    request_fingerprint TEXT NOT NULL,
    entry_id UUID NOT NULL,
    original_response TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (vault_id, document_id, actor_scope, idempotency_key),
    FOREIGN KEY (entry_id, document_id, vault_id)
        REFERENCES document_entries (id, document_id, vault_id)
        ON DELETE CASCADE ON UPDATE CASCADE
);

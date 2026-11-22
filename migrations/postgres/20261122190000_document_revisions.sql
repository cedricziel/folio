CREATE TABLE document_files (
    id UUID NOT NULL PRIMARY KEY,
    document_id UUID NOT NULL,
    vault_id UUID NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('original', 'archival')),
    storage_key TEXT NOT NULL CHECK (length(storage_key) > 0),
    original_filename TEXT,
    mime_type TEXT NOT NULL CHECK (length(mime_type) > 0),
    verification_state TEXT NOT NULL CHECK (verification_state IN ('pending', 'verified', 'unavailable')),
    sha256 TEXT,
    byte_size BIGINT,
    CHECK (
        (verification_state = 'verified'
            AND sha256 IS NOT NULL AND sha256 ~ '^[0-9a-f]{64}$'
            AND byte_size IS NOT NULL AND byte_size >= 0)
        OR (verification_state <> 'verified' AND sha256 IS NULL AND byte_size IS NULL)
    ),
    UNIQUE (id, document_id),
    FOREIGN KEY (document_id, vault_id) REFERENCES documents (id, vault_id)
        ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX document_files_unverified ON document_files (id)
    WHERE verification_state <> 'verified';

CREATE TABLE document_revisions (
    id UUID NOT NULL PRIMARY KEY,
    document_id UUID NOT NULL,
    vault_id UUID NOT NULL,
    number BIGINT NOT NULL CHECK (number > 0),
    original_file_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    created_by TEXT,
    restored_from_revision_id UUID CHECK (restored_from_revision_id <> id),
    UNIQUE (document_id, number),
    UNIQUE (id, document_id),
    FOREIGN KEY (document_id, vault_id) REFERENCES documents (id, vault_id)
        ON UPDATE CASCADE ON DELETE CASCADE,
    FOREIGN KEY (original_file_id, document_id) REFERENCES document_files (id, document_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (restored_from_revision_id, document_id) REFERENCES document_revisions (id, document_id)
        DEFERRABLE INITIALLY DEFERRED
);

ALTER TABLE documents ADD COLUMN current_revision_id UUID;
ALTER TABLE documents ADD CONSTRAINT documents_current_revision_owner
    FOREIGN KEY (current_revision_id, id) REFERENCES document_revisions (id, document_id)
    DEFERRABLE INITIALLY DEFERRED;

INSERT INTO document_files (
    id, document_id, vault_id, role, storage_key, original_filename, mime_type, verification_state
)
SELECT id, id, vault_id, 'original', file_path, original_filename,
    COALESCE(NULLIF(mime_type, ''), 'application/octet-stream'), 'pending'
FROM documents
WHERE file_path IS NOT NULL AND file_path <> '' AND NOT upload_pending;

INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at, created_by)
SELECT d.id, d.id, d.vault_id, 1, f.id, d.created_at, d.uploaded_by
FROM documents d JOIN document_files f ON f.document_id = d.id;

UPDATE documents
SET current_revision_id = id
WHERE id IN (SELECT document_id FROM document_revisions);

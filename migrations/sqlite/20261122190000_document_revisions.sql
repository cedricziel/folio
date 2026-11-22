CREATE TABLE document_files (
    id TEXT NOT NULL PRIMARY KEY,
    document_id TEXT NOT NULL,
    vault_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('original', 'archival')),
    storage_key TEXT NOT NULL CHECK (length(storage_key) > 0),
    original_filename TEXT,
    mime_type TEXT NOT NULL CHECK (length(mime_type) > 0),
    verification_state TEXT NOT NULL CHECK (verification_state IN ('pending', 'verified', 'unavailable')),
    sha256 TEXT,
    byte_size INTEGER,
    CHECK (
        (verification_state = 'verified'
            AND sha256 IS NOT NULL AND length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'
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
    id TEXT NOT NULL PRIMARY KEY,
    document_id TEXT NOT NULL,
    vault_id TEXT NOT NULL,
    number INTEGER NOT NULL CHECK (number > 0),
    original_file_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_by TEXT,
    restored_from_revision_id TEXT CHECK (restored_from_revision_id <> id),
    UNIQUE (document_id, number),
    UNIQUE (id, document_id),
    FOREIGN KEY (document_id, vault_id) REFERENCES documents (id, vault_id)
        ON UPDATE CASCADE ON DELETE CASCADE,
    FOREIGN KEY (original_file_id, document_id) REFERENCES document_files (id, document_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (restored_from_revision_id, document_id) REFERENCES document_revisions (id, document_id)
        DEFERRABLE INITIALLY DEFERRED
);

ALTER TABLE documents ADD COLUMN current_revision_id TEXT REFERENCES document_revisions (id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TRIGGER documents_current_revision_owner
BEFORE UPDATE OF current_revision_id, id ON documents
WHEN NEW.current_revision_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM document_revisions WHERE id = NEW.current_revision_id AND document_id = NEW.id
)
BEGIN
    SELECT RAISE(ABORT, 'current revision must belong to its document');
END;

CREATE TRIGGER documents_insert_revision_owner
BEFORE INSERT ON documents
WHEN NEW.current_revision_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM document_revisions WHERE id = NEW.current_revision_id AND document_id = NEW.id
)
BEGIN
    SELECT RAISE(ABORT, 'current revision must belong to its document');
END;

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

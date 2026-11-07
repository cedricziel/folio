-- Paperless-ngx import: documents remember where they were imported from

ALTER TABLE documents
    ADD COLUMN external_source TEXT,
    ADD COLUMN external_id TEXT,
    ADD COLUMN content_sha256 TEXT,
    ADD CONSTRAINT documents_external_ref_paired
        CHECK ((external_source IS NULL) = (external_id IS NULL));

-- Includes trashed rows, so a trashed import is never imported again.
CREATE UNIQUE INDEX documents_external_ref ON documents (vault_id, external_source, external_id)
    WHERE external_source IS NOT NULL;


ALTER TABLE document_entries ADD COLUMN external_id TEXT;

-- Imported notes are matched by their ID in the source system.
CREATE UNIQUE INDEX document_entries_external_id ON document_entries (document_id, external_id)
    WHERE external_id IS NOT NULL;

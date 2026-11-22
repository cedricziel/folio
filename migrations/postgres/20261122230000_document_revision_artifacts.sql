CREATE TABLE document_revision_artifacts (
    file_id UUID PRIMARY KEY,
    document_id UUID NOT NULL,
    vault_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    FOREIGN KEY (document_id, vault_id) REFERENCES documents (id, vault_id)
        ON UPDATE CASCADE ON DELETE CASCADE,
    FOREIGN KEY (revision_id, document_id) REFERENCES document_revisions (id, document_id)
        ON DELETE CASCADE,
    FOREIGN KEY (file_id, document_id) REFERENCES document_files (id, document_id)
        ON DELETE CASCADE
);

CREATE INDEX document_revision_artifacts_by_revision
    ON document_revision_artifacts (document_id, revision_id);

ALTER TABLE document_revisions
    ADD COLUMN publication_key UUID,
    ADD COLUMN publication_fingerprint TEXT,
    ADD CONSTRAINT document_revisions_publication_receipt CHECK (
        (publication_key IS NULL AND publication_fingerprint IS NULL)
        OR (number > 1 AND publication_key IS NOT NULL AND publication_fingerprint IS NOT NULL
            AND publication_fingerprint ~ '^[0-9a-f]{64}$')
    );

CREATE UNIQUE INDEX document_revisions_publication_key
    ON document_revisions (document_id, publication_key)
    WHERE publication_key IS NOT NULL;

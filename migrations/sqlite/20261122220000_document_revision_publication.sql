ALTER TABLE document_revisions ADD COLUMN publication_key TEXT;
ALTER TABLE document_revisions ADD COLUMN publication_fingerprint TEXT CHECK (
    (publication_key IS NULL AND publication_fingerprint IS NULL)
    OR (number > 1 AND publication_key IS NOT NULL AND publication_fingerprint IS NOT NULL
        AND length(publication_fingerprint) = 64
        AND publication_fingerprint NOT GLOB '*[^0-9a-f]*')
);

CREATE UNIQUE INDEX document_revisions_publication_key
    ON document_revisions (document_id, publication_key)
    WHERE publication_key IS NOT NULL;

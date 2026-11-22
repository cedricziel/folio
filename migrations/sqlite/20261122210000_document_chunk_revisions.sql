ALTER TABLE document_chunks
    ADD COLUMN revision_id TEXT REFERENCES document_revisions(id) ON DELETE CASCADE;

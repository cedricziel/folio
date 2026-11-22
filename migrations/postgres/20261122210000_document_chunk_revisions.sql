ALTER TABLE document_chunks
    ADD COLUMN revision_id UUID REFERENCES document_revisions(id) ON DELETE CASCADE;

ALTER TABLE documents ADD COLUMN reviewed_at TIMESTAMPTZ;
ALTER TABLE documents ADD COLUMN reviewed_by TEXT;
ALTER TABLE documents ADD COLUMN review_origin TEXT
    CHECK (
        (reviewed_at IS NULL AND reviewed_by IS NULL AND review_origin IS NULL)
        OR (
            reviewed_at IS NOT NULL AND review_origin IS NOT NULL
            AND (
                (review_origin = 'human' AND reviewed_by IS NOT NULL)
                OR (review_origin IN ('system', 'legacy_classification') AND reviewed_by IS NULL)
            )
        )
    );

UPDATE documents
SET reviewed_at = CURRENT_TIMESTAMP,
    review_origin = 'legacy_classification'
WHERE ocr_status NOT IN ('pending', 'running', 'failed')
  AND (correspondent_id IS NOT NULL OR document_type_id IS NOT NULL);

CREATE INDEX idx_documents_review_state
    ON documents(vault_id, reviewed_at, created_at DESC, id DESC)
    WHERE deleted_at IS NULL;

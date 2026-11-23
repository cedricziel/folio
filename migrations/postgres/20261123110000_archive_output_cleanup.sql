-- A write can outlive both its worker and the document's permanent deletion.
CREATE TABLE archive_output_cleanup (
    attempt_id UUID PRIMARY KEY NOT NULL,
    document_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    write_settled BOOLEAN NOT NULL DEFAULT FALSE,
    next_check_at BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX archive_output_cleanup_due ON archive_output_cleanup (next_check_at, attempt_id);

INSERT INTO archive_output_cleanup (attempt_id, document_id, revision_id)
SELECT id, document_id, revision_id FROM archival_attempts WHERE cleanup_pending;
DROP INDEX archival_cleanup_pending;
ALTER TABLE archival_attempts DROP COLUMN cleanup_pending;

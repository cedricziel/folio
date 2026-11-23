-- A single writer reserves the canonical key before touching object storage.
ALTER TABLE archival_attempts ADD COLUMN cleanup_pending BOOLEAN NOT NULL DEFAULT FALSE;
CREATE INDEX archival_cleanup_pending ON archival_attempts (id) WHERE cleanup_pending;

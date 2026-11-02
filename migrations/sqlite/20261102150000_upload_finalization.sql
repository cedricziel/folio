-- Existing documents have already announced their creation.
ALTER TABLE documents ADD COLUMN upload_pending BOOLEAN NOT NULL DEFAULT FALSE;

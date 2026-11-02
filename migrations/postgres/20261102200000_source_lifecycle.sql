ALTER TABLE ingestion_sources ADD COLUMN deleted_at TIMESTAMPTZ;
ALTER TABLE ingestion_sources ADD COLUMN created_by TEXT;
ALTER TABLE ingestion_sources ADD COLUMN updated_by TEXT;
ALTER TABLE ingestion_sources DROP CONSTRAINT ingestion_sources_vault_name_key;
CREATE UNIQUE INDEX ingestion_sources_live_name ON ingestion_sources(vault_id, name) WHERE deleted_at IS NULL;

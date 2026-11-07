ALTER TABLE workflows ADD COLUMN deleted_at TIMESTAMPTZ;
ALTER TABLE workflows ADD COLUMN created_by TEXT;
ALTER TABLE workflows ADD COLUMN updated_by TEXT;
CREATE INDEX workflows_active_trigger ON workflows(vault_id, trigger_event, enabled) WHERE deleted_at IS NULL;

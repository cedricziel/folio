ALTER TABLE workflows ADD COLUMN additional_triggers JSONB NOT NULL DEFAULT '[]'::jsonb
    CHECK (jsonb_typeof(additional_triggers) = 'array');

CREATE INDEX workflows_additional_triggers ON workflows USING GIN (additional_triggers)
    WHERE enabled AND deleted_at IS NULL;

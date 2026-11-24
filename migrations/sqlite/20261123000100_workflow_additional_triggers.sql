ALTER TABLE workflows ADD COLUMN additional_triggers TEXT NOT NULL DEFAULT '[]'
    CHECK (json_type(additional_triggers) = 'array');

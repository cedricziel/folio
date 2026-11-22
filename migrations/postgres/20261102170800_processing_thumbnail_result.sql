ALTER TABLE processing_stage_runs ADD COLUMN result_prefix TEXT
    CHECK (result_prefix IS NULL OR (stage = 'thumbnail' AND state = 'completed'));

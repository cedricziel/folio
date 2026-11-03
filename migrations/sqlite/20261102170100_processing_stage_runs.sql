ALTER TABLE jobs ADD COLUMN processing_group_id TEXT;

CREATE TABLE processing_stage_runs (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    vault_id TEXT NOT NULL,
    revision_id TEXT,
    job_id TEXT,
    group_id TEXT NOT NULL,
    stage TEXT NOT NULL CHECK (stage IN ('ocr', 'embedding', 'thumbnail')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'waiting_to_retry', 'completed', 'failed', 'skipped')),
    attempt BIGINT NOT NULL CHECK (attempt BETWEEN 0 AND 4294967295),
    queue_attempt INTEGER NOT NULL CHECK (queue_attempt >= 0),
    claim_id TEXT,
    reason_code TEXT CHECK (reason_code IN ('provider_disabled', 'no_text', 'unsupported_format', 'provider_failed', 'invalid_response', 'storage_failed', 'lease_expired')),
    retry_at TEXT,
    has_text INTEGER CHECK (has_text IN (0, 1)),
    is_current INTEGER NOT NULL DEFAULT 1 CHECK (is_current IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (job_id, stage),
    UNIQUE (group_id, stage),
    CHECK (reason_code <> 'no_text' OR (stage = 'embedding' AND state = 'skipped')),
    CHECK (reason_code <> 'unsupported_format' OR (stage = 'thumbnail' AND state = 'skipped')),
    CHECK (reason_code <> 'provider_disabled' OR state = 'skipped'),
    CHECK (state <> 'skipped' OR reason_code IN ('provider_disabled', 'no_text', 'unsupported_format')),
    CHECK ((state = 'pending' AND attempt = 0) OR (state <> 'pending' AND attempt > 0)),
    CHECK ((state = 'running' AND claim_id IS NOT NULL AND queue_attempt > 0) OR (state <> 'running' AND claim_id IS NULL)),
    CHECK ((state = 'waiting_to_retry' AND retry_at IS NOT NULL) OR (state <> 'waiting_to_retry' AND retry_at IS NULL)),
    CHECK ((state IN ('failed', 'skipped', 'waiting_to_retry') AND reason_code IS NOT NULL) OR (state NOT IN ('failed', 'skipped', 'waiting_to_retry') AND reason_code IS NULL)),
    CHECK ((stage = 'ocr' AND state = 'completed' AND has_text IS NOT NULL) OR (NOT (stage = 'ocr' AND state = 'completed') AND has_text IS NULL))
);

CREATE UNIQUE INDEX idx_processing_current_stage
    ON processing_stage_runs(document_id, stage) WHERE is_current = 1;
CREATE INDEX idx_processing_stage_job ON processing_stage_runs(job_id);

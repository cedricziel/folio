ALTER TABLE jobs ADD COLUMN processing_group_id UUID;

CREATE TABLE processing_stage_runs (
    id UUID PRIMARY KEY,
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    vault_id UUID NOT NULL,
    revision_id UUID,
    job_id UUID,
    group_id UUID NOT NULL,
    stage TEXT NOT NULL CHECK (stage IN ('ocr', 'embedding', 'thumbnail')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'waiting_to_retry', 'completed', 'failed', 'skipped')),
    attempt BIGINT NOT NULL CHECK (attempt BETWEEN 0 AND 4294967295),
    queue_attempt INTEGER NOT NULL CHECK (queue_attempt >= 0),
    claim_id UUID,
    reason_code TEXT CHECK (reason_code IN ('provider_disabled', 'no_text', 'unsupported_format', 'provider_failed', 'invalid_response', 'storage_failed', 'lease_expired')),
    retry_at TIMESTAMPTZ,
    has_text BOOLEAN,
    is_current BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
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
    ON processing_stage_runs(document_id, stage) WHERE is_current = TRUE;
CREATE INDEX idx_processing_stage_job ON processing_stage_runs(job_id);

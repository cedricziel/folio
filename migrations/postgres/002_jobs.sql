-- Job queue for background processing

CREATE TABLE jobs (
    id UUID PRIMARY KEY,
    job_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    scheduled_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_jobs_pending ON jobs(status, scheduled_at) WHERE status = 'pending';
CREATE INDEX idx_jobs_completed ON jobs(status, completed_at) WHERE status = 'completed';
CREATE INDEX idx_jobs_document ON jobs((payload->'data'->>'document_id'));

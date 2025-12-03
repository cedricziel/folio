-- Job queue for background processing

CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    job_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    scheduled_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT
);

-- Index for fetching pending jobs
CREATE INDEX idx_jobs_pending ON jobs(status, scheduled_at) WHERE status = 'pending';

-- Index for cleanup of old completed jobs
CREATE INDEX idx_jobs_completed ON jobs(status, completed_at) WHERE status = 'completed';

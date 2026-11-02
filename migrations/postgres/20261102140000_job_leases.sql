ALTER TABLE jobs ADD COLUMN claim_id UUID;
ALTER TABLE jobs ADD COLUMN lease_expires_at BIGINT;
CREATE INDEX idx_jobs_lease_expiry ON jobs (lease_expires_at) WHERE status = 'processing';

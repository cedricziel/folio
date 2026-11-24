ALTER TABLE workflows ADD COLUMN cadence TEXT;
ALTER TABLE workflows ADD COLUMN next_run_at TEXT;
CREATE INDEX workflows_due_clock ON workflows(next_run_at, id)
    WHERE enabled = 1 AND deleted_at IS NULL AND next_run_at IS NOT NULL;

ALTER TABLE workflow_runs ADD COLUMN scheduled_at TEXT;
CREATE UNIQUE INDEX workflow_runs_clock_occurrence ON workflow_runs(workflow_id, scheduled_at)
    WHERE scheduled_at IS NOT NULL;

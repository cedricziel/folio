ALTER TABLE workflows ADD COLUMN revision BIGINT NOT NULL DEFAULT 1;
ALTER TABLE workflow_runs ADD COLUMN definition_revision BIGINT;

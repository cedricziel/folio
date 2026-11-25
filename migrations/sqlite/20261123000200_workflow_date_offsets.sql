ALTER TABLE workflows ADD COLUMN next_schedule_document TEXT;
ALTER TABLE workflow_runs ADD COLUMN scheduled_document_date TEXT;

DROP INDEX workflow_runs_clock_occurrence;
CREATE UNIQUE INDEX workflow_runs_clock_occurrence ON workflow_runs(workflow_id, scheduled_at)
    WHERE scheduled_at IS NOT NULL AND scheduled_document_date IS NULL;
CREATE UNIQUE INDEX workflow_runs_document_occurrence
    ON workflow_runs(workflow_id, document_id, scheduled_at, scheduled_document_date)
    WHERE scheduled_at IS NOT NULL AND scheduled_document_date IS NOT NULL AND document_id IS NOT NULL;
CREATE INDEX documents_workflow_date ON documents(vault_id, document_date, id)
    WHERE deleted_at IS NULL AND document_date IS NOT NULL;

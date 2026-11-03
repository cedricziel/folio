ALTER TABLE workflow_runs ADD COLUMN mutation_id UUID;
CREATE UNIQUE INDEX workflow_runs_workflow_mutation
    ON workflow_runs (workflow_id, mutation_id)
    WHERE mutation_id IS NOT NULL;

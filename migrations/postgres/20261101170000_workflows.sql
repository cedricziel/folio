-- Workflow engine: automations and their execution history (PostgreSQL)

CREATE TABLE workflows (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    trigger_event TEXT NOT NULL,
    conditions TEXT NOT NULL DEFAULT '{}',
    actions TEXT NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_workflows_trigger ON workflows(trigger_event, enabled);

CREATE TABLE workflow_runs (
    id UUID PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    event TEXT NOT NULL,
    -- Deliberately not a foreign key: runs for deleted documents must survive them.
    document_id UUID,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    completed_actions INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    response_status INTEGER,
    -- TEXT, not JSONB: the body is signed, so it must be kept byte for byte.
    payload TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs(workflow_id, created_at DESC);

-- Once-per-document events: concurrent inserts for the same workflow and document collapse to one run.
CREATE UNIQUE INDEX idx_workflow_runs_once ON workflow_runs(workflow_id, event, document_id)
    WHERE event IN ('document.uploaded', 'document.processed');

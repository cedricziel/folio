-- Workflow engine: automations and their execution history

CREATE TABLE workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_event TEXT NOT NULL,
    conditions TEXT NOT NULL DEFAULT '{}',
    actions TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_workflows_trigger ON workflows(trigger_event, enabled);

CREATE TABLE workflow_runs (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    event TEXT NOT NULL,
    -- Deliberately not a foreign key: runs for deleted documents must survive them.
    document_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    completed_actions INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    response_status INTEGER,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs(workflow_id, created_at DESC);
-- Once-per-document events: concurrent inserts for the same workflow and document collapse to one run.
CREATE UNIQUE INDEX idx_workflow_runs_once ON workflow_runs(workflow_id, event, document_id)
    WHERE event IN ('document.uploaded', 'document.processed');

-- Transactional outbox: domain events are written in the transaction of the
-- change they describe and delivered afterwards by a dispatcher.

CREATE TABLE outbox_events (
    -- Doubles as the SSE event id, so it must only ever grow.
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    dispatched_at TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    -- Retry backoff: not claimed before this time.
    available_at TEXT NOT NULL,
    -- Lease of the dispatcher that claimed the row, so a crashed one does not hold it forever.
    claimed_until TEXT,
    last_error TEXT
);

CREATE INDEX idx_outbox_pending ON outbox_events(available_at) WHERE dispatched_at IS NULL;
CREATE INDEX idx_outbox_dispatched ON outbox_events(dispatched_at) WHERE dispatched_at IS NOT NULL;

-- Runs execute the actions they were created with, and a run exists once per
-- workflow and event, however often the event is delivered.
ALTER TABLE workflow_runs ADD COLUMN actions TEXT NOT NULL DEFAULT '[]';
ALTER TABLE workflow_runs ADD COLUMN source_event_id INTEGER;

UPDATE workflow_runs
SET actions = (SELECT workflows.actions FROM workflows WHERE workflows.id = workflow_runs.workflow_id);

CREATE UNIQUE INDEX idx_workflow_runs_source_event ON workflow_runs(workflow_id, source_event_id)
    WHERE source_event_id IS NOT NULL;

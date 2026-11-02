ALTER TABLE outbox_events ADD COLUMN event_id TEXT;
CREATE UNIQUE INDEX idx_outbox_event_identity ON outbox_events (event_id);

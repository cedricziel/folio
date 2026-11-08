-- Apple devices registered for push notifications, owned by the OAuth grant
-- that registered them, with their delivery bookkeeping.

CREATE TABLE push_devices (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    installation_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    environment TEXT NOT NULL,
    locale TEXT NOT NULL,
    preferences TEXT NOT NULL,
    alert_token TEXT,
    alert_token_registered_at TEXT,
    widget_token TEXT,
    widget_token_registered_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_delivered_at TEXT,
    -- Lease of the sender working on this device, so replicas never send twice.
    claimed_until TEXT,
    -- Hourly alert cap.
    window_started_at TEXT,
    alerts_in_window INTEGER NOT NULL DEFAULT 0,
    backoff_until TEXT,
    failures INTEGER NOT NULL DEFAULT 0,
    widgets_dirty_at TEXT,
    widgets_sent_at TEXT,
    test_sent_at TEXT,
    UNIQUE (user_id, installation_id)
);

CREATE UNIQUE INDEX idx_push_devices_alert_token ON push_devices(alert_token) WHERE alert_token IS NOT NULL;
CREATE UNIQUE INDEX idx_push_devices_widget_token ON push_devices(widget_token) WHERE widget_token IS NOT NULL;
CREATE INDEX idx_push_devices_grant ON push_devices(grant_id);
CREATE INDEX idx_push_devices_client ON push_devices(user_id, client_id);

-- The first completion and first terminal failure of each document; later
-- ones (reprocessing, re-embedding, repair, duplicate delivery) never notify.
CREATE TABLE push_document_outcomes (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (document_id, outcome)
);

-- What each device has yet to hear about, one row per document and kind.
CREATE TABLE push_pending (
    device_id TEXT NOT NULL REFERENCES push_devices(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    vault_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (device_id, document_id, kind)
);

CREATE INDEX idx_push_pending_created ON push_pending(created_at);

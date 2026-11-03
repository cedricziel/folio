-- Enqueue-time scope survives vault moves and deletion; legacy jobs remain unscoped.
ALTER TABLE jobs ADD COLUMN origin_vault_id UUID;
ALTER TABLE jobs ADD COLUMN correlation_id UUID;
ALTER TABLE jobs ADD COLUMN causation_id UUID;

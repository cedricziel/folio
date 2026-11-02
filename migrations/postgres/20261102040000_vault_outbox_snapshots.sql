-- Deletion snapshots can outlive every document and vocabulary row. They need
-- the upgrade vault even when the main vault backfill had no live data.
INSERT INTO vaults (id, name, created_by, created_at, updated_at)
SELECT gen_random_uuid(), 'Archive',
    (SELECT id FROM users WHERE is_admin OR is_owner ORDER BY is_owner DESC, created_at, id LIMIT 1),
    now(), now()
WHERE NOT EXISTS (SELECT 1 FROM vaults WHERE name = 'Archive')
  AND EXISTS (
      SELECT 1 FROM outbox_events
      WHERE jsonb_typeof(payload #> '{snapshot,document}') = 'object'
        AND NOT (payload #> '{snapshot,document}' ? 'vault_id')
  );

INSERT INTO vault_members (vault_id, principal_type, user_id, role, granted_at)
SELECT v.id, 'user', v.created_by, 'owner', v.created_at
FROM vaults v
WHERE v.name = 'Archive' AND v.created_by IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM vault_members m WHERE m.vault_id = v.id AND m.user_id IS NOT NULL AND m.role = 'owner');

UPDATE users
SET default_vault_id = (SELECT id FROM vaults WHERE name = 'Archive' ORDER BY created_at, id LIMIT 1)
WHERE default_vault_id IS NULL
  AND id = (SELECT created_by FROM vaults WHERE name = 'Archive' ORDER BY created_at, id LIMIT 1);

UPDATE outbox_events
SET payload = jsonb_set(payload, '{snapshot,document,vault_id}', to_jsonb(
    (SELECT id::text FROM vaults WHERE name = 'Archive' ORDER BY created_at, id LIMIT 1)))
WHERE jsonb_typeof(payload #> '{snapshot,document}') = 'object'
  AND NOT (payload #> '{snapshot,document}' ? 'vault_id');

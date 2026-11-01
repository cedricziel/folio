-- The first account is the owner: always admin, never demoted by a group or
-- role mapping. Existing installs treat their earliest account as the owner.
ALTER TABLE users ADD COLUMN is_owner INTEGER NOT NULL DEFAULT 0;
UPDATE users SET is_owner = 1, is_admin = 1
 WHERE id = (SELECT id FROM users ORDER BY created_at, id LIMIT 1);

-- The first account is the owner: always admin, never demoted by a group or
-- role mapping. Existing installs treat their earliest account as the owner.
ALTER TABLE users ADD COLUMN is_owner BOOLEAN NOT NULL DEFAULT FALSE;
UPDATE users SET is_owner = TRUE, is_admin = TRUE
 WHERE id = (SELECT id FROM users ORDER BY created_at, id LIMIT 1);

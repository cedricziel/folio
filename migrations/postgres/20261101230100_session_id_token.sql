-- The ID token of an OpenID Connect login, sent back to the provider as the
-- hint when the user signs out.

ALTER TABLE sessions ADD COLUMN id_token TEXT;

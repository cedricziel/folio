-- Every code and the tokens it yields share a grant id, so reuse of a rotated
-- refresh token or a revocation can invalidate the whole family. Rows from
-- before this migration keep the empty id and are not grouped.

ALTER TABLE oauth_codes ADD COLUMN grant_id TEXT NOT NULL DEFAULT '';
ALTER TABLE oauth_access_tokens ADD COLUMN grant_id TEXT NOT NULL DEFAULT '';
ALTER TABLE oauth_refresh_tokens ADD COLUMN grant_id TEXT NOT NULL DEFAULT '';
-- A rotated refresh token is kept (used = 1) so its reuse can be detected.
ALTER TABLE oauth_refresh_tokens ADD COLUMN used INTEGER NOT NULL DEFAULT 0;

CREATE INDEX oauth_access_tokens_grant ON oauth_access_tokens (grant_id);
CREATE INDEX oauth_refresh_tokens_grant ON oauth_refresh_tokens (grant_id);

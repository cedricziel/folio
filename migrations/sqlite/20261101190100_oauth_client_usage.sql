-- Set once a client has been issued a token. Clients that never get one are
-- purged after a day so open dynamic registration cannot fill the table.

ALTER TABLE oauth_clients ADD COLUMN token_issued_at INTEGER;

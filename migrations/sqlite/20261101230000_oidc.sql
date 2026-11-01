-- In-flight OpenID Connect logins. One row per authorization request, bound to
-- the browser that started it and consumed once by the callback. The state and
-- the browser token are stored as SHA-256 hashes.

CREATE TABLE oidc_logins (
    state_hash TEXT PRIMARY KEY,
    browser_hash TEXT NOT NULL,
    nonce TEXT NOT NULL,
    pkce_verifier TEXT NOT NULL,
    next TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

-- Pending OpenID Connect logins: state, nonce and PKCE verifier survive the
-- redirect to the identity provider and are consumed by the callback.

CREATE TABLE oidc_flows (
    state_hash TEXT PRIMARY KEY,
    nonce TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    next TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

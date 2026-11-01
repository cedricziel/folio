//! SQLite implementation of the auth store.

use async_trait::async_trait;
use folio_auth::{
    AccessToken, AuthCode, AuthError, AuthStore, OAuthClient, RefreshToken, Session, User,
};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

pub struct SqliteAuthStore {
    pool: SqlitePool,
}

impl SqliteAuthStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn err(e: sqlx::Error) -> AuthError {
    AuthError::Store(e.to_string())
}

fn split(scopes: &str) -> Vec<String> {
    scopes.split_whitespace().map(String::from).collect()
}

fn user_from(row: SqliteRow) -> User {
    User {
        id: row.get("id"),
        username: row.get("username"),
        password_hash: row.get("password_hash"),
        created_at: row.get("created_at"),
    }
}

#[async_trait]
impl AuthStore for SqliteAuthStore {
    async fn count_users(&self) -> Result<i64, AuthError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(err)
    }

    async fn create_first_user(&self, user: User) -> Result<Option<User>, AuthError> {
        let result = sqlx::query(
            "INSERT INTO users (id, username, username_normalized, password_hash, created_at)
             SELECT $1, $2, $3, $4, $5 WHERE NOT EXISTS (SELECT 1 FROM users)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(user.username.to_lowercase())
        .bind(&user.password_hash)
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok((result.rows_affected() == 1).then_some(user))
    }

    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>, AuthError> {
        sqlx::query("SELECT * FROM users WHERE username_normalized = $1")
            .bind(username.to_lowercase())
            .fetch_optional(&self.pool)
            .await
            .map(|r| r.map(user_from))
            .map_err(err)
    }

    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AuthError> {
        sqlx::query("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map(|r| r.map(user_from))
            .map_err(err)
    }

    async fn create_session(&self, s: Session) -> Result<(), AuthError> {
        sqlx::query("INSERT INTO sessions (id_hash, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(s.id_hash)
            .bind(s.user_id)
            .bind(s.expires_at)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn find_session(&self, id_hash: &str) -> Result<Option<Session>, AuthError> {
        sqlx::query("SELECT id_hash, user_id, expires_at FROM sessions WHERE id_hash = $1")
            .bind(id_hash)
            .fetch_optional(&self.pool)
            .await
            .map(|r| {
                r.map(|row| Session {
                    id_hash: row.get("id_hash"),
                    user_id: row.get("user_id"),
                    expires_at: row.get("expires_at"),
                })
            })
            .map_err(err)
    }

    async fn delete_session(&self, id_hash: &str) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM sessions WHERE id_hash = $1")
            .bind(id_hash)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn insert_client(&self, c: OAuthClient) -> Result<(), AuthError> {
        let uris =
            serde_json::to_string(&c.redirect_uris).map_err(|e| AuthError::Store(e.to_string()))?;
        sqlx::query(
            "INSERT INTO oauth_clients (client_id, client_name, redirect_uris, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(c.client_id)
        .bind(c.client_name)
        .bind(uris)
        .bind(c.created_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(err)
    }

    async fn find_client(&self, client_id: &str) -> Result<Option<OAuthClient>, AuthError> {
        let row = sqlx::query("SELECT * FROM oauth_clients WHERE client_id = $1")
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        row.map(|r| {
            let uris: String = r.get("redirect_uris");
            Ok(OAuthClient {
                client_id: r.get("client_id"),
                client_name: r.get("client_name"),
                redirect_uris: serde_json::from_str(&uris)
                    .map_err(|e| AuthError::Store(e.to_string()))?,
                created_at: r.get("created_at"),
            })
        })
        .transpose()
    }

    async fn mark_client_used(&self, client_id: &str, at: i64) -> Result<(), AuthError> {
        sqlx::query(
            "UPDATE oauth_clients SET token_issued_at = $2
             WHERE client_id = $1 AND token_issued_at IS NULL",
        )
        .bind(client_id)
        .bind(at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(err)
    }

    async fn purge_unused_clients(&self, registered_before: i64) -> Result<u64, AuthError> {
        sqlx::query("DELETE FROM oauth_clients WHERE token_issued_at IS NULL AND created_at < $1")
            .bind(registered_before)
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected())
            .map_err(err)
    }

    async fn insert_code(&self, c: AuthCode) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO oauth_codes
             (code_hash, client_id, user_id, redirect_uri, code_challenge, scopes, audience, expires_at, grant_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(c.code_hash)
        .bind(c.client_id)
        .bind(c.user_id)
        .bind(c.redirect_uri)
        .bind(c.code_challenge)
        .bind(c.scopes.join(" "))
        .bind(c.audience)
        .bind(c.expires_at)
        .bind(c.grant_id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(err)
    }

    async fn take_code(&self, code_hash: &str) -> Result<Option<AuthCode>, AuthError> {
        let row = sqlx::query("DELETE FROM oauth_codes WHERE code_hash = $1 RETURNING *")
            .bind(code_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.map(|r| AuthCode {
            code_hash: r.get("code_hash"),
            client_id: r.get("client_id"),
            user_id: r.get("user_id"),
            redirect_uri: r.get("redirect_uri"),
            code_challenge: r.get("code_challenge"),
            scopes: split(r.get("scopes")),
            audience: r.get("audience"),
            expires_at: r.get("expires_at"),
            grant_id: r.get("grant_id"),
        }))
    }

    async fn insert_access_token(&self, t: AccessToken) -> Result<(), AuthError> {
        insert_token(
            &self.pool,
            INSERT_ACCESS,
            &t.token_hash,
            &t.client_id,
            &t.user_id,
            &t.scopes,
            &t.audience,
            t.expires_at,
            &t.grant_id,
        )
        .await
    }

    async fn find_access_token(&self, token_hash: &str) -> Result<Option<AccessToken>, AuthError> {
        let row = sqlx::query("SELECT * FROM oauth_access_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.map(|r| AccessToken {
            token_hash: r.get("token_hash"),
            client_id: r.get("client_id"),
            user_id: r.get("user_id"),
            scopes: split(r.get("scopes")),
            audience: r.get("audience"),
            expires_at: r.get("expires_at"),
            grant_id: r.get("grant_id"),
        }))
    }

    async fn insert_refresh_token(&self, t: RefreshToken) -> Result<(), AuthError> {
        insert_token(
            &self.pool,
            INSERT_REFRESH,
            &t.token_hash,
            &t.client_id,
            &t.user_id,
            &t.scopes,
            &t.audience,
            t.expires_at,
            &t.grant_id,
        )
        .await
    }

    async fn take_refresh_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshToken>, AuthError> {
        let row = sqlx::query(
            "UPDATE oauth_refresh_tokens SET used = 1 WHERE token_hash = $1 AND used = 0 RETURNING *",
        )
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.map(|r| RefreshToken {
            token_hash: r.get("token_hash"),
            client_id: r.get("client_id"),
            user_id: r.get("user_id"),
            scopes: split(r.get("scopes")),
            audience: r.get("audience"),
            expires_at: r.get("expires_at"),
            grant_id: r.get("grant_id"),
        }))
    }

    async fn used_refresh_grant(&self, token_hash: &str) -> Result<Option<String>, AuthError> {
        sqlx::query_scalar(
            "SELECT grant_id FROM oauth_refresh_tokens WHERE token_hash = $1 AND used = 1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(err)
    }

    async fn revoke_grant(&self, grant_id: &str) -> Result<(), AuthError> {
        if grant_id.is_empty() {
            return Ok(());
        }
        for sql in [
            "DELETE FROM oauth_access_tokens WHERE grant_id = $1",
            "DELETE FROM oauth_refresh_tokens WHERE grant_id = $1",
        ] {
            sqlx::query(sql)
                .bind(grant_id)
                .execute(&self.pool)
                .await
                .map_err(err)?;
        }
        Ok(())
    }

    async fn revoke_token(&self, token_hash: &str, client_id: &str) -> Result<(), AuthError> {
        let lookups = [
            (
                "SELECT grant_id FROM oauth_access_tokens WHERE token_hash = $1 AND client_id = $2",
                "DELETE FROM oauth_access_tokens WHERE token_hash = $1",
            ),
            (
                "SELECT grant_id FROM oauth_refresh_tokens WHERE token_hash = $1 AND client_id = $2",
                "DELETE FROM oauth_refresh_tokens WHERE token_hash = $1",
            ),
        ];
        for (select, delete) in lookups {
            let found: Option<String> = sqlx::query_scalar(select)
                .bind(token_hash)
                .bind(client_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(err)?;
            if let Some(grant_id) = found {
                // Tokens from before grants existed are revoked on their own.
                sqlx::query(delete)
                    .bind(token_hash)
                    .execute(&self.pool)
                    .await
                    .map_err(err)?;
                return self.revoke_grant(&grant_id).await;
            }
        }
        Ok(())
    }

    async fn purge_expired(&self, now: i64) -> Result<(), AuthError> {
        let statements = [
            "DELETE FROM oauth_codes WHERE expires_at <= $1",
            "DELETE FROM oauth_access_tokens WHERE expires_at <= $1",
            "DELETE FROM oauth_refresh_tokens WHERE expires_at <= $1",
            "DELETE FROM sessions WHERE expires_at <= $1",
        ];
        for sql in statements {
            sqlx::query(sql)
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(err)?;
        }
        Ok(())
    }
}

const INSERT_ACCESS: &str = "INSERT INTO oauth_access_tokens
    (token_hash, client_id, user_id, scopes, audience, expires_at, grant_id)
    VALUES ($1, $2, $3, $4, $5, $6, $7)";
const INSERT_REFRESH: &str = "INSERT INTO oauth_refresh_tokens
    (token_hash, client_id, user_id, scopes, audience, expires_at, grant_id)
    VALUES ($1, $2, $3, $4, $5, $6, $7)";

#[allow(clippy::too_many_arguments)]
async fn insert_token(
    pool: &SqlitePool,
    sql: &'static str,
    hash: &str,
    client_id: &str,
    user_id: &str,
    scopes: &[String],
    audience: &str,
    expires_at: i64,
    grant_id: &str,
) -> Result<(), AuthError> {
    sqlx::query(sql)
        .bind(hash)
        .bind(client_id)
        .bind(user_id)
        .bind(scopes.join(" "))
        .bind(audience)
        .bind(expires_at)
        .bind(grant_id)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(err)
}

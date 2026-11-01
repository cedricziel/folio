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
            "INSERT INTO users (id, username, password_hash, created_at)
             SELECT ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok((result.rows_affected() == 1).then_some(user))
    }

    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>, AuthError> {
        sqlx::query("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await
            .map(|r| r.map(user_from))
            .map_err(err)
    }

    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AuthError> {
        sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map(|r| r.map(user_from))
            .map_err(err)
    }

    async fn create_session(&self, s: Session) -> Result<(), AuthError> {
        sqlx::query("INSERT INTO sessions (id_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(s.id_hash)
            .bind(s.user_id)
            .bind(s.expires_at)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn find_session(&self, id_hash: &str) -> Result<Option<Session>, AuthError> {
        sqlx::query("SELECT id_hash, user_id, expires_at FROM sessions WHERE id_hash = ?")
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
        sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
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
             VALUES (?, ?, ?, ?)",
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
        let row = sqlx::query("SELECT * FROM oauth_clients WHERE client_id = ?")
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

    async fn insert_code(&self, c: AuthCode) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO oauth_codes
             (code_hash, client_id, user_id, redirect_uri, code_challenge, scopes, audience, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(c.code_hash)
        .bind(c.client_id)
        .bind(c.user_id)
        .bind(c.redirect_uri)
        .bind(c.code_challenge)
        .bind(c.scopes.join(" "))
        .bind(c.audience)
        .bind(c.expires_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(err)
    }

    async fn take_code(&self, code_hash: &str) -> Result<Option<AuthCode>, AuthError> {
        let row = sqlx::query("DELETE FROM oauth_codes WHERE code_hash = ? RETURNING *")
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
        }))
    }

    async fn insert_access_token(&self, t: AccessToken) -> Result<(), AuthError> {
        insert_token(
            &self.pool,
            "oauth_access_tokens",
            &t.token_hash,
            &t.client_id,
            &t.user_id,
            &t.scopes,
            &t.audience,
            t.expires_at,
        )
        .await
    }

    async fn find_access_token(&self, token_hash: &str) -> Result<Option<AccessToken>, AuthError> {
        let row = sqlx::query("SELECT * FROM oauth_access_tokens WHERE token_hash = ?")
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
        }))
    }

    async fn insert_refresh_token(&self, t: RefreshToken) -> Result<(), AuthError> {
        insert_token(
            &self.pool,
            "oauth_refresh_tokens",
            &t.token_hash,
            &t.client_id,
            &t.user_id,
            &t.scopes,
            &t.audience,
            t.expires_at,
        )
        .await
    }

    async fn take_refresh_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshToken>, AuthError> {
        let row = sqlx::query("DELETE FROM oauth_refresh_tokens WHERE token_hash = ? RETURNING *")
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
        }))
    }

    async fn revoke_token(&self, token_hash: &str) -> Result<(), AuthError> {
        for table in ["oauth_access_tokens", "oauth_refresh_tokens"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE token_hash = ?"))
                .bind(token_hash)
                .execute(&self.pool)
                .await
                .map_err(err)?;
        }
        Ok(())
    }

    async fn purge_expired(&self, now: i64) -> Result<(), AuthError> {
        for table in [
            "oauth_codes",
            "oauth_access_tokens",
            "oauth_refresh_tokens",
            "sessions",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE expires_at <= ?"))
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(err)?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_token(
    pool: &SqlitePool,
    table: &str,
    hash: &str,
    client_id: &str,
    user_id: &str,
    scopes: &[String],
    audience: &str,
    expires_at: i64,
) -> Result<(), AuthError> {
    sqlx::query(&format!(
        "INSERT INTO {table} (token_hash, client_id, user_id, scopes, audience, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)"
    ))
    .bind(hash)
    .bind(client_id)
    .bind(user_id)
    .bind(scopes.join(" "))
    .bind(audience)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(err)
}

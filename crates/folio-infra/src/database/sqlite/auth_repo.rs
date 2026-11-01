//! SQLite implementation of the auth store.

use async_trait::async_trait;
use folio_auth::{
    AccessToken, AuthCode, AuthError, AuthStore, Group, GroupSource, OAuthClient, OidcLogin,
    RefreshToken, Session, User,
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
        email: row.get("email"),
        display_name: row.get("display_name"),
        is_admin: row.get::<i64, _>("is_admin") != 0,
        auth_source: row.get("auth_source"),
        external_id: row.get("external_id"),
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
            "INSERT INTO users
             (id, username, username_normalized, password_hash, created_at, is_admin, auth_source)
             SELECT $1, $2, $3, $4, $5, 1, 'local' WHERE NOT EXISTS (SELECT 1 FROM users)",
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

    async fn create_user(&self, user: User) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO users
             (id, username, username_normalized, password_hash, created_at,
              email, display_name, is_admin, auth_source, external_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(user.username.to_lowercase())
        .bind(&user.password_hash)
        .bind(user.created_at)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(i64::from(user.is_admin))
        .bind(&user.auth_source)
        .bind(&user.external_id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            sqlx::Error::Database(ref d) if d.is_unique_violation() => AuthError::Conflict,
            other => err(other),
        })
    }

    async fn find_user_by_external(
        &self,
        source: &str,
        external_id: &str,
    ) -> Result<Option<User>, AuthError> {
        sqlx::query("SELECT * FROM users WHERE auth_source = $1 AND external_id = $2")
            .bind(source)
            .bind(external_id)
            .fetch_optional(&self.pool)
            .await
            .map(|r| r.map(user_from))
            .map_err(err)
    }

    async fn update_user_profile(
        &self,
        id: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        is_admin: bool,
    ) -> Result<(), AuthError> {
        sqlx::query("UPDATE users SET email = $1, display_name = $2, is_admin = $3 WHERE id = $4")
            .bind(email)
            .bind(display_name)
            .bind(i64::from(is_admin))
            .bind(id)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn set_external_groups(&self, user_id: &str, groups: &[String]) -> Result<(), AuthError> {
        let mut current: Vec<String> = sqlx::query_scalar(
            "SELECT g.name FROM auth_group_members m
             JOIN auth_groups g ON g.id = m.group_id
             WHERE m.user_id = $1 AND g.source = 'external'",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        let mut wanted = groups.to_vec();
        current.sort();
        wanted.sort();
        wanted.dedup();
        if current == wanted {
            return Ok(());
        }

        let mut tx = self.pool.begin().await.map_err(err)?;
        sqlx::query(
            "DELETE FROM auth_group_members
             WHERE user_id = $1
               AND group_id IN (SELECT id FROM auth_groups WHERE source = 'external')",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
        for name in &wanted {
            sqlx::query(
                "INSERT INTO auth_groups (id, source, name, created_at)
                 VALUES ($1, 'external', $2, $3) ON CONFLICT (source, name) DO NOTHING",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(name)
            .bind(folio_auth::server::now())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
            sqlx::query(
                "INSERT INTO auth_group_members (group_id, user_id)
                 SELECT id, $1 FROM auth_groups WHERE source = 'external' AND name = $2",
            )
            .bind(user_id)
            .bind(name)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }
        tx.commit().await.map_err(err)
    }

    async fn user_groups(&self, user_id: &str) -> Result<Vec<Group>, AuthError> {
        let rows = sqlx::query(
            "SELECT g.source, g.name FROM auth_group_members m
             JOIN auth_groups g ON g.id = m.group_id
             WHERE m.user_id = $1 ORDER BY g.source, g.name",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let source: String = r.get("source");
                Some(Group {
                    source: GroupSource::parse(&source)?,
                    name: r.get("name"),
                })
            })
            .collect())
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

    async fn insert_code(&self, c: AuthCode) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO oauth_codes
             (code_hash, client_id, user_id, redirect_uri, code_challenge, scopes, audience, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
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
        )
        .await
    }

    async fn take_refresh_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshToken>, AuthError> {
        let row = sqlx::query("DELETE FROM oauth_refresh_tokens WHERE token_hash = $1 RETURNING *")
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
        sqlx::query("DELETE FROM oauth_access_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        sqlx::query("DELETE FROM oauth_refresh_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn insert_oidc_login(&self, login: OidcLogin) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO oidc_logins
             (state_hash, browser_hash, nonce, pkce_verifier, next, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(login.state_hash)
        .bind(login.browser_hash)
        .bind(login.nonce)
        .bind(login.pkce_verifier)
        .bind(login.next)
        .bind(login.expires_at)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn take_oidc_login(&self, state_hash: &str) -> Result<Option<OidcLogin>, AuthError> {
        let row = sqlx::query("DELETE FROM oidc_logins WHERE state_hash = $1 RETURNING *")
            .bind(state_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.map(|r| OidcLogin {
            state_hash: r.get("state_hash"),
            browser_hash: r.get("browser_hash"),
            nonce: r.get("nonce"),
            pkce_verifier: r.get("pkce_verifier"),
            next: r.get("next"),
            expires_at: r.get("expires_at"),
        }))
    }

    async fn purge_expired(&self, now: i64) -> Result<(), AuthError> {
        let statements = [
            "DELETE FROM oauth_codes WHERE expires_at <= $1",
            "DELETE FROM oauth_access_tokens WHERE expires_at <= $1",
            "DELETE FROM oauth_refresh_tokens WHERE expires_at <= $1",
            "DELETE FROM sessions WHERE expires_at <= $1",
            "DELETE FROM oidc_logins WHERE expires_at <= $1",
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
    (token_hash, client_id, user_id, scopes, audience, expires_at)
    VALUES ($1, $2, $3, $4, $5, $6)";
const INSERT_REFRESH: &str = "INSERT INTO oauth_refresh_tokens
    (token_hash, client_id, user_id, scopes, audience, expires_at)
    VALUES ($1, $2, $3, $4, $5, $6)";

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
) -> Result<(), AuthError> {
    sqlx::query(sql)
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

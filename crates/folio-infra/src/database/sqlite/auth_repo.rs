//! SQLite implementation of the auth store.

use async_trait::async_trait;
use folio_auth::{
    AccessToken, AuthCode, AuthError, AuthStore, ConnectedClient, GrantCheck, Group, GroupSource,
    IssuedTokens, OAuthClient, OidcLogin, PasskeyRecord, PersonalToken, RefreshExchange,
    RefreshToken, Session, TotpRecord, User,
};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

pub struct SqliteAuthStore {
    pool: SqlitePool,
}

impl SqliteAuthStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Starts a transaction that serializes with every other writer, so token
    /// exchanges and disconnects of a client run one after the other.
    async fn begin_locked(
        &self,
        _client_id: &str,
    ) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, AuthError> {
        self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)
    }

    /// Takes a grant with `take_sql` and stores the tokens `check` issues for it.
    async fn take_and_issue<G>(
        tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
        client_id: &str,
        take_sql: &'static str,
        hash: &str,
        from_row: fn(SqliteRow) -> G,
        check: GrantCheck<'_, G>,
    ) -> Result<Option<G>, AuthError> {
        let Some(row) = sqlx::query(take_sql)
            .bind(hash)
            .fetch_optional(&mut **tx)
            .await
            .map_err(err)?
        else {
            return Ok(None);
        };
        let grant = from_row(row);
        let Some(IssuedTokens { access, refresh }) = check(&grant) else {
            return Ok(Some(grant));
        };
        if access.client_id != client_id || refresh.client_id != client_id {
            // Only the locked client's tokens are serialized with its disconnect.
            return Err(AuthError::Store(
                "issued tokens belong to another client".into(),
            ));
        }
        sqlx::query(MARK_CLIENT_USED)
            .bind(client_id)
            .bind(folio_auth::server::now())
            .execute(&mut **tx)
            .await
            .map_err(err)?;
        let a = &access;
        insert_token(
            &mut **tx,
            INSERT_ACCESS,
            &a.token_hash,
            &a.client_id,
            &a.user_id,
            &a.scopes,
            &a.audience,
            a.expires_at,
            &a.grant_id,
        )
        .await?;
        let r = &refresh;
        insert_token(
            &mut **tx,
            INSERT_REFRESH,
            &r.token_hash,
            &r.client_id,
            &r.user_id,
            &r.scopes,
            &r.audience,
            r.expires_at,
            &r.grant_id,
        )
        .await?;
        Ok(Some(grant))
    }

    /// Deletes every token of a grant inside `tx`.
    async fn delete_grant(
        tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
        grant_id: &str,
    ) -> Result<(), AuthError> {
        for sql in [
            "DELETE FROM oauth_access_tokens WHERE grant_id = $1",
            "DELETE FROM oauth_refresh_tokens WHERE grant_id = $1",
        ] {
            sqlx::query(sql)
                .bind(grant_id)
                .execute(&mut **tx)
                .await
                .map_err(err)?;
        }
        Ok(())
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
        is_owner: row.get::<i64, _>("is_owner") != 0,
        auth_source: row.get("auth_source"),
        external_id: row.get("external_id"),
    }
}

fn personal_token_from(row: SqliteRow) -> PersonalToken {
    PersonalToken {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        token_hash: row.get("token_hash"),
        prefix: row.get("prefix"),
        audience: row.get("audience"),
        scopes: split(row.get("scopes")),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
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
             (id, username, username_normalized, password_hash, created_at, is_admin, is_owner, auth_source)
             SELECT $1, $2, $3, $4, $5, 1, 1, 'local' WHERE NOT EXISTS (SELECT 1 FROM users)",
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

    async fn create_external_user(&self, user: User) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO users
             (id, username, username_normalized, password_hash, created_at,
              email, display_name, is_admin, is_owner, auth_source, external_id)
             SELECT $1, $2, $3, $4, $5, $6, $7,
                    CASE WHEN $8 = 1 OR NOT EXISTS (SELECT 1 FROM users) THEN 1 ELSE 0 END,
                    CASE WHEN NOT EXISTS (SELECT 1 FROM users) THEN 1 ELSE 0 END,
                    $9, $10",
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
        sqlx::query("UPDATE users SET email = $1, display_name = $2, is_admin = CASE WHEN is_owner = 1 THEN 1 ELSE $3 END WHERE id = $4")
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
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
        let mut current: Vec<String> = sqlx::query_scalar(
            "SELECT g.name FROM auth_group_members m
             JOIN auth_groups g ON g.id = m.group_id
             WHERE m.user_id = $1 AND g.source = 'external'",
        )
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(err)?;
        let mut wanted = groups.to_vec();
        current.sort();
        wanted.sort();
        wanted.dedup();
        if current == wanted {
            return Ok(());
        }

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
                 SELECT id, $1 FROM auth_groups WHERE source = 'external' AND name = $2
                 ON CONFLICT DO NOTHING",
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
        sqlx::query(
            "INSERT INTO sessions (id_hash, user_id, expires_at, created_at, user_agent)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(s.id_hash)
        .bind(s.user_id)
        .bind(s.expires_at)
        .bind(s.created_at)
        .bind(s.user_agent)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(err)
    }

    async fn find_session(&self, id_hash: &str) -> Result<Option<Session>, AuthError> {
        sqlx::query(
            "SELECT id_hash, user_id, expires_at, created_at, user_agent FROM sessions
             WHERE id_hash = $1",
        )
        .bind(id_hash)
        .fetch_optional(&self.pool)
        .await
        .map(|r| {
            r.map(|row| Session {
                id_hash: row.get("id_hash"),
                user_id: row.get("user_id"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
                user_agent: row.get("user_agent"),
            })
        })
        .map_err(err)
    }

    async fn set_session_id_token(&self, id_hash: &str, id_token: &str) -> Result<(), AuthError> {
        sqlx::query("UPDATE sessions SET id_token = $1 WHERE id_hash = $2")
            .bind(id_token)
            .bind(id_hash)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn session_id_token(&self, id_hash: &str) -> Result<Option<String>, AuthError> {
        let row = sqlx::query("SELECT id_token FROM sessions WHERE id_hash = $1")
            .bind(id_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.and_then(|r| r.get("id_token")))
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
        sqlx::query(MARK_CLIENT_USED)
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
        let row = sqlx::query(TAKE_CODE)
            .bind(code_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.map(code_from))
    }

    async fn exchange_code(
        &self,
        client_id: &str,
        code_hash: &str,
        check: GrantCheck<'_, AuthCode>,
    ) -> Result<Option<AuthCode>, AuthError> {
        let mut tx = self.begin_locked(client_id).await?;
        let taken =
            Self::take_and_issue(&mut tx, client_id, TAKE_CODE, code_hash, code_from, check)
                .await?;
        tx.commit().await.map_err(err)?;
        Ok(taken)
    }

    async fn insert_personal_token(&self, token: PersonalToken) -> Result<(), AuthError> {
        sqlx::query("INSERT INTO personal_access_tokens (id, user_id, name, token_hash, prefix, audience, scopes, created_at, expires_at, revoked_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(token.id).bind(token.user_id).bind(token.name).bind(token.token_hash)
            .bind(token.prefix).bind(token.audience).bind(token.scopes.join(" "))
            .bind(token.created_at).bind(token.expires_at).bind(token.revoked_at)
            .execute(&self.pool).await.map(|_| ()).map_err(err)
    }

    async fn find_personal_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalToken>, AuthError> {
        sqlx::query("SELECT * FROM personal_access_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.map(personal_token_from))
            .map_err(err)
    }

    async fn list_personal_tokens(&self, user_id: &str) -> Result<Vec<PersonalToken>, AuthError> {
        sqlx::query(
            "SELECT * FROM personal_access_tokens WHERE user_id = $1 ORDER BY created_at DESC, id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(personal_token_from).collect())
        .map_err(err)
    }

    async fn revoke_personal_token(
        &self,
        user_id: &str,
        id: &str,
        at: i64,
    ) -> Result<bool, AuthError> {
        sqlx::query("UPDATE personal_access_tokens SET revoked_at = COALESCE(revoked_at, $1) WHERE user_id = $2 AND id = $3")
            .bind(at).bind(user_id).bind(id).execute(&self.pool).await
            .map(|result| result.rows_affected() > 0).map_err(err)
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
        let row = sqlx::query(TAKE_REFRESH)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        Ok(row.map(refresh_from))
    }

    async fn exchange_refresh_token(
        &self,
        client_id: &str,
        token_hash: &str,
        check: GrantCheck<'_, RefreshToken>,
    ) -> Result<RefreshExchange, AuthError> {
        let mut tx = self.begin_locked(client_id).await?;
        let taken = Self::take_and_issue(
            &mut tx,
            client_id,
            TAKE_REFRESH,
            token_hash,
            refresh_from,
            check,
        )
        .await?;
        if let Some(token) = taken {
            tx.commit().await.map_err(err)?;
            return Ok(RefreshExchange::Rotated(token));
        }
        let used: Option<(String, String)> = sqlx::query_as(
            "SELECT grant_id, client_id FROM oauth_refresh_tokens WHERE token_hash = $1 AND used = 1",
        )
        .bind(token_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(err)?;
        let Some((grant_id, owner)) = used else {
            tx.commit().await.map_err(err)?;
            return Ok(RefreshExchange::Unknown);
        };
        // Rotations of the grant hold its owner's lock; revoking under that lock
        // keeps a rotation racing the replay from keeping its new tokens.
        if owner != client_id {
            tx.commit().await.map_err(err)?;
            tx = self.begin_locked(&owner).await?;
        }
        if !grant_id.is_empty() {
            Self::delete_grant(&mut tx, &grant_id).await?;
        }
        tx.commit().await.map_err(err)?;
        Ok(RefreshExchange::Replayed)
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

    async fn list_sessions(&self, user_id: &str, now: i64) -> Result<Vec<Session>, AuthError> {
        let rows = sqlx::query(
            "SELECT id_hash, user_id, expires_at, created_at, user_agent FROM sessions
             WHERE user_id = $1 AND expires_at > $2 ORDER BY created_at DESC, id_hash",
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        Ok(rows
            .into_iter()
            .map(|row| Session {
                id_hash: row.get("id_hash"),
                user_id: row.get("user_id"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
                user_agent: row.get("user_agent"),
            })
            .collect())
    }

    async fn delete_user_sessions(
        &self,
        user_id: &str,
        keep: Option<&str>,
    ) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id_hash <> $2")
            .bind(user_id)
            .bind(keep.unwrap_or(""))
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn set_password_hash(&self, user_id: &str, hash: &str) -> Result<(), AuthError> {
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(hash)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn list_connected_clients(
        &self,
        user_id: &str,
        now: i64,
    ) -> Result<Vec<ConnectedClient>, AuthError> {
        let rows = sqlx::query(
            "SELECT c.client_id, c.client_name, MAX(t.expires_at) AS expires_at FROM (
                 SELECT client_id, expires_at FROM oauth_access_tokens
                 WHERE user_id = $1 AND expires_at > $2
                 UNION ALL
                 SELECT client_id, expires_at FROM oauth_refresh_tokens
                 WHERE user_id = $1 AND expires_at > $2 AND used = 0
             ) t JOIN oauth_clients c ON c.client_id = t.client_id
             GROUP BY c.client_id, c.client_name
             ORDER BY expires_at DESC, c.client_id",
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        Ok(rows
            .into_iter()
            .map(|r| ConnectedClient {
                client_id: r.get("client_id"),
                client_name: r.get("client_name"),
                expires_at: r.get("expires_at"),
            })
            .collect())
    }

    async fn revoke_client_tokens(&self, user_id: &str, client_id: &str) -> Result<(), AuthError> {
        let mut tx = self.begin_locked(client_id).await?;
        for sql in [
            "DELETE FROM oauth_codes WHERE user_id = $1 AND client_id = $2",
            "DELETE FROM oauth_access_tokens WHERE user_id = $1 AND client_id = $2",
            "DELETE FROM oauth_refresh_tokens WHERE user_id = $1 AND client_id = $2",
        ] {
            sqlx::query(sql)
                .bind(user_id)
                .bind(client_id)
                .execute(&mut *tx)
                .await
                .map_err(err)?;
        }
        tx.commit().await.map_err(err)
    }

    async fn get_totp(&self, user_id: &str) -> Result<Option<TotpRecord>, AuthError> {
        let row = sqlx::query(
            "SELECT user_id, secret_enc, confirmed, last_step FROM user_totp WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(err)?;
        Ok(row.map(|r| TotpRecord {
            user_id: r.get("user_id"),
            secret_enc: r.get("secret_enc"),
            confirmed: r.get::<i64, _>("confirmed") != 0,
            last_step: r.get("last_step"),
        }))
    }

    async fn save_pending_totp(
        &self,
        user_id: &str,
        secret_enc: &str,
        now: i64,
    ) -> Result<bool, AuthError> {
        sqlx::query(
            "INSERT INTO user_totp (user_id, secret_enc, confirmed, last_step, created_at)
             VALUES ($1, $2, 0, 0, $3)
             ON CONFLICT (user_id) DO UPDATE
             SET secret_enc = excluded.secret_enc, created_at = excluded.created_at
             WHERE user_totp.confirmed = 0",
        )
        .bind(user_id)
        .bind(secret_enc)
        .bind(now)
        .execute(&self.pool)
        .await
        .map(|r| r.rows_affected() == 1)
        .map_err(err)
    }

    async fn confirm_totp(
        &self,
        user_id: &str,
        expected_secret: &str,
        step: i64,
        recovery_hashes: &[String],
    ) -> Result<bool, AuthError> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        let confirmed = sqlx::query(
            "UPDATE user_totp SET confirmed = 1, last_step = $2 WHERE user_id = $1 AND confirmed = 0 AND secret_enc = $3",
        )
        .bind(user_id)
        .bind(step)
        .bind(expected_secret)
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected()
            == 1;
        if !confirmed {
            return Ok(false);
        }
        replace_codes(&mut tx, user_id, recovery_hashes).await?;
        tx.commit().await.map_err(err)?;
        Ok(true)
    }

    async fn advance_totp_step(&self, user_id: &str, step: i64) -> Result<bool, AuthError> {
        sqlx::query(
            "UPDATE user_totp SET last_step = $2
             WHERE user_id = $1 AND confirmed = 1 AND last_step < $2",
        )
        .bind(user_id)
        .bind(step)
        .execute(&self.pool)
        .await
        .map(|r| r.rows_affected() == 1)
        .map_err(err)
    }

    async fn delete_totp(&self, user_id: &str) -> Result<(), AuthError> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        for sql in [
            "DELETE FROM user_recovery_codes WHERE user_id = $1",
            "DELETE FROM user_totp WHERE user_id = $1",
        ] {
            sqlx::query(sql)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(err)?;
        }
        tx.commit().await.map_err(err)
    }

    async fn delete_pending_totp(&self, user_id: &str) -> Result<bool, AuthError> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        let removed = sqlx::query("DELETE FROM user_totp WHERE user_id = $1 AND confirmed = 0")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(err)?
            .rows_affected()
            == 1;
        if removed {
            sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(err)?;
        }
        tx.commit().await.map_err(err)?;
        Ok(removed)
    }

    async fn replace_recovery_codes(
        &self,
        user_id: &str,
        hashes: &[String],
    ) -> Result<(), AuthError> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        replace_codes(&mut tx, user_id, hashes).await?;
        tx.commit().await.map_err(err)
    }

    async fn use_recovery_code(&self, user_id: &str, hash: &str) -> Result<bool, AuthError> {
        sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1 AND code_hash = $2")
            .bind(user_id)
            .bind(hash)
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected() == 1)
            .map_err(err)
    }

    async fn count_recovery_codes(&self, user_id: &str) -> Result<i64, AuthError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM user_recovery_codes WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await
            .map_err(err)
    }

    async fn list_passkeys(&self, user_id: &str) -> Result<Vec<PasskeyRecord>, AuthError> {
        let rows = sqlx::query(
            "SELECT id, user_id, name, credential_id, data, created_at, last_used_at
             FROM user_passkeys WHERE user_id = $1 ORDER BY created_at, id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        Ok(rows
            .into_iter()
            .map(|r| PasskeyRecord {
                id: r.get("id"),
                user_id: r.get("user_id"),
                name: r.get("name"),
                credential_id: r.get("credential_id"),
                data: r.get("data"),
                created_at: r.get("created_at"),
                last_used_at: r.get("last_used_at"),
            })
            .collect())
    }

    async fn insert_passkey(&self, p: PasskeyRecord) -> Result<(), AuthError> {
        sqlx::query(
            "INSERT INTO user_passkeys (id, user_id, name, credential_id, data, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(p.id)
        .bind(p.user_id)
        .bind(p.name)
        .bind(p.credential_id)
        .bind(p.data)
        .bind(p.created_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            sqlx::Error::Database(ref d) if d.is_unique_violation() => AuthError::Conflict,
            other => err(other),
        })
    }

    async fn touch_passkey(&self, id: &str, data: &str, used_at: i64) -> Result<(), AuthError> {
        sqlx::query("UPDATE user_passkeys SET data = $2, last_used_at = $3 WHERE id = $1")
            .bind(id)
            .bind(data)
            .bind(used_at)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn rename_passkey(&self, user_id: &str, id: &str, name: &str) -> Result<bool, AuthError> {
        sqlx::query("UPDATE user_passkeys SET name = $3 WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .bind(name)
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected() == 1)
            .map_err(err)
    }

    async fn delete_passkey(&self, user_id: &str, id: &str) -> Result<bool, AuthError> {
        sqlx::query("DELETE FROM user_passkeys WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected() == 1)
            .map_err(err)
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

async fn replace_codes(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    hashes: &[String],
) -> Result<(), AuthError> {
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(err)?;
    for hash in hashes {
        sqlx::query(
            "INSERT INTO user_recovery_codes (user_id, code_hash) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(hash)
        .execute(&mut **tx)
        .await
        .map_err(err)?;
    }
    Ok(())
}

const INSERT_ACCESS: &str = "INSERT INTO oauth_access_tokens
    (token_hash, client_id, user_id, scopes, audience, expires_at, grant_id)
    VALUES ($1, $2, $3, $4, $5, $6, $7)";
const INSERT_REFRESH: &str = "INSERT INTO oauth_refresh_tokens
    (token_hash, client_id, user_id, scopes, audience, expires_at, grant_id)
    VALUES ($1, $2, $3, $4, $5, $6, $7)";

#[allow(clippy::too_many_arguments)]
async fn insert_token(
    executor: impl sqlx::Executor<'_, Database = sqlx::Sqlite>,
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
        .execute(executor)
        .await
        .map(|_| ())
        .map_err(err)
}

const TAKE_CODE: &str = "DELETE FROM oauth_codes WHERE code_hash = $1 RETURNING *";
const TAKE_REFRESH: &str =
    "UPDATE oauth_refresh_tokens SET used = 1 WHERE token_hash = $1 AND used = 0 RETURNING *";
const MARK_CLIENT_USED: &str = "UPDATE oauth_clients SET token_issued_at = $2
    WHERE client_id = $1 AND token_issued_at IS NULL";

fn code_from(r: SqliteRow) -> AuthCode {
    AuthCode {
        code_hash: r.get("code_hash"),
        client_id: r.get("client_id"),
        user_id: r.get("user_id"),
        redirect_uri: r.get("redirect_uri"),
        code_challenge: r.get("code_challenge"),
        scopes: split(r.get("scopes")),
        audience: r.get("audience"),
        expires_at: r.get("expires_at"),
        grant_id: r.get("grant_id"),
    }
}

fn refresh_from(r: SqliteRow) -> RefreshToken {
    RefreshToken {
        token_hash: r.get("token_hash"),
        client_id: r.get("client_id"),
        user_id: r.get("user_id"),
        scopes: split(r.get("scopes")),
        audience: r.get("audience"),
        expires_at: r.get("expires_at"),
        grant_id: r.get("grant_id"),
    }
}

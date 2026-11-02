//! SQLite vaults, grants, audit log and access policy

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::models::{
    AuditEvent, GroupId, Id, NewAuditEvent, Principal, UserId, Vault, VaultId, VaultMember,
    VaultRole,
};
use folio_core::ports::{AccessPolicy, AuditLog, MembershipRepository, VaultRepository};

/// Rank used to pick the highest of several grants in SQL
const ROLE_RANK: &str = "CASE m.role WHEN 'owner' THEN 4 WHEN 'editor' THEN 3
                                     WHEN 'contributor' THEN 2 ELSE 1 END";

/// Grants that apply to `$1` (a user id): direct ones and those of the user's groups
const APPLIES_TO_USER: &str = "(m.user_id = $1 OR m.group_id IN
                                (SELECT group_id FROM auth_group_members WHERE user_id = $1))";

fn db(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

fn time(s: &str) -> Result<DateTime<Utc>> {
    s.parse()
        .map_err(|e| Error::Repository(format!("Invalid datetime: {e}")))
}

fn vault_id(s: &str) -> Result<VaultId> {
    Uuid::parse_str(s)
        .map(VaultId)
        .map_err(|e| Error::Repository(format!("Invalid UUID: {e}")))
}

fn role(s: &str) -> Result<VaultRole> {
    s.parse().map_err(Error::Repository)
}

fn rank_role(rank: i64) -> VaultRole {
    match rank {
        4 => VaultRole::Owner,
        3 => VaultRole::Editor,
        2 => VaultRole::Contributor,
        _ => VaultRole::Viewer,
    }
}

type VaultRow = (String, String, Option<String>, String, String);

fn vault(row: VaultRow) -> Result<Vault> {
    Ok(Vault {
        id: vault_id(&row.0)?,
        name: row.1,
        created_by: row.2.map(UserId),
        created_at: time(&row.3)?,
        updated_at: time(&row.4)?,
    })
}

/// SQLite implementation of the vault, membership, audit and access ports
pub struct SqliteVaultRepository {
    pool: SqlitePool,
}

impl SqliteVaultRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn insert_vault(&self, name: &str, created_by: Option<&UserId>) -> Result<Vault> {
        let now = Utc::now();
        let id = VaultId::new();
        sqlx::query(
            "INSERT INTO vaults (id, name, created_by, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(created_by.map(UserId::as_str))
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(Vault {
            id,
            name: name.to_string(),
            created_by: created_by.cloned(),
            created_at: now,
            updated_at: now,
        })
    }
}

#[async_trait]
impl VaultRepository for SqliteVaultRepository {
    async fn create(&self, name: &str, created_by: Option<&UserId>) -> Result<Vault> {
        self.insert_vault(name, created_by).await
    }

    async fn find(&self, id: VaultId) -> Result<Option<Vault>> {
        sqlx::query_as::<_, VaultRow>(
            "SELECT id, name, created_by, created_at, updated_at FROM vaults WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?
        .map(vault)
        .transpose()
    }

    async fn list_for_user(&self, user: &UserId) -> Result<Vec<(Vault, VaultRole)>> {
        let rows: Vec<(String, String, Option<String>, String, String, i64)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT v.id, v.name, v.created_by, v.created_at, v.updated_at, MAX({ROLE_RANK})
                 FROM vaults v JOIN vault_members m ON m.vault_id = v.id
                 WHERE {APPLIES_TO_USER}
                 GROUP BY v.id ORDER BY v.name, v.id"
            )))
            .bind(user.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter()
            .map(|(id, name, by, c, u, rank)| Ok((vault((id, name, by, c, u))?, rank_role(rank))))
            .collect()
    }

    async fn rename(&self, id: VaultId, name: &str) -> Result<Vault> {
        let updated = sqlx::query("UPDATE vaults SET name = $2, updated_at = $3 WHERE id = $1")
            .bind(id.to_string())
            .bind(name)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(db)?
            .rows_affected();
        if updated == 0 {
            return Err(not_found(id));
        }
        self.find(id).await?.ok_or_else(|| not_found(id))
    }

    async fn delete(&self, id: VaultId) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let (documents,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM documents WHERE vault_id = $1")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if documents > 0 {
            return Err(Error::Conflict("the vault still holds documents".into()));
        }
        let deleted = sqlx::query("DELETE FROM vaults WHERE id = $1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected();
        if deleted == 0 {
            return Err(not_found(id));
        }
        tx.commit().await.map_err(db)
    }

    async fn default_for_user(&self, user: &UserId) -> Result<Option<VaultId>> {
        let stored: Option<(Option<String>,)> =
            sqlx::query_as("SELECT default_vault_id FROM users WHERE id = $1")
                .bind(user.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        if let Some(stored) = stored.and_then(|(v,)| v) {
            let stored = vault_id(&stored)?;
            if self.effective_role(user, stored).await?.is_some() {
                return Ok(Some(stored));
            }
        }
        Ok(self
            .list_for_user(user)
            .await?
            .into_iter()
            .next()
            .map(|(v, _)| v.id))
    }

    async fn set_default(&self, user: &UserId, vault: VaultId) -> Result<()> {
        sqlx::query("UPDATE users SET default_vault_id = $2 WHERE id = $1")
            .bind(user.as_str())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn instance_default(&self) -> Result<VaultId> {
        let owners: Option<(String,)> = sqlx::query_as(
            "SELECT default_vault_id FROM users
             WHERE is_owner = 1 AND default_vault_id IS NOT NULL ORDER BY created_at, id LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        if let Some((id,)) = owners {
            return vault_id(&id);
        }
        let oldest = || async {
            sqlx::query_as::<_, (String,)>("SELECT id FROM vaults ORDER BY created_at, id LIMIT 1")
                .fetch_optional(&self.pool)
                .await
                .map_err(db)
        };
        if let Some((id,)) = oldest().await? {
            return vault_id(&id);
        }
        self.insert_vault("Archive", None).await?;
        let (id,) = oldest()
            .await?
            .ok_or_else(|| Error::Repository("no vault".into()))?;
        vault_id(&id)
    }

    async fn find_ownerless(&self, name: &str) -> Result<Vec<VaultId>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT v.id FROM vaults v WHERE v.name = $1 AND NOT EXISTS (
                 SELECT 1 FROM vault_members m
                 WHERE m.vault_id = v.id AND m.role = 'owner' AND m.user_id IS NOT NULL)
             ORDER BY v.created_at, v.id",
        )
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.iter().map(|(id,)| vault_id(id)).collect()
    }
}

fn not_found(id: VaultId) -> Error {
    Error::NotFound {
        entity_type: "Vault",
        id: id.to_string(),
    }
}

#[async_trait]
impl MembershipRepository for SqliteVaultRepository {
    async fn list(&self, vault: VaultId) -> Result<Vec<VaultMember>> {
        let rows: Vec<(
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            String,
        )> = sqlx::query_as(
            "SELECT user_id, group_id, role, granted_by, granted_at FROM vault_members
                 WHERE vault_id = $1 ORDER BY granted_at, COALESCE(user_id, group_id)",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter()
            .map(|(user, group, r, by, at)| {
                let principal = match (user, group) {
                    (Some(u), _) => Principal::User(UserId(u)),
                    (None, Some(g)) => Principal::Group(GroupId(g)),
                    (None, None) => {
                        return Err(Error::Repository("grant without principal".into()))
                    }
                };
                Ok(VaultMember {
                    vault_id: vault,
                    principal,
                    role: role(&r)?,
                    granted_by: by.map(UserId),
                    granted_at: time(&at)?,
                })
            })
            .collect()
    }

    async fn grant(
        &self,
        vault: VaultId,
        principal: &Principal,
        new_role: VaultRole,
        granted_by: Option<&UserId>,
    ) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM vaults WHERE id = $1")
            .bind(vault.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        if exists.is_none() {
            return Err(not_found(vault));
        }
        let (column, subject) = match principal {
            Principal::User(u) => ("user_id", u.as_str()),
            Principal::Group(g) => ("group_id", g.0.as_str()),
        };
        let subject_table = if column == "user_id" {
            "users"
        } else {
            "auth_groups"
        };
        let known: Option<(i64,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT 1 FROM {subject_table} WHERE id = $1"
        )))
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        if known.is_none() {
            return Err(Error::NotFound {
                entity_type: if column == "user_id" { "User" } else { "Group" },
                id: subject.to_string(),
            });
        }
        if let Principal::User(_) = principal {
            if new_role != VaultRole::Owner {
                guard_last_owner(&mut tx, vault, subject).await?;
            }
        }
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO vault_members
                 (vault_id, principal_type, {column}, role, granted_by, granted_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (vault_id, {column}) WHERE {column} IS NOT NULL
             DO UPDATE SET role = excluded.role, granted_by = excluded.granted_by,
                           granted_at = excluded.granted_at"
        )))
        .bind(vault.to_string())
        .bind(if column == "user_id" { "user" } else { "group" })
        .bind(subject)
        .bind(new_role.as_str())
        .bind(granted_by.map(UserId::as_str))
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn revoke(&self, vault: VaultId, principal: &Principal) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let (column, subject) = match principal {
            Principal::User(u) => ("user_id", u.as_str()),
            Principal::Group(g) => ("group_id", g.0.as_str()),
        };
        if let Principal::User(_) = principal {
            guard_last_owner(&mut tx, vault, subject).await?;
        }
        let removed = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM vault_members WHERE vault_id = $1 AND {column} = $2"
        )))
        .bind(vault.to_string())
        .bind(subject)
        .execute(&mut *tx)
        .await
        .map_err(db)?
        .rows_affected();
        if removed == 0 {
            return Err(Error::NotFound {
                entity_type: "Grant",
                id: subject.to_string(),
            });
        }
        tx.commit().await.map_err(db)
    }

    async fn direct_owner_count(&self, vault: VaultId) -> Result<u64> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM vault_members
             WHERE vault_id = $1 AND role = 'owner' AND user_id IS NOT NULL",
        )
        .bind(vault.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(n as u64)
    }

    async fn ensure_group(&self, source: &str, name: &str) -> Result<GroupId> {
        if !matches!(source, "local" | "external") {
            return Err(Error::Validation(format!(
                "unknown group source '{source}'"
            )));
        }
        sqlx::query(
            "INSERT INTO auth_groups (id, source, name, created_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (source, name) DO NOTHING",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(source)
        .bind(name)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        let (id,): (String,) =
            sqlx::query_as("SELECT id FROM auth_groups WHERE source = $1 AND name = $2")
                .bind(source)
                .bind(name)
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        Ok(GroupId(id))
    }
}

/// Fails when `user` is the only direct owner of the vault. Callers hold the
/// write lock of their transaction, so the count cannot change under them.
async fn guard_last_owner(
    tx: &mut sqlx::SqliteConnection,
    vault: VaultId,
    user: &str,
) -> Result<()> {
    let (is_owner,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vault_members
         WHERE vault_id = $1 AND user_id = $2 AND role = 'owner'",
    )
    .bind(vault.to_string())
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if is_owner == 0 {
        return Ok(());
    }
    let (owners,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vault_members
         WHERE vault_id = $1 AND role = 'owner' AND user_id IS NOT NULL",
    )
    .bind(vault.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if owners <= 1 {
        return Err(Error::Conflict(
            "the last direct owner of a vault cannot be removed".into(),
        ));
    }
    Ok(())
}

#[async_trait]
impl AuditLog for SqliteVaultRepository {
    async fn record(&self, event: NewAuditEvent) -> Result<()> {
        sqlx::query(
            "INSERT INTO audit_events
                 (id, at, actor_user_id, action, vault_id, target_type, target_id, details)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(event.actor.as_ref().map(UserId::as_str))
        .bind(&event.action)
        .bind(event.vault_id.map(|v| v.to_string()))
        .bind(&event.target_type)
        .bind(&event.target_id)
        .bind(event.details.to_string())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn list(&self, vault: VaultId, limit: u32) -> Result<Vec<AuditEvent>> {
        type Row = (
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, at, actor_user_id, action, vault_id, target_type, target_id, details
             FROM audit_events WHERE vault_id = $1 ORDER BY at DESC, id LIMIT $2",
        )
        .bind(vault.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter()
            .map(|(id, at, actor, action, v, tt, ti, details)| {
                Ok(AuditEvent {
                    id: Id::parse_str(&id).map_err(|e| Error::Repository(e.to_string()))?,
                    at: time(&at)?,
                    actor: actor.map(UserId),
                    action,
                    vault_id: v.as_deref().map(vault_id).transpose()?,
                    target_type: tt,
                    target_id: ti,
                    details: serde_json::from_str(&details)
                        .map_err(|e| Error::Repository(e.to_string()))?,
                })
            })
            .collect()
    }
}

#[async_trait]
impl AccessPolicy for SqliteVaultRepository {
    async fn effective_role(&self, user: &UserId, vault: VaultId) -> Result<Option<VaultRole>> {
        let (rank,): (Option<i64>,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT MAX({ROLE_RANK}) FROM vault_members m
             WHERE m.vault_id = $2 AND {APPLIES_TO_USER}"
        )))
        .bind(user.as_str())
        .bind(vault.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(rank.map(rank_role))
    }
}

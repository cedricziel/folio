//! SQLite vaults, grants, audit log and access policy

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::outbox::insert_domain_event;
use folio_core::error::{Error, Result};
use folio_core::models::{
    AuditEvent, GroupId, Id, NewAuditEvent, Principal, UserId, Vault, VaultId, VaultMember,
    VaultRole,
};
use folio_core::mutation::current_actor;
use folio_core::ports::{AccessPolicy, AuditLog, MembershipRepository, VaultRepository};
use folio_core::{DomainEventEnvelope, DomainEventSubject, EventAudience};

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

type VaultRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn vault(row: VaultRow) -> Result<Vault> {
    Ok(Vault {
        id: vault_id(&row.0)?,
        name: row.1,
        created_by: row.2.map(UserId),
        created_at: time(&row.3)?,
        updated_at: time(&row.4)?,
        updated_by: row.5.map(UserId),
        deleted_at: row.6.as_deref().map(time).transpose()?,
    })
}

/// SQLite implementation of the vault, membership, audit and access ports
/// A vault row with the user's highest role rank
type RankedVaultRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
);

pub struct SqliteVaultRepository {
    pool: SqlitePool,
}

impl SqliteVaultRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn insert_vault(&self, name: &str, owner: Option<&UserId>) -> Result<Vault> {
        let name = checked_name(name)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        if let Some(owner) = owner {
            let known: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM users WHERE id = $1")
                .bind(owner.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
            if known.is_none() {
                return Err(Error::NotFound {
                    entity_type: "User",
                    id: owner.to_string(),
                });
            }
        }
        let now = Utc::now();
        let actor = current_actor().or_else(|| owner.cloned());
        let id = VaultId::new();
        sqlx::query("INSERT INTO vaults (id, name, created_by, created_at, updated_at, updated_by) VALUES ($1, $2, $3, $4, $4, $3)")
            .bind(id.to_string()).bind(&name).bind(actor.as_ref().map(UserId::as_str)).bind(now.to_rfc3339())
            .execute(&mut *tx).await.map_err(db)?;
        let created = Vault {
            id,
            name,
            created_by: actor.clone(),
            created_at: now,
            updated_at: now,
            updated_by: actor.clone(),
            deleted_at: None,
        };
        let mutation = Uuid::new_v4();
        emit_vault(&mut tx, &created, "created", mutation).await?;
        if let Some(owner) = owner {
            let principal = Principal::User(owner.clone());
            write_grant(
                &mut tx,
                id,
                &principal,
                VaultRole::Owner,
                now,
                actor.as_ref(),
            )
            .await?;
            emit_membership(
                &mut tx,
                id,
                &principal,
                None,
                Some(VaultRole::Owner),
                now,
                mutation,
            )
            .await?;
        }
        tx.commit().await.map_err(db)?;
        Ok(created)
    }

    async fn visible_vaults(
        &self,
        user: &UserId,
        deleted: bool,
    ) -> Result<Vec<(Vault, VaultRole)>> {
        let state = if deleted {
            "v.deleted_at IS NOT NULL"
        } else {
            "v.deleted_at IS NULL"
        };
        let ownership = if deleted {
            format!("HAVING MAX({ROLE_RANK}) = 4")
        } else {
            String::new()
        };
        let rows: Vec<RankedVaultRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT v.id, v.name, v.created_by, v.created_at, v.updated_at, v.updated_by, v.deleted_at, MAX({ROLE_RANK}) FROM vaults v JOIN vault_members m ON m.vault_id = v.id WHERE {APPLIES_TO_USER} AND {state} GROUP BY v.id {ownership} ORDER BY v.name, v.id"
        ))).bind(user.as_str()).fetch_all(&self.pool).await.map_err(db)?;
        rows.into_iter()
            .map(|(id, name, by, c, u, updated_by, deleted_at, rank)| {
                Ok((
                    vault((id, name, by, c, u, updated_by, deleted_at))?,
                    rank_role(rank),
                ))
            })
            .collect()
    }
}

#[async_trait]
impl VaultRepository for SqliteVaultRepository {
    async fn create(&self, name: &str, owner: Option<&UserId>) -> Result<Vault> {
        self.insert_vault(name, owner).await
    }

    async fn find(&self, id: VaultId) -> Result<Option<Vault>> {
        let row: Option<VaultRow> = sqlx::query_as("SELECT id, name, created_by, created_at, updated_at, updated_by, deleted_at FROM vaults WHERE id = $1 AND deleted_at IS NULL")
            .bind(id.to_string()).fetch_optional(&self.pool).await.map_err(db)?;
        row.map(vault).transpose()
    }

    async fn list_for_user(&self, user: &UserId) -> Result<Vec<(Vault, VaultRole)>> {
        self.visible_vaults(user, false).await
    }

    async fn list_deleted_for_user(&self, user: &UserId) -> Result<Vec<Vault>> {
        Ok(self
            .visible_vaults(user, true)
            .await?
            .into_iter()
            .map(|(vault, _)| vault)
            .collect())
    }

    async fn rename(&self, id: VaultId, name: &str) -> Result<Vault> {
        let name = checked_name(name)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let mut changed = locked_vault(&mut tx, id, false).await?;
        authorize_actor(&mut tx, id).await?;
        if changed.name == name {
            return Ok(changed);
        }
        changed.name = name;
        changed.updated_at = Utc::now();
        changed.updated_by = current_actor();
        sqlx::query("UPDATE vaults SET name = $2, updated_at = $3, updated_by = $4 WHERE id = $1")
            .bind(id.to_string())
            .bind(&changed.name)
            .bind(changed.updated_at.to_rfc3339())
            .bind(changed.updated_by.as_ref().map(UserId::as_str))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        emit_vault(&mut tx, &changed, "updated", Uuid::new_v4()).await?;
        tx.commit().await.map_err(db)?;
        Ok(changed)
    }

    async fn delete(&self, id: VaultId) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let mut changed = locked_vault(&mut tx, id, true).await?;
        authorize_actor(&mut tx, id).await?;
        if changed.deleted_at.is_some() {
            return Ok(());
        }
        let (defaults,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM users WHERE id = $2 AND default_vault_id = $1")
                .bind(id.to_string())
                .bind(current_actor().as_ref().map(UserId::as_str))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        let instance: Option<(String,)> = sqlx::query_as("SELECT COALESCE((SELECT v.id FROM users u JOIN vaults v ON v.id = u.default_vault_id WHERE u.is_owner = 1 AND v.deleted_at IS NULL AND EXISTS (SELECT 1 FROM vault_members dm WHERE dm.vault_id = v.id AND (dm.user_id = u.id OR dm.group_id IN (SELECT group_id FROM auth_group_members WHERE user_id = u.id))) ORDER BY u.created_at, u.id LIMIT 1), (SELECT id FROM vaults WHERE deleted_at IS NULL ORDER BY created_at, id LIMIT 1))")
            .fetch_optional(&mut *tx).await.map_err(db)?;
        if defaults > 0 || instance.is_some_and(|(current,)| current == id.to_string()) {
            return Err(Error::Conflict(
                "Choose another default vault before deleting this vault".into(),
            ));
        }
        let now = Utc::now();
        changed.updated_at = now;
        changed.updated_by = current_actor();
        changed.deleted_at = Some(now);
        sqlx::query(
            "UPDATE vaults SET deleted_at = $2, updated_at = $2, updated_by = $3 WHERE id = $1",
        )
        .bind(id.to_string())
        .bind(now.to_rfc3339())
        .bind(changed.updated_by.as_ref().map(UserId::as_str))
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("UPDATE workflow_runs SET status = 'failed', finished_at = $2, last_error = 'Vault deleted before workflow execution completed' WHERE status = 'pending' AND workflow_id IN (SELECT id FROM workflows WHERE vault_id = $1)")
            .bind(id.to_string()).bind(now.to_rfc3339()).execute(&mut *tx).await.map_err(db)?;
        emit_vault(&mut tx, &changed, "deleted", Uuid::new_v4()).await?;
        tx.commit().await.map_err(db)
    }

    async fn restore(&self, id: VaultId) -> Result<Vault> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let mut changed = locked_vault(&mut tx, id, true).await?;
        authorize_actor(&mut tx, id).await?;
        if changed.deleted_at.is_none() {
            return Ok(changed);
        }
        changed.updated_at = Utc::now();
        changed.updated_by = current_actor();
        changed.deleted_at = None;
        sqlx::query(
            "UPDATE vaults SET deleted_at = NULL, updated_at = $2, updated_by = $3 WHERE id = $1",
        )
        .bind(id.to_string())
        .bind(changed.updated_at.to_rfc3339())
        .bind(changed.updated_by.as_ref().map(UserId::as_str))
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        emit_vault(&mut tx, &changed, "restored", Uuid::new_v4()).await?;
        tx.commit().await.map_err(db)?;
        Ok(changed)
    }

    async fn default_for_user(&self, user: &UserId) -> Result<Option<VaultId>> {
        let stored: Option<(Option<String>,)> =
            sqlx::query_as("SELECT default_vault_id FROM users WHERE id = $1")
                .bind(user.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        if let Some(stored) = stored.and_then(|(id,)| id) {
            let id = vault_id(&stored)?;
            if self.effective_role(user, id).await?.is_some() {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    async fn set_default(&self, user: &UserId, id: VaultId) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        locked_vault(&mut tx, id, false).await?;
        if current_role(&mut tx, user, id).await?.is_none() {
            return Err(not_found(id));
        }
        sqlx::query("UPDATE users SET default_vault_id = $2 WHERE id = $1")
            .bind(user.as_str())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    async fn provisioned(&self, user: &UserId) -> Result<bool> {
        sqlx::query_scalar("SELECT vaults_provisioned <> 0 FROM users WHERE id = $1")
            .bind(user.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)
            .map(|value| value.unwrap_or(false))
    }

    async fn mark_provisioned(&self, user: &UserId) -> Result<()> {
        sqlx::query("UPDATE users SET vaults_provisioned = 1 WHERE id = $1")
            .bind(user.as_str())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn instance_default(&self) -> Result<VaultId> {
        let owners: Option<(String,)> = sqlx::query_as("SELECT v.id FROM users u JOIN vaults v ON v.id = u.default_vault_id WHERE u.is_owner = 1 AND v.deleted_at IS NULL AND EXISTS (SELECT 1 FROM vault_members dm WHERE dm.vault_id = v.id AND (dm.user_id = u.id OR dm.group_id IN (SELECT group_id FROM auth_group_members WHERE user_id = u.id))) ORDER BY u.created_at, u.id LIMIT 1")
            .fetch_optional(&self.pool).await.map_err(db)?;
        if let Some((id,)) = owners {
            return vault_id(&id);
        }
        let oldest: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM vaults WHERE deleted_at IS NULL ORDER BY created_at, id LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        if let Some((id,)) = oldest {
            return vault_id(&id);
        }
        Ok(self.insert_vault("Archive", None).await?.id)
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
        _granted_by: Option<&UserId>,
    ) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        locked_vault(&mut tx, vault, false).await?;
        authorize_actor(&mut tx, vault).await?;
        let (column, subject, table, entity) = principal_parts(principal);
        let known: Option<(i32,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT 1 FROM {table} WHERE id = $1"
        )))
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        if known.is_none() {
            return Err(Error::NotFound {
                entity_type: entity,
                id: subject.into(),
            });
        }
        let previous: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT role FROM vault_members WHERE vault_id = $1 AND {column} = $2"
        )))
        .bind(vault.to_string())
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let previous = previous.map(|(value,)| role(&value)).transpose()?;
        if previous == Some(new_role) {
            return Ok(());
        }
        if matches!(principal, Principal::User(_)) && new_role != VaultRole::Owner {
            guard_last_owner(&mut tx, vault, subject).await?;
        }
        let now = Utc::now();
        let actor = current_actor();
        write_grant(&mut tx, vault, principal, new_role, now, actor.as_ref()).await?;
        emit_membership(
            &mut tx,
            vault,
            principal,
            previous,
            Some(new_role),
            now,
            Uuid::new_v4(),
        )
        .await?;
        tx.commit().await.map_err(db)
    }

    async fn revoke(&self, vault: VaultId, principal: &Principal) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        locked_vault(&mut tx, vault, false).await?;
        authorize_actor(&mut tx, vault).await?;
        let (column, subject, _, _) = principal_parts(principal);
        let previous: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT role FROM vault_members WHERE vault_id = $1 AND {column} = $2"
        )))
        .bind(vault.to_string())
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let Some((previous,)) = previous else {
            return Ok(());
        };
        if matches!(principal, Principal::User(_)) {
            guard_last_owner(&mut tx, vault, subject).await?;
        }
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM vault_members WHERE vault_id = $1 AND {column} = $2"
        )))
        .bind(vault.to_string())
        .bind(subject)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        emit_membership(
            &mut tx,
            vault,
            principal,
            Some(role(&previous)?),
            None,
            Utc::now(),
            Uuid::new_v4(),
        )
        .await?;
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
        let mut conn = self.pool.acquire().await.map_err(db)?;
        current_role(&mut conn, user, vault).await
    }
}

fn checked_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 255 {
        return Err(Error::Validation(
            "Vault name must contain 1 to 255 characters".into(),
        ));
    }
    Ok(name.into())
}

fn principal_parts(principal: &Principal) -> (&'static str, &str, &'static str, &'static str) {
    match principal {
        Principal::User(user) => ("user_id", user.as_str(), "users", "User"),
        Principal::Group(group) => ("group_id", group.0.as_str(), "auth_groups", "Group"),
    }
}

async fn locked_vault(
    conn: &mut SqliteConnection,
    id: VaultId,
    include_deleted: bool,
) -> Result<Vault> {
    let row: Option<VaultRow> = sqlx::query_as("SELECT id, name, created_by, created_at, updated_at, updated_by, deleted_at FROM vaults WHERE id = $1 AND ($2 OR deleted_at IS NULL)")
        .bind(id.to_string()).bind(include_deleted).fetch_optional(conn).await.map_err(db)?;
    row.map(vault).transpose()?.ok_or_else(|| not_found(id))
}

async fn current_role(
    conn: &mut SqliteConnection,
    user: &UserId,
    vault: VaultId,
) -> Result<Option<VaultRole>> {
    let (rank,): (Option<i64>,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT MAX({ROLE_RANK}) FROM vault_members m JOIN vaults v ON v.id = m.vault_id WHERE m.vault_id = $2 AND v.deleted_at IS NULL AND {APPLIES_TO_USER}"
    ))).bind(user.as_str()).bind(vault.to_string()).fetch_one(conn).await.map_err(db)?;
    Ok(rank.map(rank_role))
}

async fn write_grant(
    conn: &mut SqliteConnection,
    vault: VaultId,
    principal: &Principal,
    role: VaultRole,
    now: DateTime<Utc>,
    actor: Option<&UserId>,
) -> Result<()> {
    let (column, subject, _, _) = principal_parts(principal);
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO vault_members (vault_id, principal_type, {column}, role, granted_by, granted_at) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (vault_id, {column}) WHERE {column} IS NOT NULL DO UPDATE SET role = excluded.role, granted_by = excluded.granted_by, granted_at = excluded.granted_at"
    ))).bind(vault.to_string()).bind(if column == "user_id" { "user" } else { "group" }).bind(subject).bind(role.as_str())
        .bind(actor.map(UserId::as_str)).bind(now.to_rfc3339()).execute(conn).await.map_err(db)?;
    Ok(())
}

async fn emit_vault(
    conn: &mut SqliteConnection,
    vault: &Vault,
    action: &str,
    mutation: Uuid,
) -> Result<()> {
    let envelope = DomainEventEnvelope::new(format!("vault.{action}"), vault.id,
        DomainEventSubject { subject_type: "vault".into(), id: vault.id.0 }, None, EventAudience::Owners,
        serde_json::json!({ "name": vault.name, "created_at": vault.created_at, "updated_at": vault.updated_at,
            "created_by": vault.created_by, "updated_by": vault.updated_by, "deleted_at": vault.deleted_at }),
    )?.with_mutation(mutation)?;
    record_lifecycle_audit(&mut *conn, &envelope).await?;
    insert_domain_event(conn, &envelope).await?;
    Ok(())
}

async fn emit_membership(
    conn: &mut SqliteConnection,
    vault: VaultId,
    principal: &Principal,
    before: Option<VaultRole>,
    after: Option<VaultRole>,
    at: DateTime<Utc>,
    mutation: Uuid,
) -> Result<()> {
    let (column, subject, _, _) = principal_parts(principal);
    let kind = if column == "user_id" { "user" } else { "group" };
    let action = match (before, after) {
        (None, Some(_)) => "granted",
        (Some(_), Some(_)) => "role_changed",
        (Some(_), None) => "revoked",
        (None, None) => return Ok(()),
    };
    let envelope = DomainEventEnvelope::new(format!("vault_membership.{action}"), vault,
        DomainEventSubject { subject_type: "vault_membership".into(), id: Uuid::new_v5(&vault.0, format!("{kind}:{subject}").as_bytes()) },
        None, EventAudience::Owners, serde_json::json!({ "principal_type": kind, "principal_id": subject, "before_role": before.map(VaultRole::as_str), "role": after.map(VaultRole::as_str), "at": at }),
    )?.with_mutation(mutation)?;
    record_lifecycle_audit(&mut *conn, &envelope).await?;
    insert_domain_event(conn, &envelope).await?;
    Ok(())
}

async fn authorize_actor(conn: &mut SqliteConnection, vault: VaultId) -> Result<()> {
    let Some(actor) = current_actor() else {
        return Ok(());
    };
    let (rank,): (Option<i64>,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT MAX({ROLE_RANK}) FROM vault_members m WHERE m.vault_id = $2 AND {APPLIES_TO_USER}"
    )))
    .bind(actor.as_str())
    .bind(vault.to_string())
    .fetch_one(conn)
    .await
    .map_err(db)?;
    if rank == Some(4) {
        Ok(())
    } else {
        Err(not_found(vault))
    }
}

async fn record_lifecycle_audit(
    conn: &mut SqliteConnection,
    envelope: &DomainEventEnvelope,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_events (id, at, actor_user_id, action, vault_id, target_type, target_id, details) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
        .bind(envelope.event_id().to_string())
        .bind(envelope.occurred_at().to_rfc3339())
        .bind(envelope.actor_id().map(UserId::as_str)).bind(envelope.event_type())
        .bind(envelope.vault_id().to_string())
        .bind(&envelope.subject().subject_type).bind(envelope.subject().id.to_string())
        .bind(envelope.data().to_string())
        .execute(conn).await.map_err(db)?;
    Ok(())
}

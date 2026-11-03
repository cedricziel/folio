use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::{Id, MetadataOption, MetadataOptionInput, UserId, VaultId};
use folio_core::ports::MetadataOptionRepository;
use folio_core::{Error, Result};
use sqlx::{SqliteConnection, SqlitePool};
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct SqliteMetadataOptionRepository {
    pool: SqlitePool,
    wake: Arc<Notify>,
}
impl SqliteMetadataOptionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            wake: Arc::new(Notify::new()),
        }
    }
    pub fn with_wake(mut self, wake: Arc<Notify>) -> Self {
        self.wake = wake;
        self
    }
}

type DbId = String;
fn db_id(id: Id) -> DbId {
    id.to_string()
}
fn read_id(id: DbId) -> Result<Id> {
    Id::parse_str(&id).map_err(|e| Error::Repository(e.to_string()))
}

#[derive(sqlx::FromRow)]
struct OptionRow {
    id: DbId,
    vault_id: DbId,
    field_id: DbId,
    label: String,
    position: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
    created_by: Option<String>,
    updated_by: Option<String>,
}
impl TryFrom<OptionRow> for MetadataOption {
    type Error = Error;
    fn try_from(row: OptionRow) -> Result<Self> {
        Ok(Self {
            id: read_id(row.id)?,
            vault_id: VaultId(read_id(row.vault_id)?),
            field_id: read_id(row.field_id)?,
            label: row.label,
            position: row
                .position
                .try_into()
                .map_err(|_| Error::Repository("Invalid option position".into()))?,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}
fn db_error(error: sqlx::Error) -> Error {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation() || e.is_foreign_key_violation())
    {
        Error::Conflict("Metadata option conflicts with a retained label or reference".into())
    } else {
        Error::Repository(error.to_string())
    }
}
fn missing(id: Id) -> Error {
    Error::NotFound {
        entity_type: "Metadata option",
        id: id.to_string(),
    }
}
async fn lock_field(conn: &mut SqliteConnection, vault: VaultId, field: Id) -> Result<()> {
    let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT kind, deleted_at FROM metadata_fields WHERE vault_id = ? AND id = ?",
    )
    .bind(db_id(vault.0))
    .bind(db_id(field))
    .fetch_optional(conn)
    .await
    .map_err(db_error)?;
    let (kind, deleted_at) = row.ok_or(Error::NotFound {
        entity_type: "Metadata field",
        id: field.to_string(),
    })?;
    if kind != "select" || deleted_at.is_some() {
        return Err(Error::Conflict(
            "Options require an active select field".into(),
        ));
    }
    Ok(())
}
async fn publish(
    conn: &mut SqliteConnection,
    option: &MetadataOption,
    event_type: &str,
) -> Result<()> {
    let event = DomainEventEnvelope::new(
        event_type,
        option.vault_id,
        DomainEventSubject {
            subject_type: "metadata_option".into(),
            id: option.id,
        },
        None,
        EventAudience::Vault,
        serde_json::json!({"option_id": option.id, "field_id": option.field_id, "label": option.label,
            "position": option.position, "deleted_at": option.deleted_at}),
    )?;
    super::insert_domain_event(conn, &event).await?;
    Ok(())
}
#[async_trait]
impl MetadataOptionRepository for SqliteMetadataOptionRepository {
    async fn list(
        &self,
        vault: VaultId,
        field: Id,
        include_deleted: bool,
        after: Option<Id>,
        limit: u32,
    ) -> Result<Vec<MetadataOption>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Validation(
                "Page limit must be between 1 and 100".into(),
            ));
        }
        let rows: Vec<OptionRow> = sqlx::query_as("SELECT id, vault_id, field_id, label, position, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_options WHERE vault_id = ? AND field_id = ? AND (deleted_at IS NULL OR ?) AND id > ? ORDER BY id LIMIT ?")
            .bind(db_id(vault.0)).bind(db_id(field)).bind(include_deleted).bind(db_id(after.unwrap_or_else(Id::nil))).bind(i64::from(limit))
            .fetch_all(&self.pool).await.map_err(db_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn find(&self, vault: VaultId, field: Id, id: Id) -> Result<Option<MetadataOption>> {
        let row: Option<OptionRow> = sqlx::query_as("SELECT id, vault_id, field_id, label, position, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_options WHERE vault_id = ? AND field_id = ? AND id = ?")
            .bind(db_id(vault.0)).bind(db_id(field)).bind(db_id(id)).fetch_optional(&self.pool).await.map_err(db_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn create(
        &self,
        vault: VaultId,
        field: Id,
        input: MetadataOptionInput,
    ) -> Result<MetadataOption> {
        let input = input.validate()?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        lock_field(&mut tx, vault, field).await?;
        let now = Utc::now().trunc_subsecs(6);
        let actor = folio_core::mutation::current_actor();
        let option = MetadataOption {
            id: Id::new_v4(),
            vault_id: vault,
            field_id: field,
            label: input.label,
            position: input.position,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: actor.clone(),
            updated_by: actor,
        };
        sqlx::query("INSERT INTO metadata_options (id, vault_id, field_id, label, label_key, position, created_at, updated_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(db_id(option.id)).bind(db_id(vault.0)).bind(db_id(field)).bind(&option.label).bind(option.label.to_lowercase()).bind(i64::from(option.position))
            .bind(now).bind(now).bind(option.created_by.as_ref().map(UserId::as_str)).bind(option.updated_by.as_ref().map(UserId::as_str)).execute(&mut *tx).await.map_err(db_error)?;
        publish(&mut tx, &option, "metadata_option.created").await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(option)
    }
    async fn update(
        &self,
        vault: VaultId,
        field: Id,
        id: Id,
        input: MetadataOptionInput,
    ) -> Result<MetadataOption> {
        let input = input.validate()?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        lock_field(&mut tx, vault, field).await?;
        let row: OptionRow = sqlx::query_as("SELECT id, vault_id, field_id, label, position, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_options WHERE vault_id = ? AND field_id = ? AND id = ?")
            .bind(db_id(vault.0)).bind(db_id(field)).bind(db_id(id)).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(|| missing(id))?;
        let mut option: MetadataOption = row.try_into()?;
        if option.deleted_at.is_some() {
            return Err(Error::Conflict(
                "Restore the option before editing it".into(),
            ));
        }
        if option.label == input.label && option.position == input.position {
            tx.commit().await.map_err(db_error)?;
            return Ok(option);
        }
        option.label = input.label;
        option.position = input.position;
        option.updated_at = Utc::now().trunc_subsecs(6);
        option.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE metadata_options SET label = ?, label_key = ?, position = ?, updated_at = ?, updated_by = ? WHERE vault_id = ? AND field_id = ? AND id = ?")
            .bind(&option.label).bind(option.label.to_lowercase()).bind(i64::from(option.position)).bind(option.updated_at).bind(option.updated_by.as_ref().map(UserId::as_str))
            .bind(db_id(vault.0)).bind(db_id(field)).bind(db_id(id)).execute(&mut *tx).await.map_err(db_error)?;
        publish(&mut tx, &option, "metadata_option.updated").await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(option)
    }
    async fn set_deleted(
        &self,
        vault: VaultId,
        field: Id,
        id: Id,
        deleted: bool,
    ) -> Result<MetadataOption> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        lock_field(&mut tx, vault, field).await?;
        let row: OptionRow = sqlx::query_as("SELECT id, vault_id, field_id, label, position, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_options WHERE vault_id = ? AND field_id = ? AND id = ?")
            .bind(db_id(vault.0)).bind(db_id(field)).bind(db_id(id)).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(|| missing(id))?;
        let mut option: MetadataOption = row.try_into()?;
        if option.deleted_at.is_some() == deleted {
            tx.commit().await.map_err(db_error)?;
            return Ok(option);
        }
        option.updated_at = Utc::now().trunc_subsecs(6);
        option.deleted_at = deleted.then_some(option.updated_at);
        option.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE metadata_options SET deleted_at = ?, updated_at = ?, updated_by = ? WHERE vault_id = ? AND field_id = ? AND id = ?")
            .bind(option.deleted_at).bind(option.updated_at).bind(option.updated_by.as_ref().map(UserId::as_str)).bind(db_id(vault.0)).bind(db_id(field)).bind(db_id(id))
            .execute(&mut *tx).await.map_err(db_error)?;
        publish(
            &mut tx,
            &option,
            if deleted {
                "metadata_option.deleted"
            } else {
                "metadata_option.restored"
            },
        )
        .await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(option)
    }
}

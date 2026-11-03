use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::{Id, MetadataField, MetadataFieldInput, UserId, VaultId};
use folio_core::ports::MetadataFieldRepository;
use folio_core::{Error, Result};
use sqlx::{SqliteConnection, SqlitePool};
use tokio::sync::Notify;

#[derive(Clone)]
pub struct SqliteMetadataFieldRepository {
    pool: SqlitePool,
    wake: Arc<Notify>,
}

impl SqliteMetadataFieldRepository {
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

#[derive(sqlx::FromRow)]
struct FieldRow {
    id: DbId,
    vault_id: DbId,
    name: String,
    kind: String,
    is_required: bool,
    is_unique: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

fn db_id(id: Id) -> DbId {
    id.to_string()
}
fn read_id(id: DbId) -> Result<Id> {
    Id::parse_str(&id).map_err(|e| Error::Repository(e.to_string()))
}

impl TryFrom<FieldRow> for MetadataField {
    type Error = Error;
    fn try_from(row: FieldRow) -> Result<Self> {
        Ok(Self {
            id: read_id(row.id)?,
            vault_id: VaultId(read_id(row.vault_id)?),
            name: row.name,
            kind: serde_json::from_value(serde_json::Value::String(row.kind))
                .map_err(|e| Error::Repository(e.to_string()))?,
            required: row.is_required,
            unique: row.is_unique,
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
        Error::Conflict("Metadata field conflicts with a retained name or reference".into())
    } else {
        Error::Repository(error.to_string())
    }
}

fn missing(id: Id) -> Error {
    Error::NotFound {
        entity_type: "Metadata field",
        id: id.to_string(),
    }
}

async fn publish(
    conn: &mut SqliteConnection,
    field: &MetadataField,
    event_type: &str,
) -> Result<()> {
    let event = DomainEventEnvelope::new(
        event_type,
        field.vault_id,
        DomainEventSubject {
            subject_type: "metadata_field".into(),
            id: field.id,
        },
        None,
        EventAudience::Vault,
        serde_json::json!({"field_id": field.id, "name": field.name, "kind": field.kind,
            "required": field.required, "unique": field.unique, "deleted_at": field.deleted_at}),
    )?;
    super::insert_domain_event(conn, &event).await?;
    Ok(())
}

#[async_trait]
impl MetadataFieldRepository for SqliteMetadataFieldRepository {
    async fn list(
        &self,
        vault: VaultId,
        include_deleted: bool,
        after: Option<Id>,
        limit: u32,
    ) -> Result<Vec<MetadataField>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Validation(
                "Page limit must be between 1 and 100".into(),
            ));
        }
        let rows: Vec<FieldRow> = sqlx::query_as("SELECT id, vault_id, name, kind, is_required, is_unique, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_fields WHERE vault_id = ? AND (deleted_at IS NULL OR ?) AND id > ? ORDER BY id LIMIT ?")
            .bind(db_id(vault.0)).bind(include_deleted).bind(db_id(after.unwrap_or_else(Id::nil))).bind(i64::from(limit))
            .fetch_all(&self.pool).await.map_err(db_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn find(&self, vault: VaultId, id: Id) -> Result<Option<MetadataField>> {
        let row: Option<FieldRow> = sqlx::query_as("SELECT id, vault_id, name, kind, is_required, is_unique, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_fields WHERE vault_id = ? AND id = ?")
            .bind(db_id(vault.0)).bind(db_id(id)).fetch_optional(&self.pool).await.map_err(db_error)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn create(&self, vault: VaultId, input: MetadataFieldInput) -> Result<MetadataField> {
        let input = input.validate()?;
        let now = Utc::now().trunc_subsecs(6);
        let actor = folio_core::mutation::current_actor();
        let field = MetadataField {
            id: Id::new_v4(),
            vault_id: vault,
            name: input.name,
            kind: input.kind,
            required: input.required,
            unique: input.unique,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: actor.clone(),
            updated_by: actor,
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query("INSERT INTO metadata_fields (id, vault_id, name, name_key, kind, is_required, is_unique, created_at, updated_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(db_id(field.id)).bind(db_id(vault.0)).bind(&field.name).bind(field.name.to_lowercase())
            .bind(field.kind.as_str()).bind(field.required).bind(field.unique)
            .bind(now).bind(now).bind(field.created_by.as_ref().map(UserId::as_str)).bind(field.updated_by.as_ref().map(UserId::as_str))
            .execute(&mut *tx).await.map_err(db_error)?;
        publish(&mut tx, &field, "metadata_field.created").await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(field)
    }

    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        input: MetadataFieldInput,
    ) -> Result<MetadataField> {
        let input = input.validate()?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        let row: FieldRow = sqlx::query_as("SELECT id, vault_id, name, kind, is_required, is_unique, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_fields WHERE vault_id = ? AND id = ?")
            .bind(db_id(vault.0)).bind(db_id(id)).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(|| missing(id))?;
        let mut field: MetadataField = row.try_into()?;
        if field.deleted_at.is_some() {
            return Err(Error::Conflict(
                "Restore the field before editing it".into(),
            ));
        }
        if field.name == input.name
            && field.kind == input.kind
            && field.required == input.required
            && field.unique == input.unique
        {
            tx.commit().await.map_err(db_error)?;
            return Ok(field);
        }
        field.name = input.name;
        field.kind = input.kind;
        field.required = input.required;
        field.unique = input.unique;
        field.updated_at = Utc::now().trunc_subsecs(6);
        field.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE metadata_fields SET name = ?, name_key = ?, kind = ?, is_required = ?, is_unique = ?, updated_at = ?, updated_by = ? WHERE vault_id = ? AND id = ?")
            .bind(&field.name).bind(field.name.to_lowercase()).bind(field.kind.as_str()).bind(field.required).bind(field.unique)
            .bind(field.updated_at).bind(field.updated_by.as_ref().map(UserId::as_str)).bind(db_id(vault.0)).bind(db_id(id))
            .execute(&mut *tx).await.map_err(db_error)?;
        publish(&mut tx, &field, "metadata_field.updated").await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(field)
    }

    async fn set_deleted(&self, vault: VaultId, id: Id, deleted: bool) -> Result<MetadataField> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        let row: FieldRow = sqlx::query_as("SELECT id, vault_id, name, kind, is_required, is_unique, created_at, updated_at, deleted_at, created_by, updated_by FROM metadata_fields WHERE vault_id = ? AND id = ?")
            .bind(db_id(vault.0)).bind(db_id(id)).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(|| missing(id))?;
        let mut field: MetadataField = row.try_into()?;
        if field.deleted_at.is_some() == deleted {
            tx.commit().await.map_err(db_error)?;
            return Ok(field);
        }
        field.updated_at = Utc::now().trunc_subsecs(6);
        field.deleted_at = deleted.then_some(field.updated_at);
        field.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE metadata_fields SET deleted_at = ?, updated_at = ?, updated_by = ? WHERE vault_id = ? AND id = ?")
            .bind(field.deleted_at).bind(field.updated_at).bind(field.updated_by.as_ref().map(UserId::as_str)).bind(db_id(vault.0)).bind(db_id(id))
            .execute(&mut *tx).await.map_err(db_error)?;
        publish(
            &mut tx,
            &field,
            if deleted {
                "metadata_field.deleted"
            } else {
                "metadata_field.restored"
            },
        )
        .await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(field)
    }
}

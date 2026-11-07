use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::{
    DocumentMetadata, DocumentMetadataValue, Id, MetadataChange, MetadataValue, UserId, VaultId,
};
use folio_core::ports::MetadataValueRepository;
use folio_core::{Error, Result};
use sqlx::{PgConnection, PgPool};
use tokio::sync::Notify;

use crate::database::metadata_value::{canonical_key, change, parse_kind, Columns};

const VALUE_COLS: &str = "field_id, kind, text_value, boolean_value, integer_value, decimal_value, date_value, currency, option_id, target_document_id, created_at, updated_at, deleted_at, created_by, updated_by";

#[derive(Clone)]
pub struct PostgresMetadataValueRepository {
    pool: PgPool,
    wake: Arc<Notify>,
}

impl PostgresMetadataValueRepository {
    pub fn new(pool: PgPool) -> Self {
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

#[derive(sqlx::FromRow)]
struct ValueRow {
    field_id: Id,
    kind: String,
    text_value: Option<String>,
    boolean_value: Option<bool>,
    integer_value: Option<i64>,
    decimal_value: Option<String>,
    date_value: Option<String>,
    currency: Option<String>,
    option_id: Option<Id>,
    target_document_id: Option<Id>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl TryFrom<ValueRow> for DocumentMetadataValue {
    type Error = Error;
    fn try_from(row: ValueRow) -> Result<Self> {
        let kind = parse_kind(&row.kind)?;
        let value = Columns {
            text: row.text_value,
            boolean: row.boolean_value,
            integer: row.integer_value,
            decimal: row.decimal_value,
            date: row.date_value,
            currency: row.currency,
            option: row.option_id,
            target: row.target_document_id,
        }
        .into_value(kind, row.deleted_at.is_some())?;
        Ok(Self {
            field_id: row.field_id,
            value,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}

fn db_error(error: sqlx::Error) -> Error {
    if error.as_database_error().is_some_and(|e| {
        e.is_unique_violation()
            || e.is_foreign_key_violation()
            || matches!(e.code().as_deref(), Some("40001" | "40P01"))
    }) {
        Error::Conflict("Metadata value conflicts with a retained value or reference".into())
    } else {
        Error::Repository(error.to_string())
    }
}

fn not_found(entity_type: &'static str, id: Id) -> Error {
    Error::NotFound {
        entity_type,
        id: id.to_string(),
    }
}

async fn document_state(
    conn: &mut PgConnection,
    vault: VaultId,
    document: Id,
    lock: &str,
) -> Result<Option<bool>> {
    let row: Option<(bool,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT deleted_at IS NULL AND purge_requested_at IS NULL FROM documents WHERE vault_id = $1 AND id = $2 {lock}"
    )))
    .bind(vault.0)
    .bind(document)
    .fetch_optional(conn)
    .await
    .map_err(db_error)?;
    Ok(row.map(|(active,)| active))
}

async fn current_revision(
    conn: &mut PgConnection,
    vault: VaultId,
    document: Id,
) -> Result<Option<Id>> {
    let row: Option<(Id,)> = sqlx::query_as(
        "SELECT revision FROM document_metadata_revisions WHERE vault_id = $1 AND document_id = $2",
    )
    .bind(vault.0)
    .bind(document)
    .fetch_optional(conn)
    .await
    .map_err(db_error)?;
    Ok(row.map(|(revision,)| revision))
}

async fn load(conn: &mut PgConnection, vault: VaultId, document: Id) -> Result<DocumentMetadata> {
    if document_state(conn, vault, document, "").await?.is_none() {
        return Err(not_found("Document", document));
    }
    let revision = current_revision(conn, vault, document).await?;
    let rows: Vec<ValueRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {VALUE_COLS} FROM metadata_values WHERE vault_id = $1 AND document_id = $2 ORDER BY field_id"
    )))
    .bind(vault.0)
    .bind(document)
    .fetch_all(conn)
    .await
    .map_err(db_error)?;
    Ok(DocumentMetadata {
        document_id: document,
        revision,
        values: rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_>>()?,
    })
}

async fn bump_revision(conn: &mut PgConnection, vault: VaultId, document: Id) -> Result<Id> {
    let revision = Id::new_v4();
    sqlx::query(
        "INSERT INTO document_metadata_revisions (vault_id, document_id, revision) VALUES ($1, $2, $3)
         ON CONFLICT (vault_id, document_id) DO UPDATE SET revision = excluded.revision",
    )
    .bind(vault.0)
    .bind(document)
    .bind(revision)
    .execute(conn)
    .await
    .map_err(db_error)?;
    Ok(revision)
}

async fn publish(
    conn: &mut PgConnection,
    vault: VaultId,
    document: Id,
    revision: Id,
    changes: Vec<serde_json::Value>,
) -> Result<()> {
    let event = DomainEventEnvelope::new(
        folio_core::domain_event::DOCUMENT_METADATA_CHANGED,
        vault,
        DomainEventSubject {
            subject_type: "document".into(),
            id: document,
        },
        Some(document),
        EventAudience::Vault,
        serde_json::json!({"revision": revision, "changes": changes}),
    )?;
    super::insert_domain_event(conn, &event).await?;
    Ok(())
}

async fn check_reference(
    conn: &mut PgConnection,
    vault: VaultId,
    field: Id,
    value: &MetadataValue,
) -> Result<()> {
    match value {
        MetadataValue::Select(option) => {
            let row: Option<(bool,)> = sqlx::query_as(
                "SELECT deleted_at IS NULL FROM metadata_options WHERE vault_id = $1 AND field_id = $2 AND id = $3 FOR SHARE",
            )
            .bind(vault.0)
            .bind(field)
            .bind(option)
            .fetch_optional(conn)
            .await
            .map_err(db_error)?;
            match row {
                None => Err(not_found("Metadata option", *option)),
                Some((false,)) => Err(Error::Conflict(
                    "Retired options cannot be newly assigned".into(),
                )),
                Some((true,)) => Ok(()),
            }
        }
        MetadataValue::DocumentLink(target) => {
            match document_state(conn, vault, *target, "FOR SHARE").await? {
                None => Err(not_found("Document", *target)),
                Some(false) => Err(Error::Conflict(
                    "Documents in the trash cannot receive new links".into(),
                )),
                Some(true) => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

struct FieldForWrite {
    kind_name: String,
    required: bool,
    unique: bool,
    live: bool,
}

async fn field_for_write(
    conn: &mut PgConnection,
    vault: VaultId,
    field: Id,
) -> Result<FieldForWrite> {
    let row: Option<(String, bool, bool, bool)> = sqlx::query_as(
        "SELECT kind, is_required, is_unique, deleted_at IS NULL FROM metadata_fields WHERE vault_id = $1 AND id = $2 FOR SHARE",
    )
    .bind(vault.0)
    .bind(field)
    .fetch_optional(conn)
    .await
    .map_err(db_error)?;
    let (kind_name, required, unique, live) =
        row.ok_or_else(|| not_found("Metadata field", field))?;
    Ok(FieldForWrite {
        kind_name,
        required,
        unique,
        live,
    })
}

#[allow(clippy::too_many_arguments)]
async fn write_value(
    conn: &mut PgConnection,
    vault: VaultId,
    document: Id,
    field: Id,
    kind: &str,
    unique: bool,
    value: Option<&MetadataValue>,
    exists: bool,
    actor: Option<&UserId>,
    now: DateTime<Utc>,
) -> Result<()> {
    let columns = Columns::from_value(value)?;
    let key = value.map(canonical_key).transpose()?;
    let deleted_at = value.is_none().then_some(now);
    let sql = if exists {
        "UPDATE metadata_values SET text_value = $6, boolean_value = $7,
             integer_value = $8, decimal_value = $9, date_value = $10, currency = $11, option_id = $12,
             target_document_id = $13, canonical_key = $14, updated_at = $15, deleted_at = $16, updated_by = $17
         WHERE vault_id = $1 AND document_id = $2 AND field_id = $3 AND kind = $4 AND is_unique = $5"
    } else {
        "INSERT INTO metadata_values (vault_id, document_id, field_id, kind, is_unique, text_value,
             boolean_value, integer_value, decimal_value, date_value, currency, option_id,
             target_document_id, canonical_key, created_at, updated_at, deleted_at, created_by, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15, $16, $17, $17)"
    };
    let result = sqlx::query(sql)
        .bind(vault.0)
        .bind(document)
        .bind(field)
        .bind(kind)
        .bind(unique)
        .bind(columns.text)
        .bind(columns.boolean)
        .bind(columns.integer)
        .bind(columns.decimal)
        .bind(columns.date)
        .bind(columns.currency)
        .bind(columns.option)
        .bind(columns.target)
        .bind(key)
        .bind(now)
        .bind(deleted_at)
        .bind(actor.map(UserId::as_str))
        .execute(conn)
        .await
        .map_err(db_error)?;
    if result.rows_affected() != 1 {
        return Err(Error::Repository(
            "Metadata value no longer matches its field".into(),
        ));
    }
    Ok(())
}

/// Write the values of a document created in the same transaction, without
/// announcing them: the document's `created` event covers them.
pub(crate) async fn insert_new_values(
    conn: &mut PgConnection,
    vault: VaultId,
    document: Id,
    values: &BTreeMap<Id, MetadataValue>,
    actor: Option<&UserId>,
    now: DateTime<Utc>,
) -> Result<()> {
    for (&field_id, value) in values {
        let FieldForWrite {
            kind_name,
            required: _,
            unique,
            live,
        } = field_for_write(&mut *conn, vault, field_id).await?;
        if !live {
            return Err(Error::Conflict(
                "Retired fields cannot receive new values".into(),
            ));
        }
        let value = parse_kind(&kind_name)?
            .validate_value(Some(value.clone()), false)?
            .ok_or_else(|| Error::Validation("Imported metadata value is empty".into()))?;
        check_reference(conn, vault, field_id, &value).await?;
        write_value(
            conn,
            vault,
            document,
            field_id,
            &kind_name,
            unique,
            Some(&value),
            false,
            actor,
            now,
        )
        .await?;
    }
    if !values.is_empty() {
        bump_revision(conn, vault, document).await?;
    }
    Ok(())
}

/// Clear links into a document that is about to be purged, attributed to the system.
pub(crate) async fn clear_incoming_links(
    conn: &mut PgConnection,
    vault: VaultId,
    target: Id,
) -> Result<()> {
    sqlx::query(
        "SELECT id FROM documents WHERE vault_id = $1 AND id <> $2 AND id IN
         (SELECT document_id FROM metadata_values WHERE vault_id = $1 AND target_document_id = $2 AND deleted_at IS NULL)
         ORDER BY id FOR UPDATE",
    )
    .bind(vault.0)
    .bind(target)
    .fetch_all(&mut *conn)
    .await
    .map_err(db_error)?;
    let rows: Vec<(Id, Id, bool)> = sqlx::query_as(
        "SELECT document_id, field_id, is_unique FROM metadata_values
         WHERE vault_id = $1 AND target_document_id = $2 AND deleted_at IS NULL ORDER BY document_id, field_id FOR UPDATE",
    )
    .bind(vault.0)
    .bind(target)
    .fetch_all(&mut *conn)
    .await
    .map_err(db_error)?;
    let mut sources: BTreeMap<Id, Vec<serde_json::Value>> = BTreeMap::new();
    for (document, field, unique) in rows {
        write_value(
            conn,
            vault,
            document,
            field,
            "document_link",
            unique,
            None,
            true,
            None,
            Utc::now().trunc_subsecs(6),
        )
        .await?;
        if document != target {
            sources.entry(document).or_default().push(change(
                field,
                folio_core::models::MetadataKind::DocumentLink,
                Some(&MetadataValue::DocumentLink(target)),
                None,
            ));
        }
    }
    for (document, changes) in sources {
        let revision = bump_revision(conn, vault, document).await?;
        folio_core::mutation::with_actor(None, publish(conn, vault, document, revision, changes))
            .await?;
    }
    Ok(())
}

/// Reject a vault move while values or incoming links remain, then drop cleared history.
pub(crate) async fn prepare_move(
    conn: &mut PgConnection,
    vault: VaultId,
    document: Id,
) -> Result<()> {
    let (retained,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM metadata_values
         WHERE vault_id = $1 AND (document_id = $2 OR target_document_id = $2) AND deleted_at IS NULL)",
    )
    .bind(vault.0)
    .bind(document)
    .fetch_one(&mut *conn)
    .await
    .map_err(db_error)?;
    if retained {
        return Err(Error::Conflict(
            "Clear the document's custom field values and incoming links before moving it".into(),
        ));
    }
    for table in ["metadata_values", "document_metadata_revisions"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE vault_id = $1 AND document_id = $2"
        )))
        .bind(vault.0)
        .bind(document)
        .execute(&mut *conn)
        .await
        .map_err(db_error)?;
    }
    Ok(())
}

#[async_trait]
impl MetadataValueRepository for PostgresMetadataValueRepository {
    async fn get(&self, vault: VaultId, document: Id) -> Result<DocumentMetadata> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let metadata = load(&mut tx, vault, document).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(metadata)
    }

    async fn patch(
        &self,
        vault: VaultId,
        document: Id,
        revision: Option<Id>,
        changes: Vec<MetadataChange>,
    ) -> Result<DocumentMetadata> {
        let mut seen = HashSet::new();
        if !changes.iter().all(|change| seen.insert(change.field_id)) {
            return Err(Error::Validation(
                "A patch may change each field only once".into(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        match document_state(&mut tx, vault, document, "FOR UPDATE").await? {
            None => return Err(not_found("Document", document)),
            Some(false) => {
                return Err(Error::Conflict(
                    "Restore the document before editing its custom fields".into(),
                ))
            }
            Some(true) => {}
        }
        if current_revision(&mut tx, vault, document).await? != revision {
            return Err(Error::Conflict(
                "Custom fields changed since they were read".into(),
            ));
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now().trunc_subsecs(6);
        let mut applied = Vec::new();
        for MetadataChange { field_id, value } in changes {
            let FieldForWrite {
                kind_name,
                required,
                unique,
                live,
            } = field_for_write(&mut tx, vault, field_id).await?;
            let kind = parse_kind(&kind_name)?;
            let existing: Option<ValueRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {VALUE_COLS} FROM metadata_values WHERE vault_id = $1 AND document_id = $2 AND field_id = $3"
            )))
            .bind(vault.0)
            .bind(document)
            .bind(field_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
            let exists = existing.is_some();
            let before = existing
                .map(DocumentMetadataValue::try_from)
                .transpose()?
                .and_then(|value| value.value);
            let after = kind.validate_value(value, required && live)?;
            if after == before {
                continue;
            }
            if let Some(after) = &after {
                if !live {
                    return Err(Error::Conflict(
                        "Retired fields cannot receive new values".into(),
                    ));
                }
                check_reference(&mut tx, vault, field_id, after).await?;
            }
            write_value(
                &mut tx,
                vault,
                document,
                field_id,
                &kind_name,
                unique,
                after.as_ref(),
                exists,
                actor.as_ref(),
                now,
            )
            .await?;
            applied.push(change(field_id, kind, before.as_ref(), after.as_ref()));
        }
        if !applied.is_empty() {
            let revision = bump_revision(&mut tx, vault, document).await?;
            publish(&mut tx, vault, document, revision, applied).await?;
        }
        let metadata = load(&mut tx, vault, document).await?;
        tx.commit().await.map_err(db_error)?;
        self.wake.notify_one();
        Ok(metadata)
    }
}

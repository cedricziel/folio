//! SQLite document repository implementation

use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use std::sync::Arc;

use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool, Transaction};
use tokio::sync::Notify;
use uuid::Uuid;

use super::outbox::{
    capture_document, insert_domain_event_with_origin, insert_event, insert_event_with_envelope,
};
use folio_core::document_transitions::{metadata_envelopes, move_envelopes};
use folio_core::error::{Error, Result};
use folio_core::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentSource, DocumentType, EntryKind, ExternalRef, Id, ImportedDocument,
    ProcessingStage, ProcessingStatus, ReviewOrigin, SourceKind, Tag, UpdateCorrespondent,
    UpdateDocument, UpdateDocumentType, UpdateTag, UserId, VaultId,
};
use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, TagRepository,
};
use folio_core::EventSnapshot;
use folio_core::{DeletedDocument, DocumentEvent, EventOrigin};

/// SQLite implementation of DocumentRepository
///
/// Lifecycle changes (create, update, delete, tag edits) write their outbox
/// event in the same transaction, so a change is never committed without it.
pub struct SqliteDocumentRepository {
    pool: SqlitePool,
    origin: EventOrigin,
    wake: Arc<Notify>,
}

impl SqliteDocumentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            origin: EventOrigin::User,
            wake: Arc::new(Notify::new()),
        }
    }

    /// Attribute the events of this repository's changes to `origin`
    pub fn with_origin(mut self, origin: EventOrigin) -> Self {
        self.origin = origin;
        self
    }

    /// Signal this when an event was committed
    pub fn with_wake(mut self, wake: Arc<Notify>) -> Self {
        self.wake = wake;
        self
    }

    async fn emit_in(&self, tx: &mut Transaction<'_, Sqlite>, event: DocumentEvent) -> Result<()> {
        self.emit_as(tx, event, self.origin).await
    }

    async fn emit_as(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        event: DocumentEvent,
        origin: EventOrigin,
    ) -> Result<()> {
        let context = capture_document(tx, event.document_id(), None)
            .await?
            .ok_or_else(|| {
                Error::Repository("Document vanished before its event was captured".into())
            })?;
        insert_event(tx, &event, origin, Some(&context))
            .await
            .map(|_| ())
    }

    async fn emit_metadata_in(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        before: &EventSnapshot,
    ) -> Result<()> {
        let after = capture_document(tx, before.document_id, None)
            .await?
            .ok_or_else(|| not_found("Document", before.document_id))?;
        for envelope in metadata_envelopes(before, &after)? {
            if envelope.event_type() == "document.updated" {
                insert_event_with_envelope(
                    tx,
                    &DocumentEvent::Updated {
                        document_id: after.document_id,
                    },
                    self.origin,
                    Some(&after),
                    &envelope,
                )
                .await?;
            } else {
                insert_domain_event_with_origin(tx, &envelope, self.origin).await?;
            }
        }
        Ok(())
    }

    async fn commit(&self, tx: Transaction<'_, Sqlite>) -> Result<()> {
        tx.commit().await.map_err(map_sqlx_error)?;
        self.wake.notify_one();
        Ok(())
    }
}

// Helper to convert sqlx error to domain error
fn map_sqlx_error(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

// Escape LIKE wildcards so user input matches literally
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

// Helper to map a write error, turning unique violations into `Duplicate`
fn map_write_error(e: sqlx::Error, entity_type: &'static str, value: &str) -> Error {
    if e.to_string().contains("UNIQUE constraint failed") {
        Error::Duplicate {
            entity_type,
            field: "name",
            value: value.to_string(),
        }
    } else {
        map_sqlx_error(e)
    }
}

// Helper to parse UUID from string
fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| Error::Repository(format!("Invalid UUID: {}", e)))
}

// Helper to parse optional UUID
fn parse_optional_uuid(s: Option<String>) -> Result<Option<Uuid>> {
    match s {
        Some(s) => Ok(Some(parse_uuid(&s)?)),
        None => Ok(None),
    }
}

/// Columns of `documents` read into a [`DocumentRow`]
const DOC_COLS: &str = "id, vault_id, uploaded_by, created_by, updated_by, title, content, original_filename, file_path, file_size,
    mime_type, correspondent_id, document_type_id, page_count, source_type, source_meta,
    ocr_status, embed_status, deleted_at, purge_requested_at, created_at, updated_at, document_date, reviewed_at, reviewed_by, review_origin";

fn not_found(entity_type: &'static str, id: Id) -> Error {
    Error::NotFound {
        entity_type,
        id: id.to_string(),
    }
}

/// The document must exist in `vault`
async fn require_document(conn: &mut SqliteConnection, vault: VaultId, id: Id) -> Result<()> {
    let found: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = documents.vault_id AND v.deleted_at IS NULL)",
    )
    .bind(id.to_string())
    .bind(vault.to_string())
    .fetch_optional(conn)
    .await
    .map_err(map_sqlx_error)?;
    found.map(|_| ()).ok_or_else(|| not_found("Document", id))
}

async fn stamp_document(conn: &mut SqliteConnection, vault: VaultId, id: Id) -> Result<()> {
    sqlx::query(
        "UPDATE documents SET updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
    .bind(id.to_string())
    .bind(vault.to_string())
    .execute(conn)
    .await
    .map_err(map_sqlx_error)?;
    Ok(())
}

/// A vocabulary entry must exist in `vault`; `table` is one of ours, never user input
async fn require_entry(
    conn: &mut SqliteConnection,
    table: &'static str,
    entity_type: &'static str,
    vault: VaultId,
    id: Id,
) -> Result<()> {
    let found: Option<(i64,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT 1 FROM {table} WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
    )))
    .bind(id.to_string())
    .bind(vault.to_string())
    .fetch_optional(conn)
    .await
    .map_err(map_sqlx_error)?;
    found.map(|_| ()).ok_or_else(|| not_found(entity_type, id))
}

// Row types for SQLx queries
#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    vault_id: String,
    uploaded_by: Option<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
    title: String,
    document_date: Option<String>,
    reviewed_at: Option<DateTime<Utc>>,
    reviewed_by: Option<String>,
    review_origin: Option<String>,
    content: Option<String>,
    original_filename: Option<String>,
    file_path: Option<String>,
    file_size: Option<i64>,
    mime_type: Option<String>,
    correspondent_id: Option<String>,
    document_type_id: Option<String>,
    page_count: Option<i64>,
    source_type: String,
    source_meta: Option<String>,
    ocr_status: String,
    embed_status: String,
    deleted_at: Option<DateTime<Utc>>,
    purge_requested_at: Option<DateTime<Utc>>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<DocumentRow> for Document {
    type Error = Error;

    fn try_from(row: DocumentRow) -> Result<Self> {
        Ok(Document {
            id: parse_uuid(&row.id)?,
            vault_id: VaultId(parse_uuid(&row.vault_id)?),
            uploaded_by: row.uploaded_by.map(UserId),
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
            title: row.title,
            reviewed_at: row.reviewed_at,
            reviewed_by: row.reviewed_by.map(UserId),
            review_origin: row
                .review_origin
                .map(|v| v.parse().map_err(Error::Repository))
                .transpose()?,
            document_date: row
                .document_date
                .map(|value| value.parse().map_err(|e: &str| Error::Repository(e.into())))
                .transpose()?,
            content: row.content,
            original_filename: row.original_filename,
            file_path: row.file_path,
            file_size: row.file_size.map(|size| size as u64),
            mime_type: row.mime_type,
            correspondent_id: parse_optional_uuid(row.correspondent_id)?,
            document_type_id: parse_optional_uuid(row.document_type_id)?,
            page_count: row.page_count.map(|c| c as u32),
            source: DocumentSource::from_stored(&row.source_type, row.source_meta.as_deref()),
            ocr_status: row.ocr_status.parse().map_err(Error::Repository)?,
            embed_status: row.embed_status.parse().map_err(Error::Repository)?,
            deleted_at: row.deleted_at,
            purge_requested_at: row.purge_requested_at,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
            updated_at: row
                .updated_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct DocumentTagRow {
    document_id: String,
    #[sqlx(flatten)]
    tag: TagRow,
}

#[derive(sqlx::FromRow)]
struct TagRow {
    id: String,
    vault_id: String,
    name: String,
    color: Option<String>,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl TryFrom<TagRow> for Tag {
    type Error = Error;

    fn try_from(row: TagRow) -> Result<Self> {
        Ok(Tag {
            id: parse_uuid(&row.id)?,
            vault_id: VaultId(parse_uuid(&row.vault_id)?),
            name: row.name,
            color: row.color,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
            updated_at: row
                .updated_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(e.to_string()))?,
            deleted_at: row
                .deleted_at
                .map(|value| value.parse::<DateTime<Utc>>())
                .transpose()
                .map_err(|e| Error::Repository(e.to_string()))?,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}

#[derive(sqlx::FromRow)]
struct CorrespondentRow {
    id: String,
    vault_id: String,
    name: String,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl TryFrom<CorrespondentRow> for Correspondent {
    type Error = Error;

    fn try_from(row: CorrespondentRow) -> Result<Self> {
        Ok(Correspondent {
            id: parse_uuid(&row.id)?,
            vault_id: VaultId(parse_uuid(&row.vault_id)?),
            name: row.name,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
            updated_at: row
                .updated_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(e.to_string()))?,
            deleted_at: row
                .deleted_at
                .map(|value| value.parse::<DateTime<Utc>>())
                .transpose()
                .map_err(|e| Error::Repository(e.to_string()))?,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}

#[derive(sqlx::FromRow)]
struct DocumentTypeRow {
    id: String,
    vault_id: String,
    name: String,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl TryFrom<DocumentTypeRow> for DocumentType {
    type Error = Error;

    fn try_from(row: DocumentTypeRow) -> Result<Self> {
        Ok(DocumentType {
            id: parse_uuid(&row.id)?,
            vault_id: VaultId(parse_uuid(&row.vault_id)?),
            name: row.name,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
            updated_at: row
                .updated_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(e.to_string()))?,
            deleted_at: row
                .deleted_at
                .map(|value| value.parse::<DateTime<Utc>>())
                .transpose()
                .map_err(|e| Error::Repository(e.to_string()))?,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}

/// Append the `WHERE` conditions of a document filter to a query ending in `WHERE ...`
fn push_filter(qb: &mut QueryBuilder<Sqlite>, filter: DocumentFilter) -> Result<()> {
    filter.validate()?;
    if let Some(reviewed) = filter.reviewed {
        qb.push(if reviewed {
            " AND reviewed_at IS NOT NULL"
        } else {
            " AND reviewed_at IS NULL"
        });
    }
    if let Some(id) = filter.correspondent_id {
        qb.push(" AND correspondent_id = ")
            .push_bind(id.to_string());
    }
    if let Some(id) = filter.document_type_id {
        qb.push(" AND document_type_id = ")
            .push_bind(id.to_string());
    }
    if let Some(mut tag_ids) = filter.tag_ids.filter(|t| !t.is_empty()) {
        // Duplicates would make the distinct count below unreachable
        tag_ids.sort();
        tag_ids.dedup();
        qb.push(" AND id IN (SELECT document_id FROM document_tags WHERE tag_id IN (");
        let mut sep = qb.separated(", ");
        for tag_id in &tag_ids {
            sep.push_bind(tag_id.to_string());
        }
        qb.push(") GROUP BY document_id HAVING COUNT(DISTINCT tag_id) = ")
            .push_bind(tag_ids.len() as i64)
            .push(")");
    }
    if let Some(query) = filter.search_query {
        for term in query.split_whitespace() {
            let pattern = format!("%{}%", escape_like(term));
            qb.push(" AND (title LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' OR content LIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\')");
        }
    }
    if let Some(after) = filter.created_after {
        qb.push(" AND created_at >= ").push_bind(after.to_rfc3339());
    }
    if let Some(before) = filter.created_before {
        qb.push(" AND created_at < ").push_bind(before.to_rfc3339());
    }
    if let Some(date) = filter.document_date_from {
        qb.push(" AND document_date >= ")
            .push_bind(date.to_string());
    }
    if let Some(date) = filter.document_date_to {
        qb.push(" AND document_date <= ")
            .push_bind(date.to_string());
    }
    Ok(())
}

#[async_trait]
impl DocumentRepository for SqliteDocumentRepository {
    async fn locate(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT vault_id FROM documents WHERE id = ? AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = documents.vault_id AND v.deleted_at IS NULL)")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;
        row.map(|(v,)| parse_uuid(&v).map(VaultId)).transpose()
    }

    async fn move_to(
        &self,
        id: Id,
        from: VaultId,
        to: VaultId,
        actor: Option<&UserId>,
    ) -> Result<Document> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let doc: DocumentRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
        )))
        .bind(id.to_string())
        .bind(from.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| not_found("Document", id))?;
        if from == to {
            return doc.try_into();
        }
        let target: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM vaults WHERE id = ? AND deleted_at IS NULL")
                .bind(to.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx_error)?;
        if target.is_none() {
            return Err(not_found("Vault", to.0));
        }
        super::prepare_move(&mut tx, from, id).await?;

        let before = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        let mutation_id = Id::new_v4();

        let tags: Vec<(String,)> = sqlx::query_as(
            "SELECT t.name FROM tags t JOIN document_tags dt ON dt.tag_id = t.id
             WHERE dt.document_id = ? AND t.deleted_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let correspondent = remap_by_name(
            &mut tx,
            "correspondents",
            from,
            to,
            doc.correspondent_id.as_deref(),
            mutation_id,
        )
        .await?;
        let document_type = remap_by_name(
            &mut tx,
            "document_types",
            from,
            to,
            doc.document_type_id.as_deref(),
            mutation_id,
        )
        .await?;

        sqlx::query("DELETE FROM document_tags WHERE document_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        for (name,) in tags {
            let tag_id = match sqlx::query_as::<_, (String,)>(
                "SELECT id FROM tags WHERE vault_id = ? AND name = ? AND deleted_at IS NULL",
            )
            .bind(to.to_string())
            .bind(&name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?
            {
                Some((existing,)) => existing,
                None => {
                    let new_id = Id::new_v4().to_string();
                    let actor = folio_core::mutation::current_actor();
                    sqlx::query(
                        "INSERT INTO tags (id, vault_id, name, color, created_at, updated_at, created_by, updated_by)
                         SELECT ?, ?, name, color, strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'), ?, ? FROM tags
                         WHERE vault_id = ? AND name = ? AND deleted_at IS NULL",
                    )
                    .bind(&new_id)
                    .bind(to.to_string())
                    .bind(actor.as_ref().map(UserId::as_str))
                    .bind(actor.as_ref().map(UserId::as_str))
                    .bind(from.to_string())
                    .bind(&name)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_error)?;
                    emit_moved_taxonomy(&mut tx, "tags", to, parse_uuid(&new_id)?, mutation_id)
                        .await?;
                    new_id
                }
            };
            sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES (?, ?)")
                .bind(id.to_string())
                .bind(tag_id)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_error)?;
        }

        sqlx::query(
            "UPDATE documents SET vault_id = ?, correspondent_id = ?, document_type_id = ?,
                                  updated_at = ?, updated_by = ?
             WHERE id = ?",
        )
        .bind(to.to_string())
        .bind(correspondent)
        .bind(document_type)
        .bind(Utc::now().to_rfc3339())
        .bind(actor.map(UserId::as_str))
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        sqlx::query("UPDATE document_entries SET updated_at = ?, updated_by = ? WHERE document_id = ? AND vault_id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(actor.map(UserId::as_str))
            .bind(id.to_string())
            .bind(to.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        for vault in [from, to] {
            sqlx::query(
                "INSERT INTO audit_events
                     (id, at, actor_user_id, action, vault_id, target_type, target_id, details)
                 VALUES (?, ?, ?, 'document.moved', ?, 'document', ?, ?)",
            )
            .bind(Id::new_v4().to_string())
            .bind(Utc::now().to_rfc3339())
            .bind(actor.map(UserId::as_str))
            .bind(vault.to_string())
            .bind(id.to_string())
            .bind(serde_json::json!({ "from": from, "to": to }).to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }
        let after = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        for envelope in move_envelopes(&before, &after, mutation_id)? {
            insert_domain_event_with_origin(&mut tx, &envelope, self.origin).await?;
        }
        self.commit(tx).await?;

        self.find_by_id(to, id)
            .await?
            .ok_or_else(|| not_found("Document", id))
    }

    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {DOC_COLS} FROM documents WHERE deleted_at IS NULL AND vault_id = "
        ));
        qb.push_bind(vault.to_string());

        if let Some(filter) = filter {
            push_filter(&mut qb, filter)?;
        }

        qb.push(" ORDER BY created_at DESC");

        let rows: Vec<DocumentRow> = qb
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn page(
        &self,
        vault: VaultId,
        query: folio_core::models::DocumentPageQuery,
    ) -> Result<folio_core::models::DocumentPage> {
        use folio_core::models::DocumentSort;
        query.validate(vault)?;
        let mut qb = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {DOC_COLS} FROM documents WHERE deleted_at IS NULL AND vault_id = "
        ));
        qb.push_bind(vault.to_string());
        push_filter(&mut qb, query.filter.clone())?;
        let (column, direction, comparison) = match query.sort {
            DocumentSort::Newest => ("created_at", "DESC", " < "),
            DocumentSort::Oldest => ("created_at", "ASC", " > "),
            DocumentSort::Title => ("title COLLATE BINARY", "ASC", " > "),
        };
        if let Some(cursor) = &query.cursor {
            qb.push(" AND (")
                .push(column)
                .push(", id)")
                .push(comparison)
                .push("(");
            match query.sort {
                DocumentSort::Title => {
                    qb.push_bind(cursor.title.clone());
                }
                _ => {
                    qb.push_bind(cursor.created_at.to_rfc3339());
                }
            }
            qb.push(", ").push_bind(cursor.id.to_string()).push(")");
        }
        qb.push(" ORDER BY ")
            .push(column)
            .push(" ")
            .push(direction)
            .push(", id ")
            .push(direction)
            .push(" LIMIT ")
            .push_bind(i64::from(query.limit) + 1);
        let rows: Vec<DocumentRow> = qb
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        let documents = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>>>()?;
        Ok(query.finish(vault, documents))
    }

    async fn list_ids(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Id>> {
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id FROM documents WHERE deleted_at IS NULL AND vault_id = ",
        );
        qb.push_bind(vault.to_string());
        if let Some(filter) = filter {
            push_filter(&mut qb, filter)?;
        }
        qb.push(" ORDER BY created_at DESC");

        let ids: Vec<String> = qb
            .build_query_scalar()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        ids.iter().map(|id| parse_uuid(id)).collect()
    }

    async fn create(&self, vault: VaultId, params: CreateDocument) -> Result<Document> {
        self.create_reserved(vault, Id::new_v4(), None, false, params)
            .await
    }

    async fn create_for_upload(&self, vault: VaultId, params: CreateDocument) -> Result<Document> {
        self.create_reserved(vault, Id::new_v4(), None, true, params)
            .await
    }

    async fn create_imported(
        &self,
        vault: VaultId,
        document: ImportedDocument,
    ) -> Result<Document> {
        document.validate()?;
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now().trunc_subsecs(6);
        let id = document.id;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        if let Some(c) = document.correspondent_id {
            require_entry(&mut tx, "correspondents", "Correspondent", vault, c).await?;
        }
        if let Some(t) = document.document_type_id {
            require_entry(&mut tx, "document_types", "DocumentType", vault, t).await?;
        }
        for tag_id in &document.tag_ids {
            require_entry(&mut tx, "tags", "Tag", vault, *tag_id).await?;
        }
        let review = (!document.in_inbox).then_some(ReviewOrigin::System);
        let inserted = sqlx::query(
            "INSERT INTO documents (id, vault_id, uploaded_by, created_by, updated_by, title, content,
                 original_filename, file_path, mime_type, correspondent_id, document_type_id,
                 ocr_status, embed_status, source_type, created_at, updated_at,
                 document_date, reviewed_at, review_origin, external_source, external_id, content_sha256)
             VALUES (?1, ?2, NULL, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 ?16, ?17, ?18, ?19, ?20, ?21)",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(&document.title)
        .bind(&document.content)
        .bind(&document.original_filename)
        .bind(&document.file_path)
        .bind(&document.mime_type)
        .bind(document.correspondent_id.map(|id| id.to_string()))
        .bind(document.document_type_id.map(|id| id.to_string()))
        .bind(ProcessingStatus::Done.as_str())
        .bind(document.embed_status().as_str())
        .bind(SourceKind::Import.as_str())
        .bind(document.created_at.trunc_subsecs(6).to_rfc3339())
        .bind(document.updated_at.trunc_subsecs(6).to_rfc3339())
        .bind(document.document_date.map(|date| date.to_string()))
        .bind(review.map(|_| now.to_rfc3339()))
        .bind(review.map(ReviewOrigin::as_str))
        .bind(document.external_ref.source.as_str())
        .bind(&document.external_ref.id)
        .bind(&document.content_sha256)
        .execute(&mut *tx)
        .await;
        if let Err(error) = inserted {
            if error
                .as_database_error()
                .is_some_and(|e| e.is_unique_violation())
            {
                tx.rollback().await.map_err(map_sqlx_error)?;
                return Err(crate::database::import_collision(self, vault, &document, error).await);
            }
            return Err(map_sqlx_error(error));
        }
        for tag_id in &document.tag_ids {
            sqlx::query(
                "INSERT INTO document_tags (document_id, tag_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
            )
            .bind(id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }
        super::metadata_value_repo::insert_new_values(
            &mut tx,
            vault,
            id,
            &document.metadata,
            actor.as_ref(),
            now,
        )
        .await?;
        for note in &document.notes {
            sqlx::query(
                "INSERT INTO document_entries (id, document_id, vault_id, kind, body, created_at,
                     updated_at, created_by, updated_by, external_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?7, ?8)",
            )
            .bind(Id::new_v4().to_string())
            .bind(id.to_string())
            .bind(vault.to_string())
            .bind(EntryKind::Note.as_str())
            .bind(note.body.as_str())
            .bind(note.created_at.trunc_subsecs(6).to_rfc3339())
            .bind(actor.as_ref().map(UserId::as_str))
            .bind(&note.external_id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }
        if document.embed_status() == ProcessingStatus::Pending {
            crate::queue::insert_job_sqlite(
                &mut tx,
                crate::queue::JobType::GenerateEmbedding {
                    document_id: id,
                    origin: EventOrigin::Import,
                },
                now,
                crate::queue::DEFAULT_MAX_ATTEMPTS,
            )
            .await?;
        }
        self.emit_as(
            &mut tx,
            DocumentEvent::Created { document_id: id },
            EventOrigin::Import,
        )
        .await?;
        let row: DocumentRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE id = ? AND vault_id = ?"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let created = row.try_into()?;
        self.commit(tx).await?;
        Ok(created)
    }

    async fn find_by_external_ref(
        &self,
        vault: VaultId,
        external: &ExternalRef,
    ) -> Result<Option<Id>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM documents WHERE vault_id = ? AND external_source = ? AND external_id = ?",
        )
        .bind(vault.to_string())
        .bind(external.source.as_str())
        .bind(&external.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        id.as_deref().map(parse_uuid).transpose()
    }

    async fn create_for_ingest(
        &self,
        vault: VaultId,
        claim: &folio_core::models::LedgerEntry,
        params: CreateDocument,
    ) -> Result<Document> {
        if claim.vault_id != vault {
            return Err(Error::Validation(
                "Ingest claim belongs to a different vault".into(),
            ));
        }
        if claim.outcome != "claimed" {
            return Err(Error::Conflict("Ingest reservation is not claimed".into()));
        }
        let id = claim
            .document_id
            .ok_or_else(|| Error::Validation("Ingest claim has no document identity".into()))?;
        self.create_reserved(vault, id, Some(claim), true, params)
            .await
    }

    async fn set_reviewed(&self, vault: VaultId, id: Id, reviewed: bool) -> Result<Document> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let existing: Document = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Document", id))?;

        if existing.reviewed_at.is_some() == reviewed {
            return Ok(existing);
        }
        let before = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        let mut review = folio_core::models::DocumentReview::default();
        review.set_reviewed_from_context(reviewed);
        let decision = review.decision();
        sqlx::query("UPDATE documents SET reviewed_at = ?, reviewed_by = ?, review_origin = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ?")
            .bind(decision.map(|d| d.reviewed_at.to_rfc3339()))
            .bind(decision.and_then(|d| d.reviewed_by.as_ref()).map(|id| id.0.as_str()))
            .bind(decision.map(|d| d.origin.as_str()))
            .bind(Utc::now().to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string())
            .execute(&mut *tx).await.map_err(map_sqlx_error)?;
        self.emit_metadata_in(&mut tx, &before).await?;
        let row: DocumentRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE id = ? AND vault_id = ?"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let result = row.try_into()?;
        self.commit(tx).await?;
        Ok(result)
    }

    async fn update(&self, vault: VaultId, id: Id, params: UpdateDocument) -> Result<Document> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let existing: Document = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Document", id))?;

        let before = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        let title = params.title.unwrap_or_else(|| existing.title.clone());
        let content = params.content.or_else(|| existing.content.clone());
        let correspondent_id = params.correspondent_id.unwrap_or(existing.correspondent_id);
        let document_type_id = params.document_type_id.unwrap_or(existing.document_type_id);
        let document_date = params.document_date.unwrap_or(existing.document_date);
        let now = Utc::now();

        let tags_changed = if let Some(tags) = params.tag_ids {
            self.replace_tags(&mut tx, vault, id, &tags).await?
        } else {
            false
        };
        if title == existing.title
            && content == existing.content
            && correspondent_id == existing.correspondent_id
            && document_type_id == existing.document_type_id
            && document_date == existing.document_date
            && !tags_changed
        {
            return Ok(existing);
        }

        if let Some(c) = correspondent_id.filter(|c| Some(*c) != existing.correspondent_id) {
            require_entry(&mut tx, "correspondents", "Correspondent", vault, c).await?;
        }
        if let Some(t) = document_type_id.filter(|t| Some(*t) != existing.document_type_id) {
            require_entry(&mut tx, "document_types", "DocumentType", vault, t).await?;
        }

        sqlx::query(
            "UPDATE documents SET title = ?, content = ?, correspondent_id = ?,
                                  document_type_id = ?, updated_at = ?, document_date = ?, updated_by = ?
             WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(&title)
        .bind(&content)
        .bind(correspondent_id.map(|id| id.to_string()))
        .bind(document_type_id.map(|id| id.to_string()))
        .bind(now.to_rfc3339())
        .bind(document_date.map(|value| value.to_string()))
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        self.emit_metadata_in(&mut tx, &before).await?;
        self.commit(tx).await?;

        Ok(Document {
            id,
            vault_id: vault,
            uploaded_by: existing.uploaded_by,
            created_by: existing.created_by,
            updated_by: folio_core::mutation::current_actor(),
            document_date,
            reviewed_at: existing.reviewed_at,
            reviewed_by: existing.reviewed_by,
            review_origin: existing.review_origin,
            title,
            content,
            original_filename: existing.original_filename,
            file_path: existing.file_path,
            file_size: existing.file_size,
            mime_type: existing.mime_type,
            correspondent_id,
            document_type_id,
            page_count: existing.page_count,
            source: existing.source,
            ocr_status: existing.ocr_status,
            embed_status: existing.embed_status,
            deleted_at: None,
            purge_requested_at: None,
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents
             WHERE id = ? AND vault_id = ?"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let document: Document = row.ok_or_else(|| not_found("Document", id))?.try_into()?;
        if document.deleted_at.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE documents SET deleted_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL")
            .bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
        self.emit_in(&mut tx, DocumentEvent::Trashed { document_id: id })
            .await?;
        self.commit(tx).await
    }

    async fn locate_retained(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT vault_id FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        row.map(|(v,)| parse_uuid(&v).map(VaultId)).transpose()
    }

    async fn is_purging_or_missing(&self, id: Id) -> Result<bool> {
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM documents WHERE id = ? AND purge_requested_at IS NULL)")
            .bind(id.to_string()).fetch_one(&self.pool).await.map_err(map_sqlx_error)
    }

    async fn locate_trashed(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT vault_id FROM documents
             WHERE id = ? AND deleted_at IS NOT NULL",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        row.map(|(v,)| parse_uuid(&v).map(VaultId)).transpose()
    }

    async fn find_trashed_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents
             WHERE id = ? AND vault_id = ? AND deleted_at IS NOT NULL"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Document>> {
        let rows: Vec<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents
             WHERE vault_id = ? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, id DESC"
        )))
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn restore(&self, vault: VaultId, id: Id) -> Result<Document> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents
             WHERE id = ? AND vault_id = ?"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let document: Document = row.ok_or_else(|| not_found("Document", id))?.try_into()?;
        if document.purge_requested_at.is_some() {
            return Err(Error::Conflict(
                "permanent deletion has already been requested".into(),
            ));
        }
        if document.deleted_at.is_none() {
            return Ok(document);
        }
        sqlx::query("UPDATE documents SET deleted_at = NULL,
                 ocr_status = CASE WHEN ocr_status = 'running' THEN 'pending' ELSE ocr_status END,
                 embed_status = CASE WHEN embed_status = 'running' THEN 'pending' ELSE embed_status END,
                 updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? AND purge_requested_at IS NULL")
            .bind(Utc::now().to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
        if document.file_path.is_some() {
            crate::queue::insert_job_sqlite(
                &mut tx,
                crate::queue::JobType::process_document(id),
                Utc::now(),
                crate::queue::DEFAULT_MAX_ATTEMPTS,
            )
            .await?;
        }
        self.emit_in(&mut tx, DocumentEvent::Restored { document_id: id })
            .await?;
        self.commit(tx).await?;
        self.find_by_id(vault, id)
            .await?
            .ok_or_else(|| not_found("Document", id))
    }

    async fn request_purge(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents
             WHERE id = ? AND vault_id = ?"
        )))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let document: Document = row.ok_or_else(|| not_found("Document", id))?.try_into()?;
        if document.deleted_at.is_none() {
            return Err(Error::Conflict(
                "move the document to trash before deleting it forever".into(),
            ));
        }
        if document.purge_requested_at.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE documents SET purge_requested_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ?")
            .bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
        let context = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        let event = DocumentEvent::PurgeRequested { document_id: id };
        let envelope = folio_core::events::document_event_envelope(&event, Some(&context))?
            .ok_or_else(|| not_found("Document", id))?;
        crate::queue::insert_job_sqlite(
            &mut tx,
            crate::queue::JobType::PurgeDocument {
                document_id: id,
                correlation_id: Some(envelope.correlation_id()),
                causation_id: Some(envelope.event_id()),
            },
            now,
            crate::queue::DEFAULT_MAX_ATTEMPTS,
        )
        .await?;
        insert_event_with_envelope(&mut tx, &event, self.origin, Some(&context), &envelope).await?;
        self.commit(tx).await
    }

    async fn list_purge_requested(&self) -> Result<Vec<Document>> {
        let rows: Vec<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {DOC_COLS} FROM documents
             WHERE deleted_at IS NOT NULL AND purge_requested_at IS NOT NULL ORDER BY purge_requested_at, id")))
            .fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn finalize_purge(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {} FROM documents
             WHERE id = ? AND vault_id = ? AND deleted_at IS NOT NULL AND purge_requested_at IS NOT NULL", DOC_COLS.replace(" content,", " NULL AS content,"))))
            .bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let Some(row) = row else {
            return Ok(());
        };
        let context = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        let document: Document = row.try_into()?;
        let tags: Vec<TagRow> = sqlx::query_as("SELECT t.id, t.vault_id, t.name, t.color, t.created_at, t.updated_at, t.deleted_at, t.created_by, t.updated_by FROM tags t JOIN document_tags dt ON t.id = dt.tag_id
             WHERE dt.document_id = ? AND t.vault_id = ?")
            .bind(id.to_string()).bind(vault.to_string()).fetch_all(&mut *tx).await.map_err(map_sqlx_error)?;
        let tags = tags
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Tag>>>()?;
        sqlx::query("UPDATE ingest_ledger SET outcome = 'deleted' WHERE document_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        super::clear_incoming_links(&mut tx, vault, id).await?;
        sqlx::query(
            "DELETE FROM documents
             WHERE id = ? AND vault_id = ? AND purge_requested_at IS NOT NULL",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let envelope = folio_core::domain_event::DomainEventEnvelope::new(
            "document.purged",
            vault,
            folio_core::domain_event::DomainEventSubject {
                subject_type: "document".into(),
                id,
            },
            Some(id),
            folio_core::domain_event::EventAudience::Vault,
            folio_core::events::document_event_envelope(
                &DocumentEvent::Updated { document_id: id },
                Some(&context),
            )?
            .ok_or_else(|| not_found("Document", id))?
            .data()
            .clone(),
        )?;
        insert_event_with_envelope(
            &mut tx,
            &DocumentEvent::Deleted {
                document_id: id,
                snapshot: Some(Arc::new(DeletedDocument { document, tags })),
            },
            self.origin,
            Some(&context),
            &envelope,
        )
        .await?;
        self.commit(tx).await
    }

    async fn finalize_upload(
        &self,
        vault: VaultId,
        id: Id,
        file_path: &str,
        file_size: u64,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let (pending, existing_path): (bool, Option<String>) = sqlx::query_as("SELECT upload_pending, file_path FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL")
            .bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?
            .ok_or_else(|| not_found("Document", id))?;
        if !pending && existing_path.as_deref() == Some(file_path) {
            sqlx::query("UPDATE documents SET file_size = ? WHERE id = ? AND vault_id = ? AND file_size IS NULL")
                .bind(file_size as i64).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
            return self.commit(tx).await;
        }
        sqlx::query("UPDATE documents SET file_path = ?, file_size = ?, upload_pending = FALSE, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ?")
            .bind(file_path).bind(file_size as i64).bind(Utc::now().to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
        if pending {
            self.emit_in(&mut tx, DocumentEvent::Created { document_id: id })
                .await?;
        }
        self.commit(tx).await
    }

    async fn set_file_path(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()> {
        let result = sqlx::query("UPDATE documents SET file_path = ?, file_size = NULL, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL AND file_path IS NOT ?").bind(file_path).bind(Utc::now().to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string()).bind(file_path).execute(&self.pool).await.map_err(map_sqlx_error)?;
        if result.rows_affected() == 0 && self.find_by_id(vault, id).await?.is_none() {
            return Err(not_found("Document", id));
        }
        Ok(())
    }

    async fn list_missing_file_size(&self, after: Option<Id>, limit: u32) -> Result<Vec<Document>> {
        // The backfill never reads the OCR text, so it is not loaded
        let rows: Vec<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM documents WHERE file_path IS NOT NULL AND file_size IS NULL AND purge_requested_at IS NULL AND id > ? ORDER BY id LIMIT ?",
            DOC_COLS.replace(" content,", " NULL AS content,")
        )))
        .bind(after.map(|id| id.to_string()).unwrap_or_default())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        rows.into_iter().map(Document::try_from).collect()
    }

    async fn set_file_size(
        &self,
        vault: VaultId,
        id: Id,
        file_path: &str,
        size: u64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE documents SET file_size = ? WHERE id = ? AND vault_id = ? AND file_path = ?",
        )
        .bind(size as i64)
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(file_path)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn set_page_count(&self, vault: VaultId, id: Id, page_count: u32) -> Result<()> {
        let result = sqlx::query("UPDATE documents SET page_count = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL AND page_count IS NOT ?").bind(page_count as i64).bind(Utc::now().to_rfc3339()).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).bind(id.to_string()).bind(vault.to_string()).bind(page_count as i64).execute(&self.pool).await.map_err(map_sqlx_error)?;
        if result.rows_affected() == 0 && self.find_by_id(vault, id).await?.is_none() {
            return Err(not_found("Document", id));
        }
        Ok(())
    }

    async fn set_stage_status(
        &self,
        vault: VaultId,
        id: Id,
        stage: ProcessingStage,
        status: ProcessingStatus,
    ) -> Result<()> {
        let sql = match stage {
            ProcessingStage::Ocr => {
                "UPDATE documents SET ocr_status = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL AND ocr_status != ?"
            }
            ProcessingStage::Embed => {
                "UPDATE documents SET embed_status = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL AND embed_status != ?"
            }
        };
        sqlx::query(sql)
            .bind(status.as_str())
            .bind(id.to_string())
            .bind(vault.to_string())
            .bind(status.as_str())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn complete_ocr(&self, vault: VaultId, id: Id, content: &str) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        require_document(&mut tx, vault, id).await?;
        let result = sqlx::query(
            "UPDATE documents SET content = ?, ocr_status = 'done', updated_at = ?, updated_by = ?
             WHERE id = ? AND vault_id = ? AND deleted_at IS NULL
             AND (ocr_status != 'done' OR content IS NOT ?)",
        )
        .bind(content)
        .bind(Utc::now().to_rfc3339())
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(content)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        if result.rows_affected() > 0 {
            self.emit_in(&mut tx, DocumentEvent::OcrComplete { document_id: id })
                .await?;
        }
        self.commit(tx).await
    }

    async fn list_all(&self) -> Result<Vec<Document>> {
        let rows: Vec<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE deleted_at IS NULL ORDER BY created_at DESC"
        )))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>> {
        let column = match stage {
            ProcessingStage::Ocr => "ocr_status",
            ProcessingStage::Embed => "embed_status",
        };
        let rows: Vec<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DOC_COLS} FROM documents WHERE deleted_at IS NULL AND {column} = 'pending'
             AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = documents.vault_id AND v.deleted_at IS NULL) ORDER BY created_at"
        )))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn reset_embedding_status(&self) -> Result<Vec<Id>> {
        let ids: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM documents WHERE deleted_at IS NULL AND content IS NOT NULL AND content != ''")
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        sqlx::query(
            "UPDATE documents SET embed_status = 'pending'
             WHERE deleted_at IS NULL AND content IS NOT NULL AND content != ''",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        ids.into_iter().map(|(id,)| parse_uuid(&id)).collect()
    }

    async fn get_tags(&self, vault: VaultId, document_id: Id) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT t.id, t.vault_id, t.name, t.color, t.created_at, t.updated_at, t.deleted_at, t.created_by, t.updated_by
             FROM tags t
             INNER JOIN document_tags dt ON t.id = dt.tag_id
             INNER JOIN documents d ON d.id = dt.document_id
             WHERE dt.document_id = ? AND d.deleted_at IS NULL AND d.vault_id = ? AND t.vault_id = d.vault_id",
        )
        .bind(document_id.to_string())
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn tags_for_documents(
        &self,
        vault: VaultId,
        document_ids: &[Id],
    ) -> Result<std::collections::HashMap<Id, Vec<Tag>>> {
        let mut ids = document_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let mut result: std::collections::HashMap<Id, Vec<Tag>> = Default::default();
        // Legacy unbounded list callers can exceed the database's bind limit.
        for chunk in ids.chunks(500) {
            let mut qb = QueryBuilder::<Sqlite>::new(
                "SELECT dt.document_id, t.id, t.vault_id, t.name, t.color, t.created_at, t.updated_at, t.deleted_at, t.created_by, t.updated_by
                 FROM document_tags dt JOIN tags t ON t.id = dt.tag_id
                 JOIN documents d ON d.id = dt.document_id
                 WHERE t.vault_id = d.vault_id AND d.deleted_at IS NULL AND d.vault_id = ",
            );
            qb.push_bind(vault.to_string()).push(" AND d.id IN (");
            let mut separated = qb.separated(", ");
            for id in chunk {
                separated.push_bind(id.to_string());
            }
            qb.push(") ORDER BY t.name, t.id");
            let rows: Vec<DocumentTagRow> = qb
                .build_query_as()
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx_error)?;
            for row in rows {
                result
                    .entry(parse_uuid(&row.document_id)?)
                    .or_default()
                    .push(row.tag.try_into()?);
            }
        }
        Ok(result)
    }

    async fn set_tags(&self, vault: VaultId, document_id: Id, tag_ids: &[Id]) -> Result<()> {
        self.update(
            vault,
            document_id,
            UpdateDocument {
                tag_ids: Some(tag_ids.to_vec()),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
    }

    async fn add_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        require_document(&mut tx, vault, document_id).await?;
        let before = capture_document(&mut tx, document_id, None)
            .await?
            .ok_or_else(|| not_found("Document", document_id))?;
        require_entry(&mut tx, "tags", "Tag", vault, tag_id).await?;
        let result = sqlx::query(
            "INSERT INTO document_tags (document_id, tag_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(document_id.to_string())
        .bind(tag_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        if result.rows_affected() > 0 {
            stamp_document(&mut tx, vault, document_id).await?;
            self.emit_metadata_in(&mut tx, &before).await?;
        }
        self.commit(tx).await
    }

    async fn remove_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        require_document(&mut tx, vault, document_id).await?;
        let before = capture_document(&mut tx, document_id, None)
            .await?
            .ok_or_else(|| not_found("Document", document_id))?;
        let result = sqlx::query("DELETE FROM document_tags WHERE document_id = ? AND tag_id = ?")
            .bind(document_id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        if result.rows_affected() > 0 {
            stamp_document(&mut tx, vault, document_id).await?;
            self.emit_metadata_in(&mut tx, &before).await?;
        }
        self.commit(tx).await
    }
}

/// The entry of `table` in `to` that carries the same name as entry `id` of
/// `from`, created when the target has none
async fn remap_by_name(
    conn: &mut SqliteConnection,
    table: &'static str,
    from: VaultId,
    to: VaultId,
    id: Option<&str>,
    mutation_id: Id,
) -> Result<Option<String>> {
    let Some(id) = id else { return Ok(None) };
    let name: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT name FROM {table} WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
    )))
    .bind(id)
    .bind(from.to_string())
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_sqlx_error)?;
    let Some((name,)) = name else { return Ok(None) };
    let existing: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM {table} WHERE vault_id = ? AND name = ? AND deleted_at IS NULL"
    )))
    .bind(to.to_string())
    .bind(&name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_sqlx_error)?;
    if let Some((existing,)) = existing {
        return Ok(Some(existing));
    }
    let actor = folio_core::mutation::current_actor();
    let now = Utc::now();
    let new_id = Id::new_v4().to_string();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} (id, vault_id, name, created_at, updated_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )))
    .bind(&new_id)
    .bind(to.to_string())
    .bind(&name)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(actor.as_ref().map(UserId::as_str))
    .bind(actor.as_ref().map(UserId::as_str))
    .execute(&mut *conn)
    .await
    .map_err(map_sqlx_error)?;
    emit_moved_taxonomy(conn, table, to, parse_uuid(&new_id)?, mutation_id).await?;
    Ok(Some(new_id))
}

async fn emit_moved_taxonomy(
    conn: &mut SqliteConnection,
    table: &'static str,
    vault: VaultId,
    id: Id,
    mutation_id: Id,
) -> Result<()> {
    let (subject, data) = match table {
        "tags" => {
            let row: TagRow = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE id = ?").bind(id.to_string()).fetch_one(&mut *conn).await.map_err(map_sqlx_error)?;
            let value: Tag = row.try_into()?;
            (
                "tag",
                serde_json::to_value(value)
                    .map_err(|error| Error::Repository(error.to_string()))?,
            )
        }
        "correspondents" => {
            let row: CorrespondentRow = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE id = ?").bind(id.to_string()).fetch_one(&mut *conn).await.map_err(map_sqlx_error)?;
            let value: Correspondent = row.try_into()?;
            (
                "correspondent",
                serde_json::to_value(value)
                    .map_err(|error| Error::Repository(error.to_string()))?,
            )
        }
        "document_types" => {
            let row: DocumentTypeRow = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE id = ?").bind(id.to_string()).fetch_one(&mut *conn).await.map_err(map_sqlx_error)?;
            let value: DocumentType = row.try_into()?;
            (
                "document_type",
                serde_json::to_value(value)
                    .map_err(|error| Error::Repository(error.to_string()))?,
            )
        }
        _ => return Err(Error::Repository("Unsupported taxonomy table".into())),
    };
    let event = folio_core::domain_event::DomainEventEnvelope::new(
        format!("{subject}.created"),
        vault,
        folio_core::domain_event::DomainEventSubject {
            subject_type: subject.into(),
            id,
        },
        None,
        folio_core::domain_event::EventAudience::Vault,
        data,
    )?
    .with_mutation(mutation_id)?;
    super::insert_domain_event(conn, &event).await?;
    Ok(())
}

async fn emit_taxonomy(
    conn: &mut SqliteConnection,
    subject_type: &str,
    action: &str,
    vault: VaultId,
    id: Id,
    value: &impl serde::Serialize,
) -> Result<()> {
    let event = folio_core::domain_event::DomainEventEnvelope::new(
        format!("{subject_type}.{action}"),
        vault,
        folio_core::domain_event::DomainEventSubject {
            subject_type: subject_type.into(),
            id,
        },
        None,
        folio_core::domain_event::EventAudience::Vault,
        serde_json::to_value(value).map_err(|error| Error::Repository(error.to_string()))?,
    )?;
    super::insert_domain_event(conn, &event).await?;
    Ok(())
}
pub struct SqliteTagRepository {
    pool: SqlitePool,
}
impl SqliteTagRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
#[async_trait]
impl TagRepository for SqliteTagRepository {
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE id = ? AND vault_id = ? AND deleted_at IS NULL").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn find_retained(&self, vault: VaultId, id: Id) -> Result<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE name = ? AND vault_id = ? AND deleted_at IS NULL").bind(name).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn list(&self, vault: VaultId) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE vault_id = ? AND deleted_at IS NULL ORDER BY name").bind(vault.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE vault_id = ? AND deleted_at IS NOT NULL ORDER BY name").bind(vault.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn create(&self, vault: VaultId, params: CreateTag) -> Result<Tag> {
        let id = Id::new_v4();
        let now = Utc::now();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        sqlx::query("INSERT INTO tags (id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)")
 .bind(id.to_string()).bind(vault.to_string()).bind(&params.name).bind(&params.color).bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(actor.as_ref().map(UserId::as_str)).bind(actor.as_ref().map(UserId::as_str)).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Tag", &params.name))?;
        let value = Tag {
            id,
            vault_id: vault,
            name: params.name,
            color: params.color,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: actor.clone(),
            updated_by: actor,
        };
        emit_taxonomy(&mut tx, "tag", "created", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
    async fn update(&self, vault: VaultId, id: Id, params: UpdateTag) -> Result<Tag> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: Tag = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Tag", id))?;
        if value.deleted_at.is_some() {
            return Err(not_found("Tag", id));
        }
        let name = params.name.unwrap_or_else(|| value.name.clone());
        let color = params.color.unwrap_or_else(|| value.color.clone());
        if name == value.name && color == value.color {
            return Ok(value);
        }
        value.name = name;
        value.color = color;
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE tags SET name = ?, color = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(&value.color).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Tag", &value.name))?;
        emit_taxonomy(&mut tx, "tag", "updated", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: Tag = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Tag", id))?;
        if value.deleted_at.is_some() {
            return Ok(());
        }
        value.deleted_at = Some(Utc::now());
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE tags SET name = ?, color = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(&value.color).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Tag", &value.name))?;
        emit_taxonomy(&mut tx, "tag", "deleted", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(())
    }
    async fn restore(&self, vault: VaultId, id: Id) -> Result<Tag> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<TagRow> = sqlx::query_as("SELECT id, vault_id, name, color, created_at, updated_at, deleted_at, created_by, updated_by FROM tags WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: Tag = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Tag", id))?;
        if value.deleted_at.is_none() {
            return Ok(value);
        }
        value.deleted_at = None;
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE tags SET name = ?, color = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(&value.color).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Tag", &value.name))?;
        emit_taxonomy(&mut tx, "tag", "restored", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
}

pub struct SqliteCorrespondentRepository {
    pool: SqlitePool,
}
impl SqliteCorrespondentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
#[async_trait]
impl CorrespondentRepository for SqliteCorrespondentRepository {
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Correspondent>> {
        let row: Option<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn find_retained(&self, vault: VaultId, id: Id) -> Result<Option<Correspondent>> {
        let row: Option<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Correspondent>> {
        let row: Option<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE name = ? AND vault_id = ? AND deleted_at IS NULL").bind(name).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn list(&self, vault: VaultId) -> Result<Vec<Correspondent>> {
        let rows: Vec<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE vault_id = ? AND deleted_at IS NULL ORDER BY name").bind(vault.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Correspondent>> {
        let rows: Vec<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE vault_id = ? AND deleted_at IS NOT NULL ORDER BY name").bind(vault.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn create(&self, vault: VaultId, params: CreateCorrespondent) -> Result<Correspondent> {
        let id = Id::new_v4();
        let now = Utc::now();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        sqlx::query("INSERT INTO correspondents (id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, NULL, ?, ?)")
 .bind(id.to_string()).bind(vault.to_string()).bind(&params.name).bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(actor.as_ref().map(UserId::as_str)).bind(actor.as_ref().map(UserId::as_str)).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Correspondent", &params.name))?;
        let value = Correspondent {
            id,
            vault_id: vault,
            name: params.name,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: actor.clone(),
            updated_by: actor,
        };
        emit_taxonomy(&mut tx, "correspondent", "created", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        params: UpdateCorrespondent,
    ) -> Result<Correspondent> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: Correspondent = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Correspondent", id))?;
        if value.deleted_at.is_some() {
            return Err(not_found("Correspondent", id));
        }
        let name = params.name.unwrap_or_else(|| value.name.clone());
        if name == value.name {
            return Ok(value);
        }
        value.name = name;
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE correspondents SET name = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Correspondent", &value.name))?;
        emit_taxonomy(&mut tx, "correspondent", "updated", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: Correspondent = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Correspondent", id))?;
        if value.deleted_at.is_some() {
            return Ok(());
        }
        value.deleted_at = Some(Utc::now());
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE correspondents SET name = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Correspondent", &value.name))?;
        emit_taxonomy(&mut tx, "correspondent", "deleted", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(())
    }
    async fn restore(&self, vault: VaultId, id: Id) -> Result<Correspondent> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<CorrespondentRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM correspondents WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: Correspondent = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("Correspondent", id))?;
        if value.deleted_at.is_none() {
            return Ok(value);
        }
        value.deleted_at = None;
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE correspondents SET name = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "Correspondent", &value.name))?;
        emit_taxonomy(&mut tx, "correspondent", "restored", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
}

pub struct SqliteDocumentTypeRepository {
    pool: SqlitePool,
}
impl SqliteDocumentTypeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
#[async_trait]
impl DocumentTypeRepository for SqliteDocumentTypeRepository {
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<DocumentType>> {
        let row: Option<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE id = ? AND vault_id = ? AND deleted_at IS NULL").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn find_retained(&self, vault: VaultId, id: Id) -> Result<Option<DocumentType>> {
        let row: Option<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<DocumentType>> {
        let row: Option<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE name = ? AND vault_id = ? AND deleted_at IS NULL").bind(name).bind(vault.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx_error)?;
        row.map(TryInto::try_into).transpose()
    }
    async fn list(&self, vault: VaultId) -> Result<Vec<DocumentType>> {
        let rows: Vec<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE vault_id = ? AND deleted_at IS NULL ORDER BY name").bind(vault.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<DocumentType>> {
        let rows: Vec<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE vault_id = ? AND deleted_at IS NOT NULL ORDER BY name").bind(vault.to_string()).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
    async fn create(&self, vault: VaultId, params: CreateDocumentType) -> Result<DocumentType> {
        let id = Id::new_v4();
        let now = Utc::now();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        sqlx::query("INSERT INTO document_types (id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, NULL, ?, ?)")
 .bind(id.to_string()).bind(vault.to_string()).bind(&params.name).bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(actor.as_ref().map(UserId::as_str)).bind(actor.as_ref().map(UserId::as_str)).execute(&mut *tx).await.map_err(|e| map_write_error(e, "DocumentType", &params.name))?;
        let value = DocumentType {
            id,
            vault_id: vault,
            name: params.name,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: actor.clone(),
            updated_by: actor,
        };
        emit_taxonomy(&mut tx, "document_type", "created", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        params: UpdateDocumentType,
    ) -> Result<DocumentType> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: DocumentType = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("DocumentType", id))?;
        if value.deleted_at.is_some() {
            return Err(not_found("DocumentType", id));
        }
        let name = params.name.unwrap_or_else(|| value.name.clone());
        if name == value.name {
            return Ok(value);
        }
        value.name = name;
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE document_types SET name = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "DocumentType", &value.name))?;
        emit_taxonomy(&mut tx, "document_type", "updated", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: DocumentType = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("DocumentType", id))?;
        if value.deleted_at.is_some() {
            return Ok(());
        }
        value.deleted_at = Some(Utc::now());
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE document_types SET name = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "DocumentType", &value.name))?;
        emit_taxonomy(&mut tx, "document_type", "deleted", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(())
    }
    async fn restore(&self, vault: VaultId, id: Id) -> Result<DocumentType> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentTypeRow> = sqlx::query_as("SELECT id, vault_id, name, created_at, updated_at, deleted_at, created_by, updated_by FROM document_types WHERE id = ? AND vault_id = ?").bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let mut value: DocumentType = row
            .map(TryInto::try_into)
            .transpose()?
            .ok_or_else(|| not_found("DocumentType", id))?;
        if value.deleted_at.is_none() {
            return Ok(value);
        }
        value.deleted_at = None;
        value.updated_at = Utc::now();
        value.updated_by = folio_core::mutation::current_actor();
        sqlx::query("UPDATE document_types SET name = ?, updated_at = ?, updated_by = ?, deleted_at = ? WHERE id = ? AND vault_id = ?").bind(&value.name).bind(value.updated_at.to_rfc3339()).bind(value.updated_by.as_ref().map(UserId::as_str)).bind(value.deleted_at.map(|time| time.to_rfc3339())).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(|e| map_write_error(e, "DocumentType", &value.name))?;
        emit_taxonomy(&mut tx, "document_type", "restored", vault, id, &value).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(value)
    }
}
impl SqliteDocumentRepository {
    async fn replace_tags(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        vault: VaultId,
        document_id: Id,
        tag_ids: &[Id],
    ) -> Result<bool> {
        require_document(tx, vault, document_id).await?;
        let mut wanted = tag_ids.to_vec();
        wanted.sort();
        wanted.dedup();
        let current: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM document_tags WHERE document_id = ?")
                .bind(document_id.to_string())
                .fetch_all(&mut **tx)
                .await
                .map_err(map_sqlx_error)?;
        let mut current: Vec<Id> = current
            .into_iter()
            .map(|id| parse_uuid(&id))
            .collect::<Result<_>>()?;
        current.sort();
        for tag in wanted
            .iter()
            .filter(|tag| current.binary_search(tag).is_err())
        {
            require_entry(tx, "tags", "Tag", vault, *tag).await?;
        }
        if current == wanted {
            return Ok(false);
        }
        sqlx::query("DELETE FROM document_tags WHERE document_id = ?")
            .bind(document_id.to_string())
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx_error)?;
        for tag_id in &wanted {
            sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES (?, ?)")
                .bind(document_id.to_string())
                .bind(tag_id.to_string())
                .execute(&mut **tx)
                .await
                .map_err(map_sqlx_error)?;
        }
        Ok(true)
    }
}

impl SqliteDocumentRepository {
    async fn create_reserved(
        &self,
        vault: VaultId,
        id: Id,
        claim: Option<&folio_core::models::LedgerEntry>,
        defer_event: bool,
        params: CreateDocument,
    ) -> Result<Document> {
        let now = Utc::now();
        let ocr_status = if params.content.is_some() {
            ProcessingStatus::Done
        } else {
            ProcessingStatus::Pending
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;

        if let Some(claim) = claim {
            let current: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM ingest_ledger WHERE vault_id = ? AND key = ? AND source_id = ?
                 AND document_id IS ? AND outcome = ? AND created_at = ?",
            )
            .bind(vault.to_string())
            .bind(&claim.key)
            .bind(claim.source_id.to_string())
            .bind(claim.document_id.map(|id| id.to_string()))
            .bind(&claim.outcome)
            .bind(claim.created_at.to_rfc3339())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
            if current.is_none() {
                return Err(Error::Validation("Ingest claim changed".into()));
            }
        }

        if let Some(c) = params.correspondent_id {
            require_entry(&mut tx, "correspondents", "Correspondent", vault, c).await?;
        }
        if let Some(t) = params.document_type_id {
            require_entry(&mut tx, "document_types", "DocumentType", vault, t).await?;
        }
        for tag_id in &params.tag_ids {
            require_entry(&mut tx, "tags", "Tag", vault, *tag_id).await?;
        }

        let inserted = sqlx::query(
            "INSERT INTO documents (id, vault_id, uploaded_by, title, content, original_filename,
                                    mime_type, correspondent_id, document_type_id, ocr_status,
                                    source_type, source_meta, created_at, updated_at, upload_pending, created_by, updated_by)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(params.uploaded_by.as_ref().map(UserId::as_str))
        .bind(&params.title)
        .bind(&params.content)
        .bind(&params.original_filename)
        .bind(&params.mime_type)
        .bind(params.correspondent_id.map(|id| id.to_string()))
        .bind(params.document_type_id.map(|id| id.to_string()))
        .bind(ocr_status.as_str())
        .bind(params.source.kind.as_str())
        .bind(params.source.metadata_json())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(defer_event)
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        if inserted.rows_affected() == 0 {
            tx.rollback().await.map_err(map_sqlx_error)?;
            return self
                .find_by_id(vault, id)
                .await?
                .ok_or_else(|| not_found("Document", id));
        }

        for tag_id in &params.tag_ids {
            sqlx::query(
                "INSERT INTO document_tags (document_id, tag_id) VALUES (?, ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }
        if !defer_event {
            self.emit_in(&mut tx, DocumentEvent::Created { document_id: id })
                .await?;
        }
        self.commit(tx).await?;

        Ok(Document {
            id,
            vault_id: vault,
            uploaded_by: params.uploaded_by,
            created_by: folio_core::mutation::current_actor(),
            updated_by: folio_core::mutation::current_actor(),
            document_date: None,
            reviewed_at: None,
            reviewed_by: None,
            review_origin: None,
            title: params.title,
            content: params.content,
            original_filename: params.original_filename,
            file_path: None,
            file_size: None,
            mime_type: params.mime_type,
            correspondent_id: params.correspondent_id,
            document_type_id: params.document_type_id,
            page_count: None,
            source: params.source,
            ocr_status,
            embed_status: ProcessingStatus::Pending,
            deleted_at: None,
            purge_requested_at: None,
            created_at: now,
            updated_at: now,
        })
    }
}

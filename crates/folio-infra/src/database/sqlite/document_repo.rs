//! SQLite document repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;

use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool, Transaction};
use tokio::sync::Notify;
use uuid::Uuid;

use super::outbox::insert_event;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentSource, DocumentType, Id, ProcessingStage, ProcessingStatus, Tag,
    UpdateCorrespondent, UpdateDocument, UpdateDocumentType, UpdateTag, UserId, VaultId,
};
use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, TagRepository,
};
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
        insert_event(tx, &event, self.origin).await.map(|_| ())
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
const DOC_COLS: &str = "id, vault_id, uploaded_by, title, content, original_filename, file_path,
    mime_type, correspondent_id, document_type_id, page_count, source_type, source_meta,
    ocr_status, embed_status, deleted_at, purge_requested_at, created_at, updated_at";

fn not_found(entity_type: &'static str, id: Id) -> Error {
    Error::NotFound {
        entity_type,
        id: id.to_string(),
    }
}

/// The document must exist in `vault`
async fn require_document(conn: &mut SqliteConnection, vault: VaultId, id: Id) -> Result<()> {
    let found: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM documents WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
    )
    .bind(id.to_string())
    .bind(vault.to_string())
    .fetch_optional(conn)
    .await
    .map_err(map_sqlx_error)?;
    found.map(|_| ()).ok_or_else(|| not_found("Document", id))
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
        "SELECT 1 FROM {table} WHERE id = ? AND vault_id = ?"
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
    title: String,
    content: Option<String>,
    original_filename: Option<String>,
    file_path: Option<String>,
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
            title: row.title,
            content: row.content,
            original_filename: row.original_filename,
            file_path: row.file_path,
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
    name: String,
    color: Option<String>,
    created_at: String,
}

impl TryFrom<TagRow> for Tag {
    type Error = Error;

    fn try_from(row: TagRow) -> Result<Self> {
        Ok(Tag {
            id: parse_uuid(&row.id)?,
            name: row.name,
            color: row.color,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct CorrespondentRow {
    id: String,
    name: String,
    created_at: String,
}

impl TryFrom<CorrespondentRow> for Correspondent {
    type Error = Error;

    fn try_from(row: CorrespondentRow) -> Result<Self> {
        Ok(Correspondent {
            id: parse_uuid(&row.id)?,
            name: row.name,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct DocumentTypeRow {
    id: String,
    name: String,
    created_at: String,
}

impl TryFrom<DocumentTypeRow> for DocumentType {
    type Error = Error;

    fn try_from(row: DocumentTypeRow) -> Result<Self> {
        Ok(DocumentType {
            id: parse_uuid(&row.id)?,
            name: row.name,
            created_at: row
                .created_at
                .parse::<DateTime<Utc>>()
                .map_err(|e| Error::Repository(format!("Invalid datetime: {}", e)))?,
        })
    }
}

/// Append the `WHERE` conditions of a document filter to a query ending in `WHERE ...`
fn push_filter(qb: &mut QueryBuilder<Sqlite>, filter: DocumentFilter) {
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
}

#[async_trait]
impl DocumentRepository for SqliteDocumentRepository {
    async fn locate(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT vault_id FROM documents WHERE id = ? AND deleted_at IS NULL")
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
        let target: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM vaults WHERE id = ?")
            .bind(to.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        if target.is_none() {
            return Err(not_found("Vault", to.0));
        }

        let tags: Vec<(String,)> = sqlx::query_as(
            "SELECT t.name FROM tags t JOIN document_tags dt ON dt.tag_id = t.id
             WHERE dt.document_id = ?",
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
        )
        .await?;
        let document_type = remap_by_name(
            &mut tx,
            "document_types",
            from,
            to,
            doc.document_type_id.as_deref(),
        )
        .await?;

        sqlx::query("DELETE FROM document_tags WHERE document_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        for (name,) in tags {
            let tag_id = match sqlx::query_as::<_, (String,)>(
                "SELECT id FROM tags WHERE vault_id = ? AND name = ?",
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
                    sqlx::query(
                        "INSERT INTO tags (id, vault_id, name, color, created_at)
                         SELECT ?, ?, name, color, created_at FROM tags
                         WHERE vault_id = ? AND name = ?",
                    )
                    .bind(&new_id)
                    .bind(to.to_string())
                    .bind(from.to_string())
                    .bind(&name)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_error)?;
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
                                  updated_at = ?
             WHERE id = ?",
        )
        .bind(to.to_string())
        .bind(correspondent)
        .bind(document_type)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
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
        tx.commit().await.map_err(map_sqlx_error)?;

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
            push_filter(&mut qb, filter);
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
        push_filter(&mut qb, query.filter.clone());
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
            push_filter(&mut qb, filter);
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
        let id = Id::new_v4();
        let now = Utc::now();
        let ocr_status = if params.content.is_some() {
            ProcessingStatus::Done
        } else {
            ProcessingStatus::Pending
        };
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        if let Some(c) = params.correspondent_id {
            require_entry(&mut tx, "correspondents", "Correspondent", vault, c).await?;
        }
        if let Some(t) = params.document_type_id {
            require_entry(&mut tx, "document_types", "DocumentType", vault, t).await?;
        }
        for tag_id in &params.tag_ids {
            require_entry(&mut tx, "tags", "Tag", vault, *tag_id).await?;
        }

        sqlx::query(
            "INSERT INTO documents (id, vault_id, uploaded_by, title, content, original_filename,
                                    mime_type, correspondent_id, document_type_id, ocr_status,
                                    source_type, source_meta, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

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
        self.emit_in(&mut tx, DocumentEvent::Created { document_id: id })
            .await?;
        self.commit(tx).await?;

        Ok(Document {
            id,
            vault_id: vault,
            uploaded_by: params.uploaded_by,
            title: params.title,
            content: params.content,
            original_filename: params.original_filename,
            file_path: None,
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

        let title = params.title.unwrap_or_else(|| existing.title.clone());
        let content = params.content.or_else(|| existing.content.clone());
        let correspondent_id = params.correspondent_id.unwrap_or(existing.correspondent_id);
        let document_type_id = params.document_type_id.unwrap_or(existing.document_type_id);
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
            && !tags_changed
        {
            return Ok(existing);
        }

        let mut conn = self.pool.acquire().await.map_err(map_sqlx_error)?;
        if let Some(c) = correspondent_id.filter(|c| Some(*c) != existing.correspondent_id) {
            require_entry(&mut conn, "correspondents", "Correspondent", vault, c).await?;
        }
        if let Some(t) = document_type_id.filter(|t| Some(*t) != existing.document_type_id) {
            require_entry(&mut conn, "document_types", "DocumentType", vault, t).await?;
        }

        sqlx::query(
            "UPDATE documents SET title = ?, content = ?, correspondent_id = ?,
                                  document_type_id = ?, updated_at = ?
             WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(&title)
        .bind(&content)
        .bind(correspondent_id.map(|id| id.to_string()))
        .bind(document_type_id.map(|id| id.to_string()))
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        self.emit_in(&mut tx, DocumentEvent::Updated { document_id: id })
            .await?;
        self.commit(tx).await?;

        Ok(Document {
            id,
            vault_id: vault,
            uploaded_by: existing.uploaded_by,
            title,
            content,
            original_filename: existing.original_filename,
            file_path: existing.file_path,
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
        if document.deleted_at.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE documents SET deleted_at = ?, updated_at = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL")
            .bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
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
                 updated_at = ? WHERE id = ? AND vault_id = ? AND purge_requested_at IS NULL")
            .bind(Utc::now().to_rfc3339()).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
        if document.file_path.is_some() {
            crate::queue::insert_job_sqlite(
                &mut *tx,
                crate::queue::JobType::ProcessDocument { document_id: id },
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
        if document.deleted_at.is_none() {
            return Err(Error::Conflict(
                "move the document to trash before deleting it forever".into(),
            ));
        }
        if document.purge_requested_at.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE documents SET purge_requested_at = ?, updated_at = ? WHERE id = ? AND vault_id = ?")
            .bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(id.to_string()).bind(vault.to_string()).execute(&mut *tx).await.map_err(map_sqlx_error)?;
        crate::queue::insert_job_sqlite(
            &mut *tx,
            crate::queue::JobType::PurgeDocument { document_id: id },
            now,
            crate::queue::DEFAULT_MAX_ATTEMPTS,
        )
        .await?;
        self.emit_in(&mut tx, DocumentEvent::Trashed { document_id: id })
            .await?;
        self.commit(tx).await
    }

    async fn list_purge_requested(&self) -> Result<Vec<Document>> {
        let rows: Vec<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {DOC_COLS} FROM documents
             WHERE deleted_at IS NOT NULL AND purge_requested_at IS NOT NULL ORDER BY purge_requested_at, id")))
            .fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn finalize_purge(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {DOC_COLS} FROM documents
             WHERE id = ? AND vault_id = ? AND deleted_at IS NOT NULL AND purge_requested_at IS NOT NULL")))
            .bind(id.to_string()).bind(vault.to_string()).fetch_optional(&mut *tx).await.map_err(map_sqlx_error)?;
        let Some(row) = row else {
            return Ok(());
        };
        let document: Document = row.try_into()?;
        let tags: Vec<TagRow> = sqlx::query_as("SELECT t.id, t.name, t.color, t.created_at FROM tags t JOIN document_tags dt ON t.id = dt.tag_id
             WHERE dt.document_id = ? AND t.vault_id = ?")
            .bind(id.to_string()).bind(vault.to_string()).fetch_all(&mut *tx).await.map_err(map_sqlx_error)?;
        let tags = tags
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Tag>>>()?;
        sqlx::query(
            "DELETE FROM documents
             WHERE id = ? AND vault_id = ? AND purge_requested_at IS NOT NULL",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        self.emit_in(
            &mut tx,
            DocumentEvent::Deleted {
                document_id: id,
                snapshot: Some(Arc::new(DeletedDocument { document, tags })),
            },
        )
        .await?;
        self.commit(tx).await
    }

    async fn set_file_path(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE documents SET file_path = ?, updated_at = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(file_path)
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Document", id));
        }

        Ok(())
    }

    async fn set_page_count(&self, vault: VaultId, id: Id, page_count: u32) -> Result<()> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE documents SET page_count = ?, updated_at = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(page_count as i64)
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
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
                "UPDATE documents SET ocr_status = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
            }
            ProcessingStage::Embed => {
                "UPDATE documents SET embed_status = ? WHERE id = ? AND vault_id = ? AND deleted_at IS NULL"
            }
        };
        sqlx::query(sql)
            .bind(status.as_str())
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn complete_ocr(&self, vault: VaultId, id: Id, content: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE documents SET content = ?, ocr_status = 'done', updated_at = ?
             WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(content)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .bind(vault.to_string())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Document", id));
        }
        Ok(())
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
            "SELECT {DOC_COLS} FROM documents WHERE deleted_at IS NULL AND {column} = 'pending' ORDER BY created_at"
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
            "SELECT t.id, t.name, t.color, t.created_at
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
                "SELECT dt.document_id, t.id, t.name, t.color, t.created_at
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
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        require_document(&mut tx, vault, document_id).await?;
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
            self.emit_in(&mut tx, DocumentEvent::Updated { document_id })
                .await?;
        }
        self.commit(tx).await
    }

    async fn remove_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        require_document(&mut tx, vault, document_id).await?;
        let result = sqlx::query("DELETE FROM document_tags WHERE document_id = ? AND tag_id = ?")
            .bind(document_id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        if result.rows_affected() > 0 {
            self.emit_in(&mut tx, DocumentEvent::Updated { document_id })
                .await?;
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
) -> Result<Option<String>> {
    let Some(id) = id else { return Ok(None) };
    let name: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT name FROM {table} WHERE id = ? AND vault_id = ?"
    )))
    .bind(id)
    .bind(from.to_string())
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_sqlx_error)?;
    let Some((name,)) = name else { return Ok(None) };
    let existing: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM {table} WHERE vault_id = ? AND name = ?"
    )))
    .bind(to.to_string())
    .bind(&name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_sqlx_error)?;
    if let Some((existing,)) = existing {
        return Ok(Some(existing));
    }
    let new_id = Id::new_v4().to_string();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} (id, vault_id, name, created_at) VALUES (?, ?, ?, ?)"
    )))
    .bind(&new_id)
    .bind(to.to_string())
    .bind(&name)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await
    .map_err(map_sqlx_error)?;
    Ok(Some(new_id))
}

/// SQLite implementation of TagRepository
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
        let row: Option<TagRow> = sqlx::query_as(
            "SELECT id, name, color, created_at FROM tags WHERE id = ? AND vault_id = ?",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as(
            "SELECT id, name, color, created_at FROM tags WHERE name = ? AND vault_id = ?",
        )
        .bind(name)
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self, vault: VaultId) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT id, name, color, created_at FROM tags WHERE vault_id = ? ORDER BY name",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, vault: VaultId, params: CreateTag) -> Result<Tag> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO tags (id, vault_id, name, color, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(&params.name)
        .bind(&params.color)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| map_write_error(e, "Tag", &params.name))?;

        Ok(Tag {
            id,
            name: params.name,
            color: params.color,
            created_at: now,
        })
    }

    async fn update(&self, vault: VaultId, id: Id, params: UpdateTag) -> Result<Tag> {
        let existing = self
            .find_by_id(vault, id)
            .await?
            .ok_or_else(|| not_found("Tag", id))?;

        let name = params.name.unwrap_or(existing.name);
        let color = params.color.unwrap_or(existing.color);

        sqlx::query("UPDATE tags SET name = ?, color = ? WHERE id = ? AND vault_id = ?")
            .bind(&name)
            .bind(&color)
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "Tag", &name))?;

        Ok(Tag {
            id,
            name,
            color,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM tags WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Tag", id));
        }

        Ok(())
    }
}

/// SQLite implementation of CorrespondentRepository
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
        let row: Option<CorrespondentRow> = sqlx::query_as(
            "SELECT id, name, created_at FROM correspondents WHERE id = ? AND vault_id = ?",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Correspondent>> {
        let row: Option<CorrespondentRow> = sqlx::query_as(
            "SELECT id, name, created_at FROM correspondents WHERE name = ? AND vault_id = ?",
        )
        .bind(name)
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self, vault: VaultId) -> Result<Vec<Correspondent>> {
        let rows: Vec<CorrespondentRow> = sqlx::query_as(
            "SELECT id, name, created_at FROM correspondents WHERE vault_id = ? ORDER BY name",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, vault: VaultId, params: CreateCorrespondent) -> Result<Correspondent> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO correspondents (id, vault_id, name, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(&params.name)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| map_write_error(e, "Correspondent", &params.name))?;

        Ok(Correspondent {
            id,
            name: params.name,
            created_at: now,
        })
    }

    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        params: UpdateCorrespondent,
    ) -> Result<Correspondent> {
        let existing = self
            .find_by_id(vault, id)
            .await?
            .ok_or_else(|| not_found("Correspondent", id))?;

        let name = params.name.unwrap_or(existing.name);

        sqlx::query("UPDATE correspondents SET name = ? WHERE id = ? AND vault_id = ?")
            .bind(&name)
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "Correspondent", &name))?;

        Ok(Correspondent {
            id,
            name,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM correspondents WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Correspondent", id));
        }

        Ok(())
    }
}

/// SQLite implementation of DocumentTypeRepository
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
        let row: Option<DocumentTypeRow> = sqlx::query_as(
            "SELECT id, name, created_at FROM document_types WHERE id = ? AND vault_id = ?",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<DocumentType>> {
        let row: Option<DocumentTypeRow> = sqlx::query_as(
            "SELECT id, name, created_at FROM document_types WHERE name = ? AND vault_id = ?",
        )
        .bind(name)
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self, vault: VaultId) -> Result<Vec<DocumentType>> {
        let rows: Vec<DocumentTypeRow> = sqlx::query_as(
            "SELECT id, name, created_at FROM document_types WHERE vault_id = ? ORDER BY name",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, vault: VaultId, params: CreateDocumentType) -> Result<DocumentType> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO document_types (id, vault_id, name, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(&params.name)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| map_write_error(e, "DocumentType", &params.name))?;

        Ok(DocumentType {
            id,
            name: params.name,
            created_at: now,
        })
    }

    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        params: UpdateDocumentType,
    ) -> Result<DocumentType> {
        let existing = self
            .find_by_id(vault, id)
            .await?
            .ok_or_else(|| not_found("DocumentType", id))?;

        let name = params.name.unwrap_or(existing.name);

        sqlx::query("UPDATE document_types SET name = ? WHERE id = ? AND vault_id = ?")
            .bind(&name)
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "DocumentType", &name))?;

        Ok(DocumentType {
            id,
            name,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM document_types WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(not_found("DocumentType", id));
        }

        Ok(())
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
        for tag in tag_ids {
            require_entry(tx, "tags", "Tag", vault, *tag).await?;
        }
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

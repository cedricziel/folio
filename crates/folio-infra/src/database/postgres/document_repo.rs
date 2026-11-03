//! PostgreSQL document, tag, correspondent and document type repositories

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder, Transaction};
use tokio::sync::Notify;
use uuid::Uuid;

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

use super::map_write_error;
use super::outbox::{capture_document, insert_event};

fn repo_err(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

/// Escape LIKE wildcards so user input matches literally
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn not_found(entity_type: &'static str, id: Id) -> Error {
    Error::NotFound {
        entity_type,
        id: id.to_string(),
    }
}

/// The document must exist in `vault`
async fn require_document(conn: &mut PgConnection, vault: VaultId, id: Id) -> Result<()> {
    let found: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM documents WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .bind(vault.0)
    .fetch_optional(conn)
    .await
    .map_err(repo_err)?;
    found.map(|_| ()).ok_or_else(|| not_found("Document", id))
}

async fn stamp_document(conn: &mut PgConnection, vault: VaultId, id: Id) -> Result<()> {
    sqlx::query(
        "UPDATE documents SET updated_at = $1, updated_by = $2 WHERE id = $3 AND vault_id = $4",
    )
    .bind(Utc::now())
    .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
    .bind(id)
    .bind(vault.0)
    .execute(conn)
    .await
    .map_err(repo_err)?;
    Ok(())
}

/// A vocabulary entry must exist in `vault`; `table` is one of ours, never user input
async fn require_entry(
    conn: &mut PgConnection,
    table: &'static str,
    entity_type: &'static str,
    vault: VaultId,
    id: Id,
) -> Result<()> {
    let found: Option<(i32,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT 1 FROM {table} WHERE id = $1 AND vault_id = $2"
    )))
    .bind(id)
    .bind(vault.0)
    .fetch_optional(conn)
    .await
    .map_err(repo_err)?;
    found.map(|_| ()).ok_or_else(|| not_found(entity_type, id))
}

/// The entry of `table` in `to` that carries the same name as entry `id` of
/// `from`, created when the target has none
async fn remap_by_name(
    conn: &mut PgConnection,
    table: &'static str,
    from: VaultId,
    to: VaultId,
    id: Option<Id>,
) -> Result<Option<Id>> {
    let Some(id) = id else { return Ok(None) };
    let name: Option<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT name FROM {table} WHERE id = $1 AND vault_id = $2"
    )))
    .bind(id)
    .bind(from.0)
    .fetch_optional(&mut *conn)
    .await
    .map_err(repo_err)?;
    let Some((name,)) = name else { return Ok(None) };
    let new_id = Id::new_v4();
    let (target,): (Id,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} (id, vault_id, name, created_at) VALUES ($1, $2, $3, now())
         ON CONFLICT (vault_id, name) DO UPDATE SET name = excluded.name
         RETURNING id"
    )))
    .bind(new_id)
    .bind(to.0)
    .bind(&name)
    .fetch_one(&mut *conn)
    .await
    .map_err(repo_err)?;
    Ok(Some(target))
}

/// `SELECT` over `documents` with the full column list; sqlx requires static SQL.
macro_rules! select_documents {
    ($tail:literal) => {
        concat!(
            "SELECT id, vault_id, uploaded_by, created_by, updated_by, title, content, original_filename, file_path, ",
            "mime_type, ",
            "correspondent_id, document_type_id, page_count, source_type, source_meta, ",
            "ocr_status, embed_status, deleted_at, purge_requested_at, ",
            "created_at, updated_at ",
            "FROM documents ",
            $tail
        )
    };
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: Uuid,
    vault_id: Uuid,
    uploaded_by: Option<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
    title: String,
    content: Option<String>,
    original_filename: Option<String>,
    file_path: Option<String>,
    mime_type: Option<String>,
    correspondent_id: Option<Uuid>,
    document_type_id: Option<Uuid>,
    page_count: Option<i32>,
    source_type: String,
    source_meta: Option<String>,
    ocr_status: String,
    embed_status: String,
    deleted_at: Option<DateTime<Utc>>,
    purge_requested_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<DocumentRow> for Document {
    type Error = Error;

    fn try_from(row: DocumentRow) -> Result<Self> {
        Ok(Document {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            uploaded_by: row.uploaded_by.map(UserId),
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
            title: row.title,
            content: row.content,
            original_filename: row.original_filename,
            file_path: row.file_path,
            mime_type: row.mime_type,
            correspondent_id: row.correspondent_id,
            document_type_id: row.document_type_id,
            page_count: row.page_count.map(|c| c as u32),
            source: DocumentSource::from_stored(&row.source_type, row.source_meta.as_deref()),
            ocr_status: row.ocr_status.parse().map_err(Error::Repository)?,
            embed_status: row.embed_status.parse().map_err(Error::Repository)?,
            deleted_at: row.deleted_at,
            purge_requested_at: row.purge_requested_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct DocumentTagRow {
    document_id: Uuid,
    #[sqlx(flatten)]
    tag: TagRow,
}

#[derive(sqlx::FromRow)]
struct TagRow {
    id: Uuid,
    vault_id: Uuid,
    name: String,
    color: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<TagRow> for Tag {
    fn from(row: TagRow) -> Self {
        Tag {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            name: row.name,
            color: row.color,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct NamedRow {
    id: Uuid,
    vault_id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
}

impl From<NamedRow> for Correspondent {
    fn from(row: NamedRow) -> Self {
        Correspondent {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            name: row.name,
            created_at: row.created_at,
        }
    }
}

impl From<NamedRow> for DocumentType {
    fn from(row: NamedRow) -> Self {
        DocumentType {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            name: row.name,
            created_at: row.created_at,
        }
    }
}

/// PostgreSQL implementation of DocumentRepository
///
/// Lifecycle changes (create, update, delete, tag edits) write their outbox
/// event in the same transaction, so a change is never committed without it.
pub struct PostgresDocumentRepository {
    pool: PgPool,
    origin: EventOrigin,
    wake: Arc<Notify>,
}

impl PostgresDocumentRepository {
    pub fn new(pool: PgPool) -> Self {
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

    async fn emit_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        event: DocumentEvent,
    ) -> Result<()> {
        let context = capture_document(tx, event.document_id(), None)
            .await?
            .ok_or_else(|| {
                Error::Repository("Document vanished before its event was captured".into())
            })?;
        insert_event(tx, &event, self.origin, Some(&context))
            .await
            .map(|_| ())
    }

    async fn commit(&self, tx: Transaction<'_, Postgres>) -> Result<()> {
        tx.commit().await.map_err(repo_err)?;
        self.wake.notify_one();
        Ok(())
    }
}

/// Append the `WHERE` conditions of a document filter to a query ending in `WHERE ...`
fn push_filter(qb: &mut QueryBuilder<Postgres>, filter: DocumentFilter) {
    if let Some(id) = filter.correspondent_id {
        qb.push(" AND correspondent_id = ").push_bind(id);
    }
    if let Some(id) = filter.document_type_id {
        qb.push(" AND document_type_id = ").push_bind(id);
    }
    if let Some(mut tag_ids) = filter.tag_ids.filter(|t| !t.is_empty()) {
        // Duplicates would make the distinct count below unreachable
        tag_ids.sort();
        tag_ids.dedup();
        qb.push(" AND id IN (SELECT document_id FROM document_tags WHERE tag_id IN (");
        let mut sep = qb.separated(", ");
        for tag_id in &tag_ids {
            sep.push_bind(*tag_id);
        }
        qb.push(") GROUP BY document_id HAVING COUNT(DISTINCT tag_id) = ")
            .push_bind(tag_ids.len() as i64)
            .push(")");
    }
    if let Some(query) = filter.search_query {
        for term in query.split_whitespace() {
            let pattern = format!("%{}%", escape_like(term));
            qb.push(" AND (title ILIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' OR content ILIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\')");
        }
    }
    if let Some(after) = filter.created_after {
        qb.push(" AND created_at >= ").push_bind(after);
    }
    if let Some(before) = filter.created_before {
        qb.push(" AND created_at < ").push_bind(before);
    }
}

#[async_trait]
impl DocumentRepository for PostgresDocumentRepository {
    async fn locate(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(Uuid,)> =
            sqlx::query_as("SELECT vault_id FROM documents WHERE id = $1 AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(|(v,)| VaultId(v)))
    }

    async fn move_to(
        &self,
        id: Id,
        from: VaultId,
        to: VaultId,
        actor: Option<&UserId>,
    ) -> Result<Document> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let doc: DocumentRow = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .bind(from.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| not_found("Document", id))?;
        if from == to {
            return doc.try_into();
        }
        let target: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM vaults WHERE id = $1")
            .bind(to.0)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?;
        if target.is_none() {
            return Err(not_found("Vault", to.0));
        }

        let tags: Vec<(String,)> = sqlx::query_as(
            "SELECT t.name FROM tags t JOIN document_tags dt ON dt.tag_id = t.id
             WHERE dt.document_id = $1",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(repo_err)?;

        let correspondent =
            remap_by_name(&mut tx, "correspondents", from, to, doc.correspondent_id).await?;
        let document_type =
            remap_by_name(&mut tx, "document_types", from, to, doc.document_type_id).await?;

        sqlx::query("DELETE FROM document_tags WHERE document_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        for (name,) in tags {
            let (tag_id,): (Id,) = sqlx::query_as(
                "INSERT INTO tags (id, vault_id, name, color, created_at)
                 SELECT $1, $2, name, color, created_at FROM tags WHERE vault_id = $3 AND name = $4
                 ON CONFLICT (vault_id, name) DO UPDATE SET name = excluded.name
                 RETURNING id",
            )
            .bind(Id::new_v4())
            .bind(to.0)
            .bind(from.0)
            .bind(&name)
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?;
            sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2)")
                .bind(id)
                .bind(tag_id)
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
        }

        sqlx::query(
            "UPDATE documents SET vault_id = $1, correspondent_id = $2, document_type_id = $3,
                                  updated_at = now(), updated_by = $5
             WHERE id = $4",
        )
        .bind(to.0)
        .bind(correspondent)
        .bind(document_type)
        .bind(id)
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        for vault in [from, to] {
            sqlx::query(
                "INSERT INTO audit_events
                     (id, at, actor_user_id, action, vault_id, target_type, target_id, details)
                 VALUES ($1, now(), $2, 'document.moved', $3, 'document', $4, $5)",
            )
            .bind(Id::new_v4())
            .bind(actor.map(UserId::as_str))
            .bind(vault.0)
            .bind(id.to_string())
            .bind(serde_json::json!({ "from": from, "to": to }))
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        tx.commit().await.map_err(repo_err)?;

        self.find_by_id(to, id)
            .await?
            .ok_or_else(|| not_found("Document", id))
    }

    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL"
        ))
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        let mut qb = QueryBuilder::<Postgres>::new(select_documents!(
            "WHERE deleted_at IS NULL AND vault_id = "
        ));
        qb.push_bind(vault.0);

        if let Some(filter) = filter {
            push_filter(&mut qb, filter);
        }

        qb.push(" ORDER BY created_at DESC");

        let rows: Vec<DocumentRow> = qb
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn page(
        &self,
        vault: VaultId,
        query: folio_core::models::DocumentPageQuery,
    ) -> Result<folio_core::models::DocumentPage> {
        use folio_core::models::DocumentSort;
        query.validate(vault)?;
        let mut qb = QueryBuilder::<Postgres>::new(select_documents!(
            "WHERE deleted_at IS NULL AND vault_id = "
        ));
        qb.push_bind(vault.0);
        push_filter(&mut qb, query.filter.clone());
        let (column, direction, comparison) = match query.sort {
            DocumentSort::Newest => ("created_at", "DESC", " < "),
            DocumentSort::Oldest => ("created_at", "ASC", " > "),
            DocumentSort::Title => ("title COLLATE \"C\"", "ASC", " > "),
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
                    qb.push_bind(cursor.created_at);
                }
            }
            qb.push(", ").push_bind(cursor.id).push(")");
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
            .map_err(repo_err)?;
        let documents = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>>>()?;
        Ok(query.finish(vault, documents))
    }

    async fn list_ids(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Id>> {
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT id FROM documents WHERE deleted_at IS NULL AND vault_id = ",
        );
        qb.push_bind(vault.0);
        if let Some(filter) = filter {
            push_filter(&mut qb, filter);
        }
        qb.push(" ORDER BY created_at DESC");

        qb.build_query_scalar()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)
    }

    async fn create(&self, vault: VaultId, params: CreateDocument) -> Result<Document> {
        self.create_reserved(vault, Id::new_v4(), None, false, params)
            .await
    }

    async fn create_for_upload(&self, vault: VaultId, params: CreateDocument) -> Result<Document> {
        self.create_reserved(vault, Id::new_v4(), None, true, params)
            .await
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

    async fn update(&self, vault: VaultId, id: Id, params: UpdateDocument) -> Result<Document> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR UPDATE"
        ))
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
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

        let mut conn = self.pool.acquire().await.map_err(repo_err)?;
        if let Some(c) = correspondent_id.filter(|c| Some(*c) != existing.correspondent_id) {
            require_entry(&mut conn, "correspondents", "Correspondent", vault, c).await?;
        }
        if let Some(t) = document_type_id.filter(|t| Some(*t) != existing.document_type_id) {
            require_entry(&mut conn, "document_types", "DocumentType", vault, t).await?;
        }

        sqlx::query(
            "UPDATE documents SET title = $1, content = $2, correspondent_id = $3,
                                  document_type_id = $4, updated_at = $5, updated_by = $8
             WHERE id = $6 AND vault_id = $7 AND deleted_at IS NULL",
        )
        .bind(&title)
        .bind(&content)
        .bind(correspondent_id)
        .bind(document_type_id)
        .bind(now)
        .bind(id)
        .bind(vault.0)
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        self.emit_in(&mut tx, DocumentEvent::Updated { document_id: id })
            .await?;
        self.commit(tx).await?;

        Ok(Document {
            id,
            vault_id: vault,
            uploaded_by: existing.uploaded_by,
            created_by: existing.created_by,
            updated_by: folio_core::mutation::current_actor(),
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
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 FOR UPDATE"
        ))
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
        let document: Document = row.ok_or_else(|| not_found("Document", id))?.try_into()?;
        if document.deleted_at.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE documents SET deleted_at = $3, updated_at = $3, updated_by = $4 WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL")
            .bind(id).bind(vault.0).bind(now).bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx).await.map_err(repo_err)?;
        self.emit_in(&mut tx, DocumentEvent::Trashed { document_id: id })
            .await?;
        self.commit(tx).await
    }

    async fn locate_retained(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(Uuid,)> = sqlx::query_as("SELECT vault_id FROM documents WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(row.map(|(v,)| VaultId(v)))
    }

    async fn is_purging_or_missing(&self, id: Id) -> Result<bool> {
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM documents WHERE id = $1 AND purge_requested_at IS NULL)")
            .bind(id).fetch_one(&self.pool).await.map_err(repo_err)
    }

    async fn locate_trashed(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT vault_id FROM documents
             WHERE id = $1 AND deleted_at IS NOT NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(|(v,)| VaultId(v)))
    }

    async fn find_trashed_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 AND deleted_at IS NOT NULL"
        ))
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Document>> {
        let rows: Vec<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE vault_id = $1 AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, id DESC"
        ))
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn restore(&self, vault: VaultId, id: Id) -> Result<Document> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 FOR UPDATE"
        ))
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
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
                 updated_at = $3, updated_by = $4 WHERE id = $1 AND vault_id = $2 AND purge_requested_at IS NULL")
            .bind(id).bind(vault.0).bind(Utc::now()).bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx).await.map_err(repo_err)?;
        if document.file_path.is_some() {
            crate::queue::insert_job_postgres(
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
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE id = $1 AND vault_id = $2 FOR UPDATE"
        ))
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
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
        sqlx::query("UPDATE documents SET purge_requested_at = $3, updated_at = $3, updated_by = $4 WHERE id = $1 AND vault_id = $2")
            .bind(id).bind(vault.0).bind(now).bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx).await.map_err(repo_err)?;
        crate::queue::insert_job_postgres(
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
        let rows: Vec<DocumentRow> = sqlx::query_as(select_documents!("WHERE deleted_at IS NOT NULL AND purge_requested_at IS NOT NULL ORDER BY purge_requested_at, id"))
            .fetch_all(&self.pool).await.map_err(repo_err)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn finalize_purge(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query("SELECT key FROM ingest_ledger WHERE document_id = $1 FOR UPDATE")
            .bind(id)
            .fetch_all(&mut *tx)
            .await
            .map_err(repo_err)?;
        let row: Option<DocumentRow> = sqlx::query_as(sqlx::AssertSqlSafe(select_documents!("WHERE id = $1 AND vault_id = $2 AND deleted_at IS NOT NULL AND purge_requested_at IS NOT NULL FOR UPDATE").replace(" content,", " NULL AS content,")))
            .bind(id).bind(vault.0).fetch_optional(&mut *tx).await.map_err(repo_err)?;
        let Some(row) = row else {
            return Ok(());
        };
        let context = capture_document(&mut tx, id, None)
            .await?
            .ok_or_else(|| not_found("Document", id))?;
        let document: Document = row.try_into()?;
        let tags: Vec<TagRow> = sqlx::query_as("SELECT t.id, t.vault_id, t.name, t.color, t.created_at FROM tags t JOIN document_tags dt ON t.id = dt.tag_id
             WHERE dt.document_id = $1 AND t.vault_id = $2")
            .bind(id).bind(vault.0).fetch_all(&mut *tx).await.map_err(repo_err)?;
        let tags = tags.into_iter().map(Into::into).collect();
        sqlx::query("UPDATE ingest_ledger SET outcome = 'deleted' WHERE document_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        sqlx::query(
            "DELETE FROM documents
             WHERE id = $1 AND vault_id = $2 AND purge_requested_at IS NOT NULL",
        )
        .bind(id)
        .bind(vault.0)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        insert_event(
            &mut tx,
            &DocumentEvent::Deleted {
                document_id: id,
                snapshot: Some(Arc::new(DeletedDocument { document, tags })),
            },
            self.origin,
            Some(&context),
        )
        .await?;
        self.commit(tx).await
    }

    async fn finalize_upload(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let (pending, existing_path): (bool, Option<String>) = sqlx::query_as("SELECT upload_pending, file_path FROM documents WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR UPDATE")
            .bind(id).bind(vault.0).fetch_optional(&mut *tx).await.map_err(repo_err)?
            .ok_or_else(|| not_found("Document", id))?;
        if !pending && existing_path.as_deref() == Some(file_path) {
            return Ok(());
        }
        sqlx::query("UPDATE documents SET file_path = $1, upload_pending = FALSE, updated_at = $2, updated_by = $5 WHERE id = $3 AND vault_id = $4")
            .bind(file_path).bind(Utc::now()).bind(id).bind(vault.0).bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx).await.map_err(repo_err)?;
        if pending {
            self.emit_in(&mut tx, DocumentEvent::Created { document_id: id })
                .await?;
        }
        self.commit(tx).await
    }

    async fn set_file_path(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()> {
        let result = sqlx::query("UPDATE documents SET file_path = $1, updated_at = $2, updated_by = $5 WHERE id = $3 AND vault_id = $4 AND deleted_at IS NULL AND file_path IS DISTINCT FROM $1").bind(file_path).bind(Utc::now()).bind(id).bind(vault.0).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).execute(&self.pool).await.map_err(repo_err)?;
        if result.rows_affected() == 0 && self.find_by_id(vault, id).await?.is_none() {
            return Err(not_found("Document", id));
        }
        Ok(())
    }

    async fn set_page_count(&self, vault: VaultId, id: Id, page_count: u32) -> Result<()> {
        let result = sqlx::query("UPDATE documents SET page_count = $1, updated_at = $2, updated_by = $5 WHERE id = $3 AND vault_id = $4 AND deleted_at IS NULL AND page_count IS DISTINCT FROM $1").bind(page_count as i32).bind(Utc::now()).bind(id).bind(vault.0).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).execute(&self.pool).await.map_err(repo_err)?;
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
                "UPDATE documents SET ocr_status = $1, updated_at = $4, updated_by = $5 WHERE id = $2 AND vault_id = $3 AND deleted_at IS NULL AND ocr_status != $1"
            }
            ProcessingStage::Embed => {
                "UPDATE documents SET embed_status = $1, updated_at = $4, updated_by = $5 WHERE id = $2 AND vault_id = $3 AND deleted_at IS NULL AND embed_status != $1"
            }
        };
        sqlx::query(sql)
            .bind(status.as_str())
            .bind(id)
            .bind(vault.0)
            .bind(Utc::now())
            .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn complete_ocr(&self, vault: VaultId, id: Id, content: &str) -> Result<()> {
        let result = sqlx::query("UPDATE documents SET content = $1, ocr_status = 'done', updated_at = $2, updated_by = $5 WHERE id = $3 AND vault_id = $4 AND deleted_at IS NULL AND (content IS DISTINCT FROM $1 OR ocr_status != 'done')").bind(content).bind(Utc::now()).bind(id).bind(vault.0).bind(folio_core::mutation::current_actor().map(|actor| actor.0)).execute(&self.pool).await.map_err(repo_err)?;
        if result.rows_affected() == 0 && self.find_by_id(vault, id).await?.is_none() {
            return Err(not_found("Document", id));
        }
        Ok(())
    }

    async fn list_all(&self) -> Result<Vec<Document>> {
        let rows: Vec<DocumentRow> = sqlx::query_as(select_documents!(
            "WHERE deleted_at IS NULL ORDER BY created_at DESC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>> {
        let sql = match stage {
            ProcessingStage::Ocr => {
                select_documents!(
                    "WHERE deleted_at IS NULL AND ocr_status = 'pending' ORDER BY created_at"
                )
            }
            ProcessingStage::Embed => {
                select_documents!(
                    "WHERE deleted_at IS NULL AND embed_status = 'pending' ORDER BY created_at"
                )
            }
        };
        let rows: Vec<DocumentRow> = sqlx::query_as(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn reset_embedding_status(&self) -> Result<Vec<Id>> {
        sqlx::query_scalar(
            "UPDATE documents SET updated_at = CASE WHEN embed_status != 'pending' THEN $1 ELSE updated_at END, updated_by = CASE WHEN embed_status != 'pending' THEN $2 ELSE updated_by END, embed_status = 'pending'
             WHERE deleted_at IS NULL AND content IS NOT NULL AND content != ''
             RETURNING id",
        )
        .bind(Utc::now()).bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)
    }

    async fn get_tags(&self, vault: VaultId, document_id: Id) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT t.id, t.vault_id, t.name, t.color, t.created_at
             FROM tags t
             INNER JOIN document_tags dt ON t.id = dt.tag_id
             INNER JOIN documents d ON d.id = dt.document_id
             WHERE dt.document_id = $1 AND d.deleted_at IS NULL AND d.vault_id = $2 AND t.vault_id = d.vault_id",
        )
        .bind(document_id)
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
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
            let mut qb = QueryBuilder::<Postgres>::new(
                "SELECT dt.document_id, t.id, t.vault_id, t.name, t.color, t.created_at
                 FROM document_tags dt JOIN tags t ON t.id = dt.tag_id
                 JOIN documents d ON d.id = dt.document_id
                 WHERE t.vault_id = d.vault_id AND d.deleted_at IS NULL AND d.vault_id = ",
            );
            qb.push_bind(vault.0).push(" AND d.id IN (");
            let mut separated = qb.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
            qb.push(") ORDER BY t.name, t.id");
            let rows: Vec<DocumentTagRow> = qb
                .build_query_as()
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
            for row in rows {
                result
                    .entry(row.document_id)
                    .or_default()
                    .push(row.tag.into());
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
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        require_document(&mut tx, vault, document_id).await?;
        require_entry(&mut tx, "tags", "Tag", vault, tag_id).await?;
        let result = sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(document_id).bind(tag_id).execute(&mut *tx).await.map_err(repo_err)?;
        if result.rows_affected() > 0 {
            stamp_document(&mut tx, vault, document_id).await?;
            self.emit_in(&mut tx, DocumentEvent::Updated { document_id })
                .await?;
        }
        self.commit(tx).await
    }

    async fn remove_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        require_document(&mut tx, vault, document_id).await?;
        let result =
            sqlx::query("DELETE FROM document_tags WHERE document_id = $1 AND tag_id = $2")
                .bind(document_id)
                .bind(tag_id)
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
        if result.rows_affected() > 0 {
            stamp_document(&mut tx, vault, document_id).await?;
            self.emit_in(&mut tx, DocumentEvent::Updated { document_id })
                .await?;
        }
        self.commit(tx).await
    }
}

/// PostgreSQL implementation of TagRepository
pub struct PostgresTagRepository {
    pool: PgPool,
}

impl PostgresTagRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TagRepository for PostgresTagRepository {
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as(
            "SELECT id, vault_id, name, color, created_at FROM tags WHERE id = $1 AND vault_id = $2",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as(
            "SELECT id, vault_id, name, color, created_at FROM tags WHERE name = $1 AND vault_id = $2",
        )
        .bind(name)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self, vault: VaultId) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT id, vault_id, name, color, created_at FROM tags WHERE vault_id = $1 ORDER BY name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, vault: VaultId, params: CreateTag) -> Result<Tag> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO tags (id, vault_id, name, color, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(vault.0)
        .bind(&params.name)
        .bind(&params.color)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| map_write_error(e, "Tag", &params.name))?;

        Ok(Tag {
            id,
            vault_id: vault,
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

        sqlx::query("UPDATE tags SET name = $1, color = $2 WHERE id = $3 AND vault_id = $4")
            .bind(&name)
            .bind(&color)
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "Tag", &name))?;

        Ok(Tag {
            id,
            vault_id: vault,
            name,
            color,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM tags WHERE id = $1 AND vault_id = $2")
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Tag", id));
        }
        Ok(())
    }
}

/// PostgreSQL implementation of CorrespondentRepository
pub struct PostgresCorrespondentRepository {
    pool: PgPool,
}

impl PostgresCorrespondentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CorrespondentRepository for PostgresCorrespondentRepository {
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Correspondent>> {
        let row: Option<NamedRow> = sqlx::query_as(
            "SELECT id, vault_id, name, created_at FROM correspondents WHERE id = $1 AND vault_id = $2",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Correspondent>> {
        let row: Option<NamedRow> = sqlx::query_as(
            "SELECT id, vault_id, name, created_at FROM correspondents WHERE name = $1 AND vault_id = $2",
        )
        .bind(name)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self, vault: VaultId) -> Result<Vec<Correspondent>> {
        let rows: Vec<NamedRow> = sqlx::query_as(
            "SELECT id, vault_id, name, created_at FROM correspondents WHERE vault_id = $1 ORDER BY name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, vault: VaultId, params: CreateCorrespondent) -> Result<Correspondent> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO correspondents (id, vault_id, name, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(vault.0)
        .bind(&params.name)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| map_write_error(e, "Correspondent", &params.name))?;

        Ok(Correspondent {
            id,
            vault_id: vault,
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

        sqlx::query("UPDATE correspondents SET name = $1 WHERE id = $2 AND vault_id = $3")
            .bind(&name)
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "Correspondent", &name))?;

        Ok(Correspondent {
            id,
            vault_id: vault,
            name,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM correspondents WHERE id = $1 AND vault_id = $2")
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Correspondent", id));
        }
        Ok(())
    }
}

/// PostgreSQL implementation of DocumentTypeRepository
pub struct PostgresDocumentTypeRepository {
    pool: PgPool,
}

impl PostgresDocumentTypeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DocumentTypeRepository for PostgresDocumentTypeRepository {
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<DocumentType>> {
        let row: Option<NamedRow> = sqlx::query_as(
            "SELECT id, vault_id, name, created_at FROM document_types WHERE id = $1 AND vault_id = $2",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<DocumentType>> {
        let row: Option<NamedRow> = sqlx::query_as(
            "SELECT id, vault_id, name, created_at FROM document_types WHERE name = $1 AND vault_id = $2",
        )
        .bind(name)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self, vault: VaultId) -> Result<Vec<DocumentType>> {
        let rows: Vec<NamedRow> = sqlx::query_as(
            "SELECT id, vault_id, name, created_at FROM document_types WHERE vault_id = $1 ORDER BY name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, vault: VaultId, params: CreateDocumentType) -> Result<DocumentType> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO document_types (id, vault_id, name, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(vault.0)
        .bind(&params.name)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| map_write_error(e, "DocumentType", &params.name))?;

        Ok(DocumentType {
            id,
            vault_id: vault,
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

        sqlx::query("UPDATE document_types SET name = $1 WHERE id = $2 AND vault_id = $3")
            .bind(&name)
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "DocumentType", &name))?;

        Ok(DocumentType {
            id,
            vault_id: vault,
            name,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM document_types WHERE id = $1 AND vault_id = $2")
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        if result.rows_affected() == 0 {
            return Err(not_found("DocumentType", id));
        }
        Ok(())
    }
}

impl PostgresDocumentRepository {
    async fn replace_tags(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
        let current: Vec<Id> =
            sqlx::query_scalar("SELECT tag_id FROM document_tags WHERE document_id = $1")
                .bind(document_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(repo_err)?;
        let mut current = current;
        current.sort();
        if current == wanted {
            return Ok(false);
        }
        sqlx::query("DELETE FROM document_tags WHERE document_id = $1")
            .bind(document_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        for tag_id in &wanted {
            sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2)")
                .bind(document_id)
                .bind(*tag_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }
        Ok(true)
    }
}

impl PostgresDocumentRepository {
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
        let mut tx = self.pool.begin().await.map_err(repo_err)?;

        if let Some(claim) = claim {
            let current: Option<(i32,)> = sqlx::query_as(
                "SELECT 1 FROM ingest_ledger WHERE vault_id = $1 AND key = $2 AND source_id = $3
                 AND document_id IS NOT DISTINCT FROM $4 AND outcome = $5 AND created_at = $6 FOR UPDATE")
                .bind(vault.0).bind(&claim.key).bind(claim.source_id).bind(claim.document_id)
                .bind(&claim.outcome).bind(claim.created_at).fetch_optional(&mut *tx).await.map_err(repo_err)?;
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13, $14, $15, $16) ON CONFLICT(id) DO NOTHING",
        )
        .bind(id)
        .bind(vault.0)
        .bind(params.uploaded_by.as_ref().map(UserId::as_str))
        .bind(&params.title)
        .bind(&params.content)
        .bind(&params.original_filename)
        .bind(&params.mime_type)
        .bind(params.correspondent_id)
        .bind(params.document_type_id)
        .bind(ocr_status.as_str())
        .bind(params.source.kind.as_str())
        .bind(params.source.metadata_json())
        .bind(now)
        .bind(defer_event)
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .bind(folio_core::mutation::current_actor().map(|actor| actor.0))
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        if inserted.rows_affected() == 0 {
            tx.rollback().await.map_err(repo_err)?;
            return self
                .find_by_id(vault, id)
                .await?
                .ok_or_else(|| not_found("Document", id));
        }

        for tag_id in &params.tag_ids {
            sqlx::query(
                "INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
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
}

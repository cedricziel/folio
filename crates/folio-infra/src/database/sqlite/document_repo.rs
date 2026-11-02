//! SQLite document repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;

use sqlx::{QueryBuilder, Sqlite, SqlitePool, Transaction};
use tokio::sync::Notify;
use uuid::Uuid;

use super::outbox::insert_event;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentSource, DocumentType, Id, ProcessingStage, ProcessingStatus, Tag,
    UpdateCorrespondent, UpdateDocument, UpdateDocumentType, UpdateTag,
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

// Row types for SQLx queries
#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
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
    created_at: String,
    updated_at: String,
}

impl TryFrom<DocumentRow> for Document {
    type Error = Error;

    fn try_from(row: DocumentRow) -> Result<Self> {
        Ok(Document {
            id: parse_uuid(&row.id)?,
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

#[async_trait]
impl DocumentRepository for SqliteDocumentRepository {
    async fn find_by_id(&self, id: Id) -> Result<Option<Document>> {
        let id_str = id.to_string();
        let row: Option<DocumentRow> = sqlx::query_as(
            "SELECT id, title, content, original_filename, file_path, mime_type,
                    correspondent_id, document_type_id, page_count, source_type, source_meta, ocr_status, embed_status, created_at, updated_at
             FROM documents WHERE id = ?",
        )
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, title, content, original_filename, file_path, mime_type,
                    correspondent_id, document_type_id, page_count, source_type, source_meta, ocr_status, embed_status,
                     created_at, updated_at
             FROM documents WHERE 1 = 1",
        );

        if let Some(filter) = filter {
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

        qb.push(" ORDER BY created_at DESC");

        let rows: Vec<DocumentRow> = qb
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, params: CreateDocument) -> Result<Document> {
        let id = Id::new_v4();
        let now = Utc::now();
        let ocr_status = if params.content.is_some() {
            ProcessingStatus::Done
        } else {
            ProcessingStatus::Pending
        };
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        sqlx::query(
            "INSERT INTO documents (id, title, content, original_filename, mime_type,
                                    correspondent_id, document_type_id, ocr_status,
                                    source_type, source_meta, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
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
            created_at: now,
            updated_at: now,
        })
    }

    async fn update(&self, id: Id, params: UpdateDocument) -> Result<Document> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let row: Option<DocumentRow> = sqlx::query_as(
            "SELECT id, title, content, original_filename, file_path, mime_type,
                    correspondent_id, document_type_id, page_count, source_type, source_meta,
                    ocr_status, embed_status, created_at, updated_at FROM documents WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let existing: Document =
            row.map(TryInto::try_into)
                .transpose()?
                .ok_or_else(|| Error::NotFound {
                    entity_type: "Document",
                    id: id.to_string(),
                })?;

        let title = params.title.unwrap_or_else(|| existing.title.clone());
        let content = params.content.or_else(|| existing.content.clone());
        let correspondent_id = params.correspondent_id.unwrap_or(existing.correspondent_id);
        let document_type_id = params.document_type_id.unwrap_or(existing.document_type_id);
        let now = Utc::now();

        let tags_changed = if let Some(tags) = params.tag_ids {
            self.replace_tags(&mut tx, id, &tags).await?
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

        sqlx::query(
            "UPDATE documents SET title = ?, content = ?, correspondent_id = ?,
                                  document_type_id = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&content)
        .bind(correspondent_id.map(|id| id.to_string()))
        .bind(document_type_id.map(|id| id.to_string()))
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        self.emit_in(&mut tx, DocumentEvent::Updated { document_id: id })
            .await?;
        self.commit(tx).await?;

        Ok(Document {
            id,
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
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    async fn delete(&self, id: Id) -> Result<()> {
        // Workflows still need to see the document after it is gone.
        let snapshot = match self.find_by_id(id).await? {
            Some(document) => Some(Arc::new(DeletedDocument {
                tags: self.get_tags(id).await?,
                document,
            })),
            None => None,
        };
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        let result = sqlx::query("DELETE FROM documents WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            });
        }
        self.emit_in(
            &mut tx,
            DocumentEvent::Deleted {
                document_id: id,
                snapshot,
            },
        )
        .await?;
        self.commit(tx).await
    }

    async fn set_file_path(&self, id: Id, file_path: &str) -> Result<()> {
        let now = Utc::now();
        let result = sqlx::query("UPDATE documents SET file_path = ?, updated_at = ? WHERE id = ?")
            .bind(file_path)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            });
        }

        Ok(())
    }

    async fn set_page_count(&self, id: Id, page_count: u32) -> Result<()> {
        let now = Utc::now();
        let result =
            sqlx::query("UPDATE documents SET page_count = ?, updated_at = ? WHERE id = ?")
                .bind(page_count as i64)
                .bind(now.to_rfc3339())
                .bind(id.to_string())
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            });
        }

        Ok(())
    }

    async fn set_stage_status(
        &self,
        id: Id,
        stage: ProcessingStage,
        status: ProcessingStatus,
    ) -> Result<()> {
        let sql = match stage {
            ProcessingStage::Ocr => "UPDATE documents SET ocr_status = ? WHERE id = ?",
            ProcessingStage::Embed => "UPDATE documents SET embed_status = ? WHERE id = ?",
        };
        sqlx::query(sql)
            .bind(status.as_str())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn complete_ocr(&self, id: Id, content: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE documents SET content = ?, ocr_status = 'done', updated_at = ? WHERE id = ?",
        )
        .bind(content)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>> {
        let sql = match stage {
            ProcessingStage::Ocr => {
                "SELECT id, title, content, original_filename, file_path, mime_type,
                        correspondent_id, document_type_id, page_count, source_type, source_meta, ocr_status, embed_status,
                        created_at, updated_at
                 FROM documents WHERE ocr_status = 'pending' ORDER BY created_at"
            }
            ProcessingStage::Embed => {
                "SELECT id, title, content, original_filename, file_path, mime_type,
                        correspondent_id, document_type_id, page_count, source_type, source_meta, ocr_status, embed_status,
                        created_at, updated_at
                 FROM documents WHERE embed_status = 'pending' ORDER BY created_at"
            }
        };
        let rows: Vec<DocumentRow> = sqlx::query_as(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn reset_embedding_status(&self) -> Result<Vec<Id>> {
        let ids: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM documents WHERE content IS NOT NULL AND content != ''")
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        sqlx::query(
            "UPDATE documents SET embed_status = 'pending'
             WHERE content IS NOT NULL AND content != ''",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        ids.into_iter().map(|(id,)| parse_uuid(&id)).collect()
    }

    async fn get_tags(&self, document_id: Id) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at
             FROM tags t
             INNER JOIN document_tags dt ON t.id = dt.tag_id
             WHERE dt.document_id = ?",
        )
        .bind(document_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn set_tags(&self, document_id: Id, tag_ids: &[Id]) -> Result<()> {
        self.update(
            document_id,
            UpdateDocument {
                tag_ids: Some(tag_ids.to_vec()),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
    }

    async fn add_tag(&self, document_id: Id, tag_id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        let result =
            sqlx::query("INSERT OR IGNORE INTO document_tags (document_id, tag_id) VALUES (?, ?)")
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

    async fn remove_tag(&self, document_id: Id, tag_id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
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
    async fn find_by_id(&self, id: Id) -> Result<Option<Tag>> {
        let row: Option<TagRow> =
            sqlx::query_as("SELECT id, name, color, created_at FROM tags WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Tag>> {
        let row: Option<TagRow> =
            sqlx::query_as("SELECT id, name, color, created_at FROM tags WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> =
            sqlx::query_as("SELECT id, name, color, created_at FROM tags ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, params: CreateTag) -> Result<Tag> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query("INSERT INTO tags (id, name, color, created_at) VALUES (?, ?, ?, ?)")
            .bind(id.to_string())
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

    async fn update(&self, id: Id, params: UpdateTag) -> Result<Tag> {
        let existing = self.find_by_id(id).await?.ok_or_else(|| Error::NotFound {
            entity_type: "Tag",
            id: id.to_string(),
        })?;

        let name = params.name.unwrap_or(existing.name);
        let color = params.color.unwrap_or(existing.color);

        sqlx::query("UPDATE tags SET name = ?, color = ? WHERE id = ?")
            .bind(&name)
            .bind(&color)
            .bind(id.to_string())
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

    async fn delete(&self, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM tags WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Tag",
                id: id.to_string(),
            });
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
    async fn find_by_id(&self, id: Id) -> Result<Option<Correspondent>> {
        let row: Option<CorrespondentRow> =
            sqlx::query_as("SELECT id, name, created_at FROM correspondents WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Correspondent>> {
        let row: Option<CorrespondentRow> =
            sqlx::query_as("SELECT id, name, created_at FROM correspondents WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self) -> Result<Vec<Correspondent>> {
        let rows: Vec<CorrespondentRow> =
            sqlx::query_as("SELECT id, name, created_at FROM correspondents ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, params: CreateCorrespondent) -> Result<Correspondent> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query("INSERT INTO correspondents (id, name, created_at) VALUES (?, ?, ?)")
            .bind(id.to_string())
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

    async fn update(&self, id: Id, params: UpdateCorrespondent) -> Result<Correspondent> {
        let existing = self.find_by_id(id).await?.ok_or_else(|| Error::NotFound {
            entity_type: "Correspondent",
            id: id.to_string(),
        })?;

        let name = params.name.unwrap_or(existing.name);

        sqlx::query("UPDATE correspondents SET name = ? WHERE id = ?")
            .bind(&name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "Correspondent", &name))?;

        Ok(Correspondent {
            id,
            name,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM correspondents WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Correspondent",
                id: id.to_string(),
            });
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
    async fn find_by_id(&self, id: Id) -> Result<Option<DocumentType>> {
        let row: Option<DocumentTypeRow> =
            sqlx::query_as("SELECT id, name, created_at FROM document_types WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<DocumentType>> {
        let row: Option<DocumentTypeRow> =
            sqlx::query_as("SELECT id, name, created_at FROM document_types WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list(&self) -> Result<Vec<DocumentType>> {
        let rows: Vec<DocumentTypeRow> =
            sqlx::query_as("SELECT id, name, created_at FROM document_types ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx_error)?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn create(&self, params: CreateDocumentType) -> Result<DocumentType> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query("INSERT INTO document_types (id, name, created_at) VALUES (?, ?, ?)")
            .bind(id.to_string())
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

    async fn update(&self, id: Id, params: UpdateDocumentType) -> Result<DocumentType> {
        let existing = self.find_by_id(id).await?.ok_or_else(|| Error::NotFound {
            entity_type: "DocumentType",
            id: id.to_string(),
        })?;

        let name = params.name.unwrap_or(existing.name);

        sqlx::query("UPDATE document_types SET name = ? WHERE id = ?")
            .bind(&name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| map_write_error(e, "DocumentType", &name))?;

        Ok(DocumentType {
            id,
            name,
            created_at: existing.created_at,
        })
    }

    async fn delete(&self, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM document_types WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "DocumentType",
                id: id.to_string(),
            });
        }

        Ok(())
    }
}

impl SqliteDocumentRepository {
    async fn replace_tags(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        document_id: Id,
        tag_ids: &[Id],
    ) -> Result<bool> {
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

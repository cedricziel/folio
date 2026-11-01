//! PostgreSQL document, tag, correspondent and document type repositories

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentType, Id, Tag, UpdateCorrespondent, UpdateDocument, UpdateDocumentType,
    UpdateTag,
};
use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, TagRepository,
};

use super::map_write_error;

fn repo_err(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

fn not_found(entity_type: &'static str, id: Id) -> Error {
    Error::NotFound {
        entity_type,
        id: id.to_string(),
    }
}

/// `SELECT` over `documents` with the full column list; sqlx requires static SQL.
macro_rules! select_documents {
    ($tail:literal) => {
        concat!(
            "SELECT id, title, content, original_filename, file_path, mime_type, ",
            "correspondent_id, document_type_id, page_count, created_at, updated_at ",
            "FROM documents ",
            $tail
        )
    };
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: Uuid,
    title: String,
    content: Option<String>,
    original_filename: Option<String>,
    file_path: Option<String>,
    mime_type: Option<String>,
    correspondent_id: Option<Uuid>,
    document_type_id: Option<Uuid>,
    page_count: Option<i32>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<DocumentRow> for Document {
    fn from(row: DocumentRow) -> Self {
        Document {
            id: row.id,
            title: row.title,
            content: row.content,
            original_filename: row.original_filename,
            file_path: row.file_path,
            mime_type: row.mime_type,
            correspondent_id: row.correspondent_id,
            document_type_id: row.document_type_id,
            page_count: row.page_count.map(|c| c as u32),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct TagRow {
    id: Uuid,
    name: String,
    color: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<TagRow> for Tag {
    fn from(row: TagRow) -> Self {
        Tag {
            id: row.id,
            name: row.name,
            color: row.color,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct NamedRow {
    id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
}

impl From<NamedRow> for Correspondent {
    fn from(row: NamedRow) -> Self {
        Correspondent {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
        }
    }
}

impl From<NamedRow> for DocumentType {
    fn from(row: NamedRow) -> Self {
        DocumentType {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
        }
    }
}

/// PostgreSQL implementation of DocumentRepository
pub struct PostgresDocumentRepository {
    pool: PgPool,
}

impl PostgresDocumentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn touch_one(
        &self,
        id: Id,
        query: sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments>,
    ) -> Result<()> {
        let result = query.execute(&self.pool).await.map_err(repo_err)?;
        if result.rows_affected() == 0 {
            return Err(not_found("Document", id));
        }
        Ok(())
    }
}

#[async_trait]
impl DocumentRepository for PostgresDocumentRepository {
    async fn find_by_id(&self, id: Id) -> Result<Option<Document>> {
        let row: Option<DocumentRow> = sqlx::query_as(select_documents!("WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;

        Ok(row.map(Into::into))
    }

    async fn list(&self, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        let filter = filter.unwrap_or_default();
        let rows: Vec<DocumentRow> = if let Some(correspondent_id) = filter.correspondent_id {
            sqlx::query_as(select_documents!(
                "WHERE correspondent_id = $1 ORDER BY created_at DESC"
            ))
            .bind(correspondent_id)
            .fetch_all(&self.pool)
            .await
        } else if let Some(document_type_id) = filter.document_type_id {
            sqlx::query_as(select_documents!(
                "WHERE document_type_id = $1 ORDER BY created_at DESC"
            ))
            .bind(document_type_id)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as(select_documents!("ORDER BY created_at DESC"))
                .fetch_all(&self.pool)
                .await
        }
        .map_err(repo_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, params: CreateDocument) -> Result<Document> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO documents (id, title, content, original_filename, mime_type,
                                    correspondent_id, document_type_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(id)
        .bind(&params.title)
        .bind(&params.content)
        .bind(&params.original_filename)
        .bind(&params.mime_type)
        .bind(params.correspondent_id)
        .bind(params.document_type_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;

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
            created_at: now,
            updated_at: now,
        })
    }

    async fn update(&self, id: Id, params: UpdateDocument) -> Result<Document> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| not_found("Document", id))?;

        let title = params.title.unwrap_or(existing.title);
        let content = params.content.or(existing.content);
        let correspondent_id = params.correspondent_id.unwrap_or(existing.correspondent_id);
        let document_type_id = params.document_type_id.unwrap_or(existing.document_type_id);
        let now = Utc::now();

        sqlx::query(
            "UPDATE documents SET title = $1, content = $2, correspondent_id = $3,
                                  document_type_id = $4, updated_at = $5
             WHERE id = $6",
        )
        .bind(&title)
        .bind(&content)
        .bind(correspondent_id)
        .bind(document_type_id)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;

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
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    async fn delete(&self, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM documents WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        if result.rows_affected() == 0 {
            return Err(not_found("Document", id));
        }
        Ok(())
    }

    async fn set_file_path(&self, id: Id, file_path: &str) -> Result<()> {
        self.touch_one(
            id,
            sqlx::query("UPDATE documents SET file_path = $1, updated_at = $2 WHERE id = $3")
                .bind(file_path.to_string())
                .bind(Utc::now())
                .bind(id),
        )
        .await
    }

    async fn set_page_count(&self, id: Id, page_count: u32) -> Result<()> {
        self.touch_one(
            id,
            sqlx::query("UPDATE documents SET page_count = $1, updated_at = $2 WHERE id = $3")
                .bind(page_count as i32)
                .bind(Utc::now())
                .bind(id),
        )
        .await
    }

    async fn get_tags(&self, document_id: Id) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at
             FROM tags t
             INNER JOIN document_tags dt ON t.id = dt.tag_id
             WHERE dt.document_id = $1",
        )
        .bind(document_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn set_tags(&self, document_id: Id, tag_ids: &[Id]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;

        sqlx::query("DELETE FROM document_tags WHERE document_id = $1")
            .bind(document_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;

        for tag_id in tag_ids {
            sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2)")
                .bind(document_id)
                .bind(tag_id)
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
        }

        tx.commit().await.map_err(repo_err)
    }

    async fn add_tag(&self, document_id: Id, tag_id: Id) -> Result<()> {
        sqlx::query(
            "INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(document_id)
        .bind(tag_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn remove_tag(&self, document_id: Id, tag_id: Id) -> Result<()> {
        sqlx::query("DELETE FROM document_tags WHERE document_id = $1 AND tag_id = $2")
            .bind(document_id)
            .bind(tag_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
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
    async fn find_by_id(&self, id: Id) -> Result<Option<Tag>> {
        let row: Option<TagRow> =
            sqlx::query_as("SELECT id, name, color, created_at FROM tags WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Tag>> {
        let row: Option<TagRow> =
            sqlx::query_as("SELECT id, name, color, created_at FROM tags WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self) -> Result<Vec<Tag>> {
        let rows: Vec<TagRow> =
            sqlx::query_as("SELECT id, name, color, created_at FROM tags ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, params: CreateTag) -> Result<Tag> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query("INSERT INTO tags (id, name, color, created_at) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(&params.name)
            .bind(&params.color)
            .bind(now)
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
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| not_found("Tag", id))?;

        let name = params.name.unwrap_or(existing.name);
        let color = params.color.unwrap_or(existing.color);

        sqlx::query("UPDATE tags SET name = $1, color = $2 WHERE id = $3")
            .bind(&name)
            .bind(&color)
            .bind(id)
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
        let result = sqlx::query("DELETE FROM tags WHERE id = $1")
            .bind(id)
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
    async fn find_by_id(&self, id: Id) -> Result<Option<Correspondent>> {
        let row: Option<NamedRow> =
            sqlx::query_as("SELECT id, name, created_at FROM correspondents WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Correspondent>> {
        let row: Option<NamedRow> =
            sqlx::query_as("SELECT id, name, created_at FROM correspondents WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self) -> Result<Vec<Correspondent>> {
        let rows: Vec<NamedRow> =
            sqlx::query_as("SELECT id, name, created_at FROM correspondents ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, params: CreateCorrespondent) -> Result<Correspondent> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query("INSERT INTO correspondents (id, name, created_at) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(&params.name)
            .bind(now)
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
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| not_found("Correspondent", id))?;

        let name = params.name.unwrap_or(existing.name);

        sqlx::query("UPDATE correspondents SET name = $1 WHERE id = $2")
            .bind(&name)
            .bind(id)
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
        let result = sqlx::query("DELETE FROM correspondents WHERE id = $1")
            .bind(id)
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
    async fn find_by_id(&self, id: Id) -> Result<Option<DocumentType>> {
        let row: Option<NamedRow> =
            sqlx::query_as("SELECT id, name, created_at FROM document_types WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<DocumentType>> {
        let row: Option<NamedRow> =
            sqlx::query_as("SELECT id, name, created_at FROM document_types WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn list(&self) -> Result<Vec<DocumentType>> {
        let rows: Vec<NamedRow> =
            sqlx::query_as("SELECT id, name, created_at FROM document_types ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create(&self, params: CreateDocumentType) -> Result<DocumentType> {
        let id = Id::new_v4();
        let now = Utc::now();

        sqlx::query("INSERT INTO document_types (id, name, created_at) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(&params.name)
            .bind(now)
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
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| not_found("DocumentType", id))?;

        let name = params.name.unwrap_or(existing.name);

        sqlx::query("UPDATE document_types SET name = $1 WHERE id = $2")
            .bind(&name)
            .bind(id)
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
        let result = sqlx::query("DELETE FROM document_types WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        if result.rows_affected() == 0 {
            return Err(not_found("DocumentType", id));
        }
        Ok(())
    }
}

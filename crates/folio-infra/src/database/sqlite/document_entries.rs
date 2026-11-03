use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::error::{Error, Result};
use folio_core::models::{
    DocumentEntry, EntryBody, EntryKind, EntryPage, EntryPageQuery, Id, UserId, VaultId,
};
use folio_core::mutation::current_actor;
use folio_core::ports::{DocumentEntryRepository, EntryCreation};
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct SqliteDocumentEntryRepository {
    pool: SqlitePool,
    wake: Arc<Notify>,
}

impl SqliteDocumentEntryRepository {
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

fn error(value: impl std::fmt::Display) -> Error {
    Error::Repository(value.to_string())
}
fn missing(id: Id) -> Error {
    Error::NotFound {
        entity_type: "document entry",
        id: id.to_string(),
    }
}

#[derive(sqlx::FromRow)]
struct EntryRow {
    id: String,
    document_id: String,
    vault_id: String,
    kind: String,
    body: String,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl EntryRow {
    fn decode(self) -> Result<DocumentEntry> {
        let time = |value: String| {
            DateTime::parse_from_rfc3339(&value)
                .map(|time| time.with_timezone(&Utc))
                .map_err(error)
        };
        Ok(DocumentEntry {
            id: self.id.parse().map_err(error)?,
            document_id: self.document_id.parse().map_err(error)?,
            vault_id: self.vault_id.parse().map_err(error)?,
            kind: match self.kind.as_str() {
                "note" => EntryKind::Note,
                "comment" => EntryKind::Comment,
                _ => return Err(error("Invalid entry kind")),
            },
            body: self.body,
            created_at: time(self.created_at)?,
            updated_at: time(self.updated_at)?,
            deleted_at: self.deleted_at.map(time).transpose()?,
            created_by: self.created_by.map(UserId),
            updated_by: self.updated_by.map(UserId),
        })
    }
}

async fn parent(conn: &mut SqliteConnection, vault: VaultId, document: Id) -> Result<()> {
    let found: Option<String> = sqlx::query_scalar("SELECT id FROM documents WHERE vault_id = ? AND id = ? AND deleted_at IS NULL AND purge_requested_at IS NULL")
        .bind(vault.to_string()).bind(document.to_string()).fetch_optional(conn).await.map_err(error)?;
    found
        .ok_or_else(|| Error::NotFound {
            entity_type: "document",
            id: document.to_string(),
        })
        .map(|_| ())
}

async fn entry(
    conn: &mut SqliteConnection,
    vault: VaultId,
    document: Id,
    id: Id,
) -> Result<Option<DocumentEntry>> {
    sqlx::query_as::<_, EntryRow>(
        "SELECT * FROM document_entries WHERE vault_id = ? AND document_id = ? AND id = ?",
    )
    .bind(vault.to_string())
    .bind(document.to_string())
    .bind(id.to_string())
    .fetch_optional(conn)
    .await
    .map_err(error)?
    .map(EntryRow::decode)
    .transpose()
}

async fn publish(
    conn: &mut SqliteConnection,
    entry: &DocumentEntry,
    transition: &str,
) -> Result<()> {
    let kind = entry.kind.as_str();
    let envelope = DomainEventEnvelope::new(
        format!("document_{kind}.{transition}"),
        entry.vault_id,
        DomainEventSubject {
            subject_type: format!("document_{kind}"),
            id: entry.id,
        },
        Some(entry.document_id),
        EventAudience::Vault,
        serde_json::json!({"entry_kind": kind, "transition": transition, "entry_id": entry.id}),
    )?;
    super::insert_domain_event(conn, &envelope).await?;
    Ok(())
}

#[async_trait]
impl DocumentEntryRepository for SqliteDocumentEntryRepository {
    async fn find(&self, vault: VaultId, document: Id, id: Id) -> Result<Option<DocumentEntry>> {
        let mut conn = self.pool.acquire().await.map_err(error)?;
        parent(&mut conn, vault, document).await?;
        Ok(entry(&mut conn, vault, document, id)
            .await?
            .filter(|entry| entry.deleted_at.is_none()))
    }

    async fn page(&self, vault: VaultId, document: Id, query: EntryPageQuery) -> Result<EntryPage> {
        query.validate(vault, document)?;
        let mut conn = self.pool.acquire().await.map_err(error)?;
        parent(&mut conn, vault, document).await?;
        let entries = sqlx::query_as::<_, EntryRow>("SELECT e.* FROM document_entries e JOIN documents d ON d.id = e.document_id AND d.vault_id = e.vault_id WHERE e.vault_id = ?1 AND e.document_id = ?2 AND e.kind = ?3 AND e.deleted_at IS NULL AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL AND (?4 IS NULL OR (e.created_at, e.id) > (?4, ?5)) ORDER BY e.created_at, e.id LIMIT ?6")
            .bind(vault.to_string()).bind(document.to_string()).bind(query.kind.as_str())
            .bind(query.cursor.as_ref().map(|cursor| cursor.created_at.to_rfc3339()))
            .bind(query.cursor.as_ref().map(|cursor| cursor.id.to_string())).bind(i64::from(query.limit) + 1)
            .fetch_all(&mut *conn).await.map_err(error)?.into_iter().map(EntryRow::decode).collect::<Result<Vec<_>>>()?;
        Ok(query.finish(vault, document, entries))
    }

    async fn create(
        &self,
        vault: VaultId,
        document: Id,
        kind: EntryKind,
        body: EntryBody,
        key: Id,
    ) -> Result<EntryCreation> {
        if key.is_nil() {
            return Err(Error::Validation("Idempotency key must not be nil".into()));
        }
        let actor = current_actor();
        let actor_scope = actor
            .as_ref()
            .map(|actor| format!("user:{}", actor.as_str()))
            .unwrap_or_else(|| "system".into());
        let fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(&(kind, body.as_str())).map_err(error)?,
        ));
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(error)?;
        parent(&mut tx, vault, document).await?;
        let receipt: Option<(String, String, String)> = sqlx::query_as("SELECT request_fingerprint, entry_id, original_response FROM document_entry_creation_receipts WHERE vault_id = ? AND document_id = ? AND actor_scope = ? AND idempotency_key = ?")
            .bind(vault.to_string()).bind(document.to_string()).bind(&actor_scope).bind(key.to_string()).fetch_optional(&mut *tx).await.map_err(error)?;
        if let Some((previous, id, response)) = receipt {
            if previous != fingerprint {
                return Err(Error::Conflict(
                    "Idempotency key already belongs to different entry content".into(),
                ));
            }
            let current = entry(&mut tx, vault, document, id.parse().map_err(error)?)
                .await?
                .ok_or_else(|| error("Creation receipt has no entry"))?;
            if current.deleted_at.is_some() {
                tx.commit().await.map_err(error)?;
                return Ok(EntryCreation::Deleted);
            }
            let mut original: DocumentEntry = serde_json::from_str(&response).map_err(error)?;
            original.vault_id = vault;
            tx.commit().await.map_err(error)?;
            return Ok(EntryCreation::Replayed(original));
        }
        let now = Utc::now().trunc_subsecs(6);
        let created = DocumentEntry {
            id: Id::new_v4(),
            document_id: document,
            vault_id: vault,
            kind,
            body: body.as_str().into(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: actor.clone(),
            updated_by: actor.clone(),
        };
        sqlx::query("INSERT INTO document_entries (id, document_id, vault_id, kind, body, created_at, updated_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(created.id.to_string()).bind(document.to_string()).bind(vault.to_string()).bind(kind.as_str()).bind(&created.body)
            .bind(now.to_rfc3339()).bind(now.to_rfc3339()).bind(actor.as_ref().map(UserId::as_str)).bind(actor.as_ref().map(UserId::as_str))
            .execute(&mut *tx).await.map_err(error)?;
        sqlx::query("INSERT INTO document_entry_creation_receipts (vault_id, document_id, actor_scope, idempotency_key, request_fingerprint, entry_id, original_response, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(vault.to_string()).bind(document.to_string()).bind(actor_scope).bind(key.to_string()).bind(fingerprint).bind(created.id.to_string())
            .bind(serde_json::to_string(&created).map_err(error)?).bind(now.to_rfc3339()).execute(&mut *tx).await.map_err(error)?;
        publish(&mut tx, &created, "created").await?;
        tx.commit().await.map_err(error)?;
        self.wake.notify_one();
        Ok(EntryCreation::Created(created))
    }

    async fn update(
        &self,
        vault: VaultId,
        document: Id,
        id: Id,
        body: EntryBody,
    ) -> Result<DocumentEntry> {
        let actor = current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(error)?;
        parent(&mut tx, vault, document).await?;
        let mut current = entry(&mut tx, vault, document, id)
            .await?
            .ok_or_else(|| missing(id))?;
        if current.deleted_at.is_some() || actor.is_none() || actor != current.created_by {
            return Err(missing(id));
        }
        if current.body == body.as_str() {
            tx.commit().await.map_err(error)?;
            return Ok(current);
        }
        current.body = body.as_str().into();
        current.updated_at = Utc::now().trunc_subsecs(6);
        current.updated_by = actor;
        sqlx::query("UPDATE document_entries SET body = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? AND document_id = ?")
            .bind(&current.body).bind(current.updated_at.to_rfc3339()).bind(current.updated_by.as_ref().map(UserId::as_str))
            .bind(id.to_string()).bind(vault.to_string()).bind(document.to_string()).execute(&mut *tx).await.map_err(error)?;
        publish(&mut tx, &current, "updated").await?;
        tx.commit().await.map_err(error)?;
        self.wake.notify_one();
        Ok(current)
    }

    async fn delete(
        &self,
        vault: VaultId,
        document: Id,
        id: Id,
        allow_moderation: bool,
    ) -> Result<()> {
        let actor = current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(error)?;
        parent(&mut tx, vault, document).await?;
        let current = entry(&mut tx, vault, document, id)
            .await?
            .ok_or_else(|| missing(id))?;
        if !allow_moderation && (actor.is_none() || actor != current.created_by) {
            return Err(missing(id));
        }
        if current.deleted_at.is_some() {
            tx.commit().await.map_err(error)?;
            return Ok(());
        }
        let now = Utc::now().trunc_subsecs(6).to_rfc3339();
        sqlx::query("UPDATE document_entries SET deleted_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? AND document_id = ?")
            .bind(&now).bind(&now).bind(actor.as_ref().map(UserId::as_str)).bind(id.to_string()).bind(vault.to_string()).bind(document.to_string()).execute(&mut *tx).await.map_err(error)?;
        publish(&mut tx, &current, "deleted").await?;
        tx.commit().await.map_err(error)?;
        self.wake.notify_one();
        Ok(())
    }
}

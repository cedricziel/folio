//! PostgreSQL outbox
//!
//! Rows are claimed with `FOR UPDATE SKIP LOCKED` and a lease, so several
//! dispatchers (or Folio instances) never hand out the same event twice, and
//! a crashed one does not hold rows forever.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::Notify;
use tracing::warn;

use folio_core::domain_event::DomainEventEnvelope;
use folio_core::error::{Error, Result};
use folio_core::events::{decode_stored_outbox_payload, encode_stored_outbox_payload};
use folio_core::models::{DocumentSource, Id, VaultId};
use folio_core::ports::OutboxStore;
use folio_core::{DocumentEvent, EventOrigin, EventSnapshot, OutboxEvent};
use sqlx::Row as _;

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

pub(crate) async fn capture_document(
    conn: &mut PgConnection,
    id: Id,
    expected_vault: Option<VaultId>,
) -> Result<Option<EventSnapshot>> {
    let row = sqlx::query("SELECT d.id AS document_id, d.document_date::text AS document_date, d.vault_id, d.title, d.original_filename, d.mime_type,
        d.file_path, d.source_type, d.source_meta, d.created_at, d.correspondent_id, c.name AS correspondent_name,
        d.document_type_id, k.name AS document_type_name, COALESCE((SELECT json_agg(json_build_object('id', t.id, 'name', t.name) ORDER BY t.id)::text FROM document_tags dt JOIN tags t ON t.id = dt.tag_id AND t.vault_id = d.vault_id WHERE dt.document_id = d.id), '[]') AS tags
        FROM documents d LEFT JOIN correspondents c ON c.id = d.correspondent_id AND c.vault_id = d.vault_id
        LEFT JOIN document_types k ON k.id = d.document_type_id AND k.vault_id = d.vault_id
        WHERE d.id = $1 AND ($2::uuid IS NULL OR (d.vault_id = $2 AND d.deleted_at IS NULL)) FOR SHARE OF d")
        .bind(id).bind(expected_vault.map(|v| v.0)).fetch_optional(conn).await.map_err(repo_err)?;
    row.map(|row| {
        let source_type: String = row.get("source_type");
        let source_meta: Option<String> = row.get("source_meta");
        Ok(EventSnapshot {
            document_date: row
                .get::<Option<String>, _>("document_date")
                .map(|value| value.parse::<folio_core::models::DocumentDate>())
                .transpose()
                .map_err(repo_err)?,
            document_id: row.get("document_id"),
            vault_id: VaultId(row.get("vault_id")),
            title: row.get("title"),
            original_filename: row.get("original_filename"),
            mime_type: row.get("mime_type"),
            file_path: row.get("file_path"),
            source: DocumentSource::from_stored(&source_type, source_meta.as_deref()),
            created_at: row.get("created_at"),
            correspondent_id: row.get("correspondent_id"),
            correspondent_name: row.get("correspondent_name"),
            document_type_id: row.get("document_type_id"),
            document_type_name: row.get("document_type_name"),
            tags: serde_json::from_str(&row.get::<String, _>("tags")).map_err(repo_err)?,
        })
    })
    .transpose()
}

/// Write a document event in the caller's transaction, capturing its immutable metadata.
pub(crate) async fn insert_event(
    conn: &mut PgConnection,
    event: &DocumentEvent,
    origin: EventOrigin,
    context: Option<&EventSnapshot>,
) -> Result<i64> {
    let captured;
    let context = match context {
        Some(context) => Some(context),
        None => {
            captured = match event {
                DocumentEvent::Deleted {
                    snapshot: Some(snapshot),
                    ..
                } => Some(EventSnapshot::from(snapshot.as_ref())),
                _ => capture_document(conn, event.document_id(), None).await?,
            };
            captured.as_ref()
        }
    };
    let envelope = folio_core::events::document_event_envelope(event, context)?;
    let payload = encode_stored_outbox_payload(Some(event), origin, context, envelope.as_ref())?;
    insert_payload(conn, event.event_type(), &payload, envelope.as_ref()).await
}

/// Persist a document projection and its validated immutable envelope in the caller's transaction.
pub async fn insert_event_with_envelope(
    conn: &mut PgConnection,
    event: &DocumentEvent,
    origin: EventOrigin,
    context: Option<&EventSnapshot>,
    envelope: &DomainEventEnvelope,
) -> Result<i64> {
    let payload = encode_stored_outbox_payload(Some(event), origin, context, Some(envelope))?;
    insert_payload(conn, event.event_type(), &payload, Some(envelope)).await
}

/// Persist a validated domain transition in the caller's transaction.
/// Reusing an event UUID is idempotent only when the immutable payload is identical.
pub async fn insert_domain_event(
    conn: &mut PgConnection,
    envelope: &DomainEventEnvelope,
) -> Result<i64> {
    insert_domain_event_with_origin(conn, envelope, EventOrigin::User).await
}

/// Persist a domain transition with an explicit workflow delivery origin.
pub async fn insert_domain_event_with_origin(
    conn: &mut PgConnection,
    envelope: &DomainEventEnvelope,
    origin: EventOrigin,
) -> Result<i64> {
    let payload = encode_stored_outbox_payload(None, origin, None, Some(envelope))?;
    insert_payload(conn, envelope.event_type(), &payload, Some(envelope)).await
}

async fn insert_payload(
    conn: &mut PgConnection,
    event_type: &str,
    payload: &str,
    envelope: Option<&DomainEventEnvelope>,
) -> Result<i64> {
    let now = envelope
        .map(DomainEventEnvelope::occurred_at)
        .unwrap_or_else(Utc::now);
    let event_id = envelope.map(DomainEventEnvelope::event_id);
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO outbox_events (type, payload, created_at, available_at, event_id)
         VALUES ($1, $2::jsonb, $3, $3, $4) ON CONFLICT(event_id) DO NOTHING RETURNING id",
    )
    .bind(event_type)
    .bind(payload)
    .bind(now)
    .bind(event_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(repo_err)?;
    if let Some(id) = id {
        return Ok(id);
    }
    let (id, stored): (i64, String) =
        sqlx::query_as("SELECT id, payload::text FROM outbox_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(conn)
            .await
            .map_err(repo_err)?;
    if serde_json::from_str::<serde_json::Value>(&stored).map_err(repo_err)?
        != serde_json::from_str::<serde_json::Value>(payload).map_err(repo_err)?
    {
        return Err(Error::Conflict(
            "Domain event UUID already belongs to a different payload".into(),
        ));
    }
    Ok(id)
}

type Row = (i64, String, DateTime<Utc>, i32);

fn decode(row: Row) -> std::result::Result<OutboxEvent, i64> {
    let (id, payload, created_at, attempts) = row;
    let decoded = decode_stored_outbox_payload(&payload).map_err(|e| {
        warn!("Outbox event {id} has an unreadable payload: {e}");
        id
    })?;
    Ok(OutboxEvent {
        id,
        event: decoded.event,
        envelope: decoded.envelope,
        origin: decoded.origin,
        context: decoded.context,
        created_at,
        attempts,
    })
}

#[derive(Clone)]
pub struct PostgresOutboxStore {
    pool: PgPool,
    wake: Arc<Notify>,
}

impl PostgresOutboxStore {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            wake: Arc::new(Notify::new()),
        }
    }

    /// Share a wake-up signal with the repositories writing to the outbox
    pub fn with_wake(mut self, wake: Arc<Notify>) -> Self {
        self.wake = wake;
        self
    }
}

#[async_trait]
impl OutboxStore for PostgresOutboxStore {
    async fn append(&self, event: &DocumentEvent, origin: EventOrigin) -> Result<i64> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let _: Option<Id> = sqlx::query_scalar("SELECT id FROM documents WHERE id = $1 FOR SHARE")
            .bind(event.document_id())
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?;
        let id = insert_event(&mut tx, event, origin, None).await?;
        tx.commit().await.map_err(repo_err)?;
        self.wake.notify_one();
        Ok(id)
    }

    async fn append_document(
        &self,
        vault: VaultId,
        event: &DocumentEvent,
        origin: EventOrigin,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        // A lock wait can refresh only the target row; capture on a fresh statement snapshot.
        let locked: Option<Id> = sqlx::query_scalar(
            "SELECT id FROM documents WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR SHARE",
        )
        .bind(event.document_id())
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
        if locked.is_none() {
            return Err(Error::Conflict(
                "Event document is no longer active in the expected vault".into(),
            ));
        }
        let context = capture_document(&mut tx, event.document_id(), Some(vault))
            .await?
            .ok_or_else(|| {
                Error::Conflict("Event document is no longer active in the expected vault".into())
            })?;
        let id = insert_event(&mut tx, event, origin, Some(&context)).await?;
        tx.commit().await.map_err(repo_err)?;
        self.wake.notify_one();
        Ok(id)
    }

    async fn claim(
        &self,
        limit: u32,
        lease: Duration,
        max_attempts: i32,
    ) -> Result<Vec<OutboxEvent>> {
        let now = Utc::now();
        let rows: Vec<Row> = sqlx::query_as(
            "UPDATE outbox_events
             SET attempts = attempts + 1, claimed_until = $1
             WHERE id IN (
                 SELECT id FROM outbox_events
                 WHERE dispatched_at IS NULL
                   AND available_at <= $2
                   AND (claimed_until IS NULL OR claimed_until <= $2)
                   AND attempts < $3
                 ORDER BY id
                 LIMIT $4
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING id, payload::text, created_at, attempts",
        )
        .bind(now + lease)
        .bind(now)
        .bind(max_attempts)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            match decode(row) {
                Ok(event) => events.push(event),
                Err(id) => self.drop_unreadable(id).await?,
            }
        }
        events.sort_by_key(|e| e.id);
        Ok(events)
    }

    async fn mark_dispatched(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE outbox_events SET dispatched_at = $1, claimed_until = NULL, last_error = NULL
             WHERE id = $2",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn release(&self, id: i64, error: &str, retry_at: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE outbox_events SET available_at = $1, claimed_until = NULL, last_error = $2
             WHERE id = $3",
        )
        .bind(retry_at)
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn replay(
        &self,
        after_id: i64,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<OutboxEvent>> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, payload::text, created_at, attempts FROM outbox_events
             WHERE created_at >= $2 AND id <> $1
               AND (dispatched_at IS NULL OR dispatched_at >=
                    COALESCE((SELECT created_at FROM outbox_events WHERE id = $1), $2))
             ORDER BY COALESCE(dispatched_at, created_at) DESC, id DESC LIMIT $3",
        )
        .bind(after_id)
        .bind(since)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        let mut events: Vec<_> = rows
            .into_iter()
            .filter_map(|row| decode(row).ok())
            .collect();
        events.sort_by_key(|event| event.id);
        Ok(events)
    }

    async fn prune(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM outbox_events WHERE dispatched_at IS NOT NULL AND dispatched_at < $1",
        )
        .bind(before)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected())
    }

    fn wake(&self) -> Arc<Notify> {
        self.wake.clone()
    }
}

impl PostgresOutboxStore {
    /// An event nobody can read would be claimed forever; retire it with a note
    async fn drop_unreadable(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE outbox_events SET dispatched_at = $1, claimed_until = NULL,
                                      last_error = 'unreadable payload'
             WHERE id = $2",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
}

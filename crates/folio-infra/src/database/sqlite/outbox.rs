//! SQLite outbox
//!
//! SQLite has a single writer, so the claiming `UPDATE` is atomic without row
//! locks; the lease keeps a crashed dispatcher from holding rows forever.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sqlx::{SqliteConnection, SqlitePool};
use tokio::sync::Notify;
use tracing::warn;

use folio_core::error::{Error, Result};
use folio_core::events::{decode_outbox_payload, encode_outbox_payload};
use folio_core::ports::OutboxStore;
use folio_core::{DocumentEvent, EventOrigin, OutboxEvent};

/// Fixed-width UTC timestamps, so text comparison orders them correctly
fn ts(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

/// Write an event inside the caller's transaction
pub(crate) async fn insert_event(
    conn: &mut SqliteConnection,
    event: &DocumentEvent,
    origin: EventOrigin,
) -> Result<i64> {
    let now = ts(Utc::now());
    let id = sqlx::query_scalar(
        "INSERT INTO outbox_events (type, payload, created_at, available_at)
         VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(event.event_type())
    .bind(encode_outbox_payload(event, origin))
    .bind(&now)
    .bind(&now)
    .fetch_one(conn)
    .await
    .map_err(repo_err)?;
    Ok(id)
}

type Row = (i64, String, String, i32);

fn decode(row: Row) -> std::result::Result<OutboxEvent, i64> {
    let (id, payload, created_at, attempts) = row;
    let (event, origin) = decode_outbox_payload(&payload).map_err(|e| {
        warn!("Outbox event {id} has an unreadable payload: {e}");
        id
    })?;
    Ok(OutboxEvent {
        id,
        event,
        origin,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|_| id)?,
        attempts,
    })
}

#[derive(Clone)]
pub struct SqliteOutboxStore {
    pool: SqlitePool,
    wake: Arc<Notify>,
}

impl SqliteOutboxStore {
    pub fn new(pool: SqlitePool) -> Self {
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
impl OutboxStore for SqliteOutboxStore {
    async fn append(&self, event: &DocumentEvent, origin: EventOrigin) -> Result<i64> {
        let mut conn = self.pool.acquire().await.map_err(repo_err)?;
        let id = insert_event(&mut conn, event, origin).await?;
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
             SET attempts = attempts + 1, claimed_until = ?
             WHERE id IN (
                 SELECT id FROM outbox_events
                 WHERE dispatched_at IS NULL
                   AND available_at <= ?
                   AND (claimed_until IS NULL OR claimed_until <= ?)
                   AND attempts < ?
                 ORDER BY id
                 LIMIT ?
             )
             RETURNING id, payload, created_at, attempts",
        )
        .bind(ts(now + lease))
        .bind(ts(now))
        .bind(ts(now))
        .bind(max_attempts)
        .bind(limit)
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
            "UPDATE outbox_events SET dispatched_at = ?, claimed_until = NULL, last_error = NULL
             WHERE id = ?",
        )
        .bind(ts(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn release(&self, id: i64, error: &str, retry_at: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE outbox_events SET available_at = ?, claimed_until = NULL, last_error = ?
             WHERE id = ?",
        )
        .bind(ts(retry_at))
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
            "SELECT id, payload, created_at, attempts FROM outbox_events
             WHERE id > ? AND dispatched_at IS NOT NULL AND created_at >= ?
             ORDER BY id LIMIT ?",
        )
        .bind(after_id)
        .bind(ts(since))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| decode(row).ok())
            .collect())
    }

    async fn prune(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM outbox_events WHERE dispatched_at IS NOT NULL AND dispatched_at < ?",
        )
        .bind(ts(before))
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected())
    }

    fn wake(&self) -> Arc<Notify> {
        self.wake.clone()
    }
}

impl SqliteOutboxStore {
    /// An event nobody can read would be claimed forever; retire it with a note
    async fn drop_unreadable(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE outbox_events SET dispatched_at = ?, claimed_until = NULL,
                                      last_error = 'unreadable payload'
             WHERE id = ?",
        )
        .bind(ts(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
}

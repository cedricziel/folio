//! Port for the transactional event outbox

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::Notify;

use crate::error::Result;
use crate::events::{DocumentEvent, EventOrigin, OutboxEvent};

/// Durable queue of domain events
///
/// Repositories write lifecycle events into it in the same transaction as the
/// change they describe; background work appends its own through [`append`].
/// A dispatcher then claims undelivered rows and hands them to consumers, so
/// no event is lost when the process stops between a change and its delivery.
///
/// [`append`]: OutboxStore::append
#[async_trait]
pub trait OutboxStore: Send + Sync {
    /// Persist an event on its own and wake the dispatcher
    async fn append(&self, event: &DocumentEvent, origin: EventOrigin) -> Result<i64>;

    /// Claim up to `limit` due, undispatched events for `lease`
    ///
    /// Each claim counts as an attempt. Claims by concurrent callers never
    /// overlap, and an abandoned claim becomes available again once the lease
    /// runs out. Events that already used `max_attempts` are left alone.
    async fn claim(
        &self,
        limit: u32,
        lease: chrono::Duration,
        max_attempts: i32,
    ) -> Result<Vec<OutboxEvent>>;

    async fn mark_dispatched(&self, id: i64) -> Result<()>;

    /// Give a claimed event back to be retried at `retry_at`
    async fn release(&self, id: i64, error: &str, retry_at: DateTime<Utc>) -> Result<()>;

    /// The newest recent events that may have been missed after `after_id`.
    ///
    /// Returns up to `limit` events in ID order, including pending events.
    /// Events created before `since` are excluded. Replay can overlap prior
    /// deliveries, so callers must tolerate repeats.
    async fn replay(
        &self,
        after_id: i64,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<OutboxEvent>>;

    /// Delete events dispatched before `before`; returns how many
    async fn prune(&self, before: DateTime<Utc>) -> Result<u64>;

    /// Signalled whenever a new event was committed
    fn wake(&self) -> Arc<Notify>;
}

//! Drains the event outbox into its consumers

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{self, StreamExt};
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::error::Result;
use crate::events::{EventBroadcaster, OutboxEvent};
use crate::ports::OutboxStore;

/// Something that reacts to a committed event
///
/// Consumers see an event at least once, so they must tolerate repeats.
#[async_trait]
pub trait EventConsumer: Send + Sync {
    async fn consume(&self, event: &OutboxEvent) -> Result<()>;
}

/// Pushes events to live SSE connections
pub struct BroadcastConsumer(pub EventBroadcaster);

#[async_trait]
impl EventConsumer for BroadcastConsumer {
    async fn consume(&self, event: &OutboxEvent) -> Result<()> {
        self.0.send(event.clone());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    /// Events claimed per round
    pub batch_size: u32,
    /// How many events of a batch are delivered at the same time
    pub concurrency: usize,
    /// How long a claim holds an event before another dispatcher may take it
    pub lease: chrono::Duration,
    /// Deliveries before an event is given up on and left in the table
    pub max_attempts: i32,
    /// Fallback poll interval when no wake-up arrives
    pub poll_interval: Duration,
    /// Dispatched events older than this are deleted
    pub retention: chrono::Duration,
    /// How often to prune
    pub prune_interval: Duration,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            batch_size: 50,
            concurrency: 8,
            lease: chrono::Duration::seconds(30),
            max_attempts: 10,
            poll_interval: Duration::from_millis(500),
            retention: chrono::Duration::days(7),
            prune_interval: Duration::from_secs(3600),
        }
    }
}

/// Delay before delivery attempt `attempts + 1`
pub fn retry_delay(attempts: i32) -> chrono::Duration {
    let seconds = 2_i64.saturating_pow(attempts.clamp(0, 16) as u32).min(300);
    chrono::Duration::seconds(seconds)
}

/// Claims outbox events and delivers each to every consumer, in order
///
/// An event is marked dispatched only after all consumers accepted it. A
/// failing event is retried with backoff and does not hold up the others.
pub struct OutboxDispatcher {
    store: Arc<dyn OutboxStore>,
    consumers: Vec<Arc<dyn EventConsumer>>,
    config: DispatcherConfig,
}

impl OutboxDispatcher {
    pub fn new(
        store: Arc<dyn OutboxStore>,
        consumers: Vec<Arc<dyn EventConsumer>>,
        config: DispatcherConfig,
    ) -> Self {
        Self {
            store,
            consumers,
            config,
        }
    }

    /// Claim and deliver one batch; returns how many events were claimed
    pub async fn dispatch_once(&self) -> Result<usize> {
        let batch = self
            .store
            .claim(
                self.config.batch_size,
                self.config.lease,
                self.config.max_attempts,
            )
            .await?;
        let claimed = batch.len();
        stream::iter(batch)
            .for_each_concurrent(self.config.concurrency, |event| async move {
                self.deliver(event).await;
            })
            .await;
        Ok(claimed)
    }

    async fn deliver(&self, event: OutboxEvent) {
        for consumer in &self.consumers {
            if let Err(e) = consumer.consume(&event).await {
                let retry_at = Utc::now() + retry_delay(event.attempts);
                warn!(
                    "Outbox event {} failed on attempt {}: {e}",
                    event.id, event.attempts
                );
                if event.attempts >= self.config.max_attempts {
                    error!("Outbox event {} used all its attempts", event.id);
                }
                if let Err(e) = self.store.release(event.id, &e.to_string(), retry_at).await {
                    error!("Failed to release outbox event {}: {e}", event.id);
                }
                return;
            }
        }
        if let Err(e) = self.store.mark_dispatched(event.id).await {
            error!("Failed to mark outbox event {} dispatched: {e}", event.id);
        } else {
            debug!("Dispatched outbox event {}", event.id);
        }
    }

    async fn prune(&self) {
        match self.store.prune(Utc::now() - self.config.retention).await {
            Ok(0) => {}
            Ok(n) => debug!("Pruned {n} dispatched outbox events"),
            Err(e) => error!("Failed to prune the outbox: {e}"),
        }
    }

    /// Dispatch until shutdown is signalled
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let wake = self.store.wake();
        let mut last_prune = tokio::time::Instant::now();
        self.prune().await;
        loop {
            if *shutdown.borrow() {
                break;
            }
            match self.dispatch_once().await {
                Ok(n) if n >= self.config.batch_size as usize => continue,
                Ok(_) => {}
                Err(e) => error!("Outbox dispatch failed: {e}"),
            }
            if last_prune.elapsed() >= self.config.prune_interval {
                last_prune = tokio::time::Instant::now();
                self.prune().await;
            }
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(self.config.poll_interval) => {}
                _ = shutdown.changed() => {}
            }
        }
    }
}

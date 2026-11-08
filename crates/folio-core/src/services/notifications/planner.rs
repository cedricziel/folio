//! Turns committed outbox events into pending notifications
//!
//! Network I/O never happens here: a slow push service must not hold up SSE
//! or workflows, which share the dispatcher.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::Notify;
use tracing::debug;

use crate::error::Result;
use crate::events::{DocumentEvent, EventOrigin, OutboxEvent};
use crate::models::{
    Access, Document, DocumentOutcome, Id, NotificationKind, NotificationScope,
    PendingNotification, PushDevice, UserId, VaultAction, VaultId,
};
use crate::ports::{AccessPolicy, DocumentLookup, PushDeviceRepository, PushTransport};
use crate::services::EventConsumer;

const FAILED_STAGES: [&str; 2] = [
    "document.processing.ocr.failed",
    "document.processing.embedding.failed",
];
const REVIEW_CHANGES: [&str; 2] = ["document.reviewed", "document.unreviewed"];

/// What an outbox event means for notifications
#[derive(Debug, PartialEq, Eq)]
struct Trigger {
    document: Id,
    vault: VaultId,
    outcome: Option<DocumentOutcome>,
    /// Imports record their outcome so a later reprocess is not a first, but tell nobody
    notify: bool,
}

fn trigger(event: &OutboxEvent) -> Option<Trigger> {
    let notify = event.origin != EventOrigin::Import;
    if let Some(document_event) = &event.event {
        let vault = event.origin_vault()?;
        let outcome = match document_event {
            DocumentEvent::Processed { .. } => Some(DocumentOutcome::Completed),
            DocumentEvent::Created { .. }
            | DocumentEvent::Trashed { .. }
            | DocumentEvent::Restored { .. }
            | DocumentEvent::Deleted { .. } => None,
            _ => return None,
        };
        return Some(Trigger {
            document: document_event.document_id(),
            vault,
            outcome,
            notify,
        });
    }
    let envelope = event.envelope.as_ref()?;
    let document = envelope.document_id()?;
    let event_type = envelope.event_type();
    let outcome = if FAILED_STAGES.contains(&event_type) {
        Some(DocumentOutcome::Failed)
    } else if REVIEW_CHANGES.contains(&event_type) {
        None
    } else {
        return None;
    };
    Some(Trigger {
        document,
        vault: envelope.vault_id(),
        outcome,
        notify,
    })
}

/// Records first outcomes and queues them for the devices that should hear about them
pub struct NotificationPlanner {
    devices: Arc<dyn PushDeviceRepository>,
    documents: Arc<dyn DocumentLookup>,
    access: Arc<dyn AccessPolicy>,
    transport: Arc<dyn PushTransport>,
    wake: Arc<Notify>,
}

impl NotificationPlanner {
    pub fn new(
        devices: Arc<dyn PushDeviceRepository>,
        documents: Arc<dyn DocumentLookup>,
        access: Arc<dyn AccessPolicy>,
        transport: Arc<dyn PushTransport>,
        wake: Arc<Notify>,
    ) -> Self {
        Self {
            devices,
            documents,
            access,
            transport,
            wake,
        }
    }

    async fn plan(&self, trigger: Trigger, devices: &[PushDevice]) -> Result<()> {
        let Some(outcome) = trigger.outcome else {
            return Ok(());
        };
        if !trigger.notify {
            self.devices
                .record_first_outcome(trigger.document, outcome, &[])
                .await?;
            return Ok(());
        }
        let document = self
            .documents
            .document(trigger.vault, trigger.document)
            .await?;
        let mut pending = Vec::new();
        if let Some(document) = &document {
            let kind = match outcome {
                DocumentOutcome::Failed => NotificationKind::Failed,
                DocumentOutcome::Completed if document.reviewed_at.is_some() => {
                    NotificationKind::Processed
                }
                DocumentOutcome::Completed => NotificationKind::NeedsReview,
            };
            let now = Utc::now();
            let mut editors = EditCache::new();
            for device in devices {
                if self
                    .should_hear(device, document, kind, &mut editors)
                    .await?
                {
                    pending.push(PendingNotification {
                        device_id: device.id,
                        document_id: document.id,
                        kind,
                        vault_id: document.vault_id,
                        created_at: now,
                    });
                }
            }
        }
        let first = self
            .devices
            .record_first_outcome(trigger.document, outcome, &pending)
            .await?;
        debug!(
            document = %trigger.document,
            outcome = outcome.as_str(),
            first,
            devices = pending.len(),
            "Planned notifications"
        );
        if first && !pending.is_empty() {
            self.wake.notify_one();
        }
        Ok(())
    }

    async fn should_hear(
        &self,
        device: &PushDevice,
        document: &Document,
        kind: NotificationKind,
        editors: &mut EditCache,
    ) -> Result<bool> {
        let preferences = &device.preferences;
        if !preferences.wants(kind)
            || !preferences.vaults.includes(document.vault_id)
            || device.alert_token.is_none()
            // Documents from before the device registered never notify it.
            || document.created_at < device.created_at
        {
            return Ok(false);
        }
        if preferences.scope == NotificationScope::Mine
            && document.uploaded_by.as_ref() != Some(&device.user_id)
        {
            return Ok(false);
        }
        may_edit(&*self.access, &device.user_id, document, editors).await
    }

    async fn mark_widgets(&self, vault: VaultId, devices: &[PushDevice]) -> Result<()> {
        let mut readers: HashMap<UserId, bool> = HashMap::new();
        let mut dirty = Vec::new();
        for device in devices {
            if device.widget_token.is_none() || !device.preferences.vaults.includes(vault) {
                continue;
            }
            let reads = match readers.get(&device.user_id) {
                Some(reads) => *reads,
                None => {
                    let reads = self
                        .access
                        .can(&device.user_id, vault, VaultAction::Read)
                        .await?;
                    let reads = matches!(reads, Access::Allowed(_));
                    readers.insert(device.user_id.clone(), reads);
                    reads
                }
            };
            if reads {
                dirty.push(device.id);
            }
        }
        if !dirty.is_empty() {
            self.devices.mark_widgets_dirty(&dirty, Utc::now()).await?;
            self.wake.notify_one();
        }
        Ok(())
    }
}

/// The answer to "may this user edit documents of this uploader in this vault"
pub(crate) type EditCache = HashMap<(UserId, VaultId, Option<UserId>), bool>;

/// Whether `user` may edit `document`, cached per user, vault and uploader for one batch
pub(crate) async fn may_edit(
    access: &dyn AccessPolicy,
    user: &UserId,
    document: &Document,
    cache: &mut EditCache,
) -> Result<bool> {
    let key = (
        user.clone(),
        document.vault_id,
        document.uploaded_by.clone(),
    );
    if let Some(allowed) = cache.get(&key) {
        return Ok(*allowed);
    }
    let allowed = access
        .can(
            user,
            document.vault_id,
            VaultAction::EditDocument {
                uploaded_by: document.uploaded_by.clone(),
            },
        )
        .await?;
    let allowed = matches!(allowed, Access::Allowed(_));
    cache.insert(key, allowed);
    Ok(allowed)
}

#[async_trait]
impl EventConsumer for NotificationPlanner {
    async fn consume(&self, event: &OutboxEvent) -> Result<()> {
        let Some(trigger) = trigger(event) else {
            return Ok(());
        };
        // While push is off, first outcomes are still recorded, so turning it on later
        // neither sends a backlog nor mistakes a reprocess for a first completion.
        let devices = if self.transport.available().await {
            self.devices.list_all().await?
        } else {
            Vec::new()
        };
        if !devices.is_empty() {
            self.mark_widgets(trigger.vault, &devices).await?;
        }
        self.plan(trigger, &devices).await
    }
}

#[cfg(test)]
mod tests;

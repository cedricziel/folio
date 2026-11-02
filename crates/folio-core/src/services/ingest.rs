//! Automatic ingestion: the import pipeline and the scheduler that feeds it

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::NewUpload;
use crate::error::{Error, Result};
use crate::models::{
    CreateTag, Id, IngestItem, IngestOutcome, IngestionSourceDef, LedgerEntry, PollReport,
    SourceStatus, VaultId, WorkflowAction, WorkflowTrigger,
};
use crate::ports::{
    keys, DocumentImporter, DocumentRepository, DocumentStore, IngestLedger, IngestSink,
    IngestionSourceFactory, SourceRepository, TagRepository, VaultIngest, VaultRepository,
    WorkflowRepository,
};

enum ClaimOutcome {
    Claimed(LedgerEntry),
    Duplicate,
    Busy,
}

/// Applies pre-ingest rules, de-duplicates by content, and hands files to the importer
pub struct IngestPipeline {
    documents: Arc<dyn DocumentRepository>,
    files: Arc<dyn DocumentStore>,
    tags: Arc<dyn TagRepository>,
    workflows: Arc<dyn WorkflowRepository>,
    ledger: Arc<dyn IngestLedger>,
    importer: Arc<dyn DocumentImporter>,
}

impl IngestPipeline {
    pub fn new(
        documents: Arc<dyn DocumentRepository>,
        files: Arc<dyn DocumentStore>,
        tags: Arc<dyn TagRepository>,
        workflows: Arc<dyn WorkflowRepository>,
        ledger: Arc<dyn IngestLedger>,
        importer: Arc<dyn DocumentImporter>,
    ) -> Self {
        Self {
            documents,
            files,
            tags,
            workflows,
            ledger,
            importer,
        }
    }

    /// Name of the first enabled `ingest.received` workflow that discards the item
    async fn discarding_rule(&self, vault: VaultId, item: &IngestItem) -> Result<Option<String>> {
        let rules = self
            .workflows
            .list_enabled(vault, WorkflowTrigger::IngestReceived)
            .await?;
        Ok(rules
            .into_iter()
            .filter(|w| w.actions.contains(&WorkflowAction::SkipImport))
            .find(|w| {
                w.conditions
                    .matches_candidate(&item.mime_type, &item.filename, &item.source)
            })
            .map(|w| w.name))
    }

    async fn claim(&self, vault: VaultId, key: &str, source_id: Id) -> Result<ClaimOutcome> {
        let entry = LedgerEntry {
            vault_id: vault,
            key: key.to_string(),
            source_id,
            document_id: Some(Id::new_v4()),
            outcome: "claimed".to_string(),
            created_at: claim_time(),
        };
        if self.ledger.insert(vault, &entry).await? {
            return Ok(ClaimOutcome::Claimed(entry));
        }
        let Some(existing) = self.ledger.find(vault, key).await? else {
            return Ok(if self.ledger.insert(vault, &entry).await? {
                ClaimOutcome::Claimed(entry)
            } else {
                ClaimOutcome::Busy
            });
        };
        let Some(id) = existing.document_id else {
            return Ok(ClaimOutcome::Busy);
        };
        if existing.outcome != "imported" && existing.outcome != "deleted" {
            let retryable = existing.outcome == "retryable"
                || (existing.outcome == "claimed"
                    && Utc::now() - existing.created_at >= chrono::Duration::minutes(10));
            if !retryable
                || self.documents.locate_trashed(id).await?.is_some()
                || self
                    .documents
                    .locate_retained(id)
                    .await?
                    .is_some_and(|retained| retained != vault)
            {
                return Ok(ClaimOutcome::Busy);
            }
            let mut renewed = existing.clone();
            renewed.outcome = "claimed".into();
            renewed.created_at =
                claim_time().max(existing.created_at + chrono::Duration::microseconds(1));
            return Ok(
                if self
                    .ledger
                    .transition_if_matches(vault, &existing, "claimed", renewed.created_at)
                    .await?
                {
                    ClaimOutcome::Claimed(renewed)
                } else {
                    ClaimOutcome::Busy
                },
            );
        }
        if self.documents.locate_retained(id).await? == Some(vault) {
            return Ok(if self.files.exists(&keys::original(id)).await? {
                ClaimOutcome::Duplicate
            } else {
                ClaimOutcome::Busy
            });
        }
        if !self.ledger.delete_if_matches(vault, &existing).await? {
            return Ok(ClaimOutcome::Busy);
        }
        Ok(if self.ledger.insert(vault, &entry).await? {
            ClaimOutcome::Claimed(entry)
        } else {
            ClaimOutcome::Busy
        })
    }

    async fn resolve_tags(&self, vault: VaultId, names: &[String]) -> Result<Vec<Id>> {
        let mut ids = Vec::new();
        for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
            let tag = match self.tags.find_by_name(vault, name).await? {
                Some(tag) => tag,
                None => match self
                    .tags
                    .create(
                        vault,
                        CreateTag {
                            name: name.to_string(),
                            color: None,
                        },
                    )
                    .await
                {
                    Ok(tag) => tag,
                    // Lost a race with another import creating the same tag.
                    Err(Error::Duplicate { .. }) => self
                        .tags
                        .find_by_name(vault, name)
                        .await?
                        .ok_or_else(|| Error::Repository(format!("Tag '{name}' vanished")))?,
                    Err(e) => return Err(e),
                },
            };
            if !ids.contains(&tag.id) {
                ids.push(tag.id);
            }
        }
        Ok(ids)
    }
}

// PostgreSQL persists timestamps at microsecond precision; claims must compare identically.
fn claim_time() -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp_micros(Utc::now().timestamp_micros()).unwrap()
}

/// Ledger key for a file's content
pub fn content_key(data: &[u8]) -> String {
    format!("hash:{}", hex::encode(Sha256::digest(data)))
}

#[async_trait]
impl VaultIngest for IngestPipeline {
    async fn ingest(
        &self,
        vault: VaultId,
        source_id: Id,
        item: IngestItem,
    ) -> Result<IngestOutcome> {
        if let Some(rule) = self.discarding_rule(vault, &item).await? {
            debug!("Rule '{rule}' discarded {}", item.filename);
            return Ok(IngestOutcome::Skipped { rule });
        }

        let key = content_key(&item.data);
        let claim = match self.claim(vault, &key, source_id).await? {
            ClaimOutcome::Claimed(claim) => claim,
            ClaimOutcome::Duplicate => return Ok(IngestOutcome::Duplicate),
            ClaimOutcome::Busy => return Ok(IngestOutcome::Busy),
        };

        let imported = async {
            let tag_ids = self.resolve_tags(vault, &item.tags).await?;
            self.importer
                .import_reserved(
                    vault,
                    &claim,
                    NewUpload {
                        title: Some(item.filename.clone()),
                        filename: item.filename,
                        mime_type: Some(item.mime_type),
                        data: item.data,
                        source: item.source,
                        tag_ids,
                        uploaded_by: None,
                    },
                )
                .await
        }
        .await;

        match imported {
            Ok(document) => {
                if !self
                    .ledger
                    .transition_if_matches(vault, &claim, "imported", claim.created_at)
                    .await?
                {
                    return Ok(IngestOutcome::Busy);
                }
                Ok(IngestOutcome::Imported(Box::new(document)))
            }
            Err(e) => {
                if self
                    .ledger
                    .transition_if_matches(vault, &claim, "retryable", claim.created_at)
                    .await?
                {
                    Err(e)
                } else {
                    Ok(IngestOutcome::Busy)
                }
            }
        }
    }
}

/// Runs every enabled source on its own schedule
///
/// Definitions come from configuration (read-only) and from the database
/// (managed in the UI). They are re-read periodically, so edits take effect
/// without a restart.
pub struct IngestScheduler {
    managed: Arc<dyn SourceRepository>,
    configured: Vec<IngestionSourceDef>,
    factory: Arc<dyn IngestionSourceFactory>,
    sink: Arc<dyn VaultIngest>,
    vaults: Arc<dyn VaultRepository>,
    reload_every: Duration,
}

/// Feeds one pipeline from sources that do not know about vaults
struct BoundSink {
    vault: VaultId,
    inner: Arc<dyn VaultIngest>,
}

#[async_trait]
impl IngestSink for BoundSink {
    async fn ingest(&self, source_id: Id, item: IngestItem) -> Result<IngestOutcome> {
        self.inner.ingest(self.vault, source_id, item).await
    }
}

impl IngestScheduler {
    pub fn new(
        managed: Arc<dyn SourceRepository>,
        configured: Vec<IngestionSourceDef>,
        factory: Arc<dyn IngestionSourceFactory>,
        sink: Arc<dyn VaultIngest>,
        vaults: Arc<dyn VaultRepository>,
    ) -> Self {
        Self {
            managed,
            configured,
            factory,
            sink,
            vaults,
            reload_every: Duration::from_secs(5),
        }
    }

    pub fn with_reload_interval(mut self, every: Duration) -> Self {
        self.reload_every = every;
        self
    }

    /// Supervise sources until shutdown is signalled
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let mut running: HashMap<Id, (String, JoinHandle<()>)> = HashMap::new();
        loop {
            match self.managed.list_all().await {
                Ok(managed) => {
                    let wanted: Vec<_> = self
                        .configured
                        .iter()
                        .cloned()
                        .chain(managed)
                        .filter(|d| d.enabled)
                        .collect();
                    self.reconcile(&mut running, wanted, &shutdown).await;
                }
                Err(e) => warn!("Could not load ingestion sources: {e}"),
            }
            tokio::select! {
                _ = tokio::time::sleep(self.reload_every) => {}
                _ = shutdown.changed() => if *shutdown.borrow() { break },
            }
        }
        for (_, (_, handle)) in running {
            handle.abort();
        }
    }

    async fn reconcile(
        &self,
        running: &mut HashMap<Id, (String, JoinHandle<()>)>,
        wanted: Vec<IngestionSourceDef>,
        shutdown: &watch::Receiver<bool>,
    ) {
        running.retain(|id, (_, handle)| {
            let keep = wanted.iter().any(|d| d.id == *id);
            if !keep {
                handle.abort();
            }
            keep
        });
        for def in wanted {
            // Sources from the configuration have no vault of their own.
            let vault = match def.vault_id {
                Some(vault) => vault,
                None => match self.vaults.instance_default().await {
                    Ok(vault) => vault,
                    Err(e) => {
                        warn!("Source '{}' has no vault to import into: {e}", def.name);
                        continue;
                    }
                },
            };
            let fingerprint =
                serde_json::to_string(&(&def.name, &def.config, vault)).unwrap_or_default();
            if running.get(&def.id).is_some_and(|(f, _)| *f == fingerprint) {
                continue;
            }
            if let Some((_, old)) = running.remove(&def.id) {
                old.abort();
            }
            let def = IngestionSourceDef {
                vault_id: Some(vault),
                ..def
            };
            let source = match self.factory.build(&def) {
                Ok(source) => source,
                Err(e) => {
                    warn!("Source '{}' cannot start: {e}", def.name);
                    let repo = Arc::clone(&self.managed);
                    let id = def.id;
                    let status = failed_status(&e.to_string());
                    tokio::spawn(async move {
                        let _ = repo.save_status(id, &status).await;
                    });
                    continue;
                }
            };
            info!("Starting ingestion source '{}'", def.name);
            let handle = tokio::spawn(run_source(
                def.id,
                def.name.clone(),
                source,
                Arc::new(BoundSink {
                    vault,
                    inner: Arc::clone(&self.sink),
                }),
                Arc::clone(&self.managed),
                shutdown.clone(),
            ));
            running.insert(def.id, (fingerprint, handle));
        }
    }
}

fn failed_status(error: &str) -> SourceStatus {
    SourceStatus {
        last_run_at: Some(Utc::now()),
        last_error: Some(error.to_string()),
        ..Default::default()
    }
}

async fn run_source(
    id: Id,
    name: String,
    source: Box<dyn crate::ports::IngestionSource>,
    sink: Arc<dyn IngestSink>,
    repo: Arc<dyn SourceRepository>,
    mut shutdown: watch::Receiver<bool>,
) {
    let interval = Duration::from_secs(source.interval_secs().max(1));
    let mut wait = Duration::ZERO;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = source.wake() => {}
            _ = shutdown.changed() => if *shutdown.borrow() { return },
        }

        let (status, next) = match source.poll(sink.as_ref()).await {
            Ok(PollReport {
                imported,
                skipped,
                failed,
                recheck_in,
            }) => (
                SourceStatus {
                    last_run_at: Some(Utc::now()),
                    last_error: None,
                    last_imported: imported,
                    last_skipped: skipped,
                    last_failed: failed,
                },
                recheck_in.unwrap_or(interval),
            ),
            Err(e) => {
                warn!("Source '{name}' failed: {e}");
                (failed_status(&e.to_string()), interval)
            }
        };
        if let Err(e) = repo.save_status(id, &status).await {
            warn!("Could not save status of '{name}': {e}");
        }
        wait = next;
    }
}

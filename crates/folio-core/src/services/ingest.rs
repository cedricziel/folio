//! Automatic ingestion: the import pipeline and the scheduler that feeds it

use std::collections::{HashMap, HashSet};
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
    CreateTag, Document, Id, IngestAttempt, IngestClaim, IngestItem, IngestItemKey, IngestOutcome,
    IngestResult, IngestionSourceDef, LedgerEntry, PollReport, SourceStatus, VaultId,
    WorkflowAction, WorkflowTrigger,
};
use crate::ports::{
    keys, DocumentImporter, DocumentRepository, DocumentStore, IngestLedger,
    IngestOutcomeRepository, IngestSink, IngestionSourceFactory, SourceRepository, TagRepository,
    VaultIngest, VaultRepository, WorkflowRepository,
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
    outcomes: Option<Arc<dyn IngestOutcomeRepository>>,
    configured_sources: HashSet<Id>,
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
            outcomes: None,
            configured_sources: HashSet::new(),
        }
    }

    pub fn with_outcomes(mut self, outcomes: Arc<dyn IngestOutcomeRepository>) -> Self {
        self.outcomes = Some(outcomes);
        self
    }

    pub fn with_configured_sources(mut self, sources: impl IntoIterator<Item = Id>) -> Self {
        self.configured_sources = sources.into_iter().collect();
        self
    }

    /// Identity and name of the first enabled `ingest.received` workflow that discards the item
    async fn discarding_rule(
        &self,
        vault: VaultId,
        item: &IngestItem,
    ) -> Result<Option<(Id, String)>> {
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
            .map(|w| (w.id, w.name)))
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

    async fn import_claim(
        &self,
        vault: VaultId,
        claim: &LedgerEntry,
        item: IngestItem,
    ) -> Result<Document> {
        let tag_ids = self.resolve_tags(vault, &item.tags).await?;
        self.importer
            .import_reserved(
                vault,
                claim,
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

    // Keep the staged input retryable when the outcome transaction could not commit.
    async fn finish_outcome(
        outcomes: &dyn IngestOutcomeRepository,
        attempt: &IngestAttempt,
        result: IngestResult,
        document: Option<Id>,
        claim: Option<&LedgerEntry>,
    ) -> bool {
        match outcomes.complete(attempt, result, document, claim).await {
            Ok(record) => record.is_some(),
            Err(error) => {
                warn!(source = %attempt.source_id, %error, "Could not commit ingestion outcome");
                false
            }
        }
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
    async fn ingest_observed(
        &self,
        vault: VaultId,
        source_id: Id,
        key: IngestItemKey,
        item: IngestItem,
    ) -> Result<IngestOutcome> {
        let Some(outcomes) = &self.outcomes else {
            return self.ingest(vault, source_id, item).await;
        };
        let rule = self.discarding_rule(vault, &item).await?;
        let decision = rule
            .as_ref()
            .map(|(id, _)| id.to_string())
            .unwrap_or_else(|| "allow".into());
        let fingerprint = hex::encode(Sha256::digest(decision.as_bytes()));
        let origin = if self.configured_sources.contains(&source_id) {
            crate::models::SourceOrigin::Config
        } else {
            crate::models::SourceOrigin::Managed
        };
        let attempt = match outcomes
            .claim(
                vault,
                source_id,
                item.source.kind,
                origin,
                &key,
                &fingerprint,
            )
            .await?
        {
            IngestClaim::Claimed(attempt) => attempt,
            IngestClaim::Busy => return Ok(IngestOutcome::Busy),
            IngestClaim::Completed(IngestResult::Skipped) => {
                return Ok(IngestOutcome::Skipped {
                    rule: rule.map(|(_, name)| name).unwrap_or_default(),
                });
            }
            IngestClaim::Completed(_) => return Ok(IngestOutcome::Duplicate),
        };
        if let Some((_, rule)) = rule {
            return Ok(
                if Self::finish_outcome(
                    outcomes.as_ref(),
                    &attempt,
                    IngestResult::Skipped,
                    None,
                    None,
                )
                .await
                {
                    IngestOutcome::Skipped { rule }
                } else {
                    IngestOutcome::Busy
                },
            );
        }
        let content = content_key(&item.data);
        let claim = match self.claim(vault, &content, source_id).await {
            Ok(ClaimOutcome::Claimed(claim)) => claim,
            Ok(ClaimOutcome::Duplicate) => {
                return Ok(
                    if Self::finish_outcome(
                        outcomes.as_ref(),
                        &attempt,
                        IngestResult::Duplicate,
                        None,
                        None,
                    )
                    .await
                    {
                        IngestOutcome::Duplicate
                    } else {
                        IngestOutcome::Busy
                    },
                );
            }
            Ok(ClaimOutcome::Busy) => {
                if outcomes.release(&attempt).await.is_err() {
                    warn!("Could not release ingestion observation");
                }
                return Ok(IngestOutcome::Busy);
            }
            Err(error) => {
                return if Self::finish_outcome(
                    outcomes.as_ref(),
                    &attempt,
                    IngestResult::Failed,
                    None,
                    None,
                )
                .await
                {
                    Err(error)
                } else {
                    Ok(IngestOutcome::Busy)
                };
            }
        };
        match self.import_claim(vault, &claim, item).await {
            Ok(document) => Ok(
                if Self::finish_outcome(
                    outcomes.as_ref(),
                    &attempt,
                    IngestResult::Imported,
                    Some(document.id),
                    Some(&claim),
                )
                .await
                {
                    IngestOutcome::Imported(Box::new(document))
                } else {
                    IngestOutcome::Busy
                },
            ),
            Err(error) => {
                if Self::finish_outcome(
                    outcomes.as_ref(),
                    &attempt,
                    IngestResult::Failed,
                    None,
                    Some(&claim),
                )
                .await
                {
                    Err(error)
                } else {
                    Ok(IngestOutcome::Busy)
                }
            }
        }
    }

    async fn ingest(
        &self,
        vault: VaultId,
        source_id: Id,
        item: IngestItem,
    ) -> Result<IngestOutcome> {
        if let Some((_, rule)) = self.discarding_rule(vault, &item).await? {
            debug!("Rule '{rule}' discarded {}", item.filename);
            return Ok(IngestOutcome::Skipped { rule });
        }

        let key = content_key(&item.data);
        let claim = match self.claim(vault, &key, source_id).await? {
            ClaimOutcome::Claimed(claim) => claim,
            ClaimOutcome::Duplicate => return Ok(IngestOutcome::Duplicate),
            ClaimOutcome::Busy => return Ok(IngestOutcome::Busy),
        };

        let imported = self.import_claim(vault, &claim, item).await;

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
    outcomes: Option<Arc<dyn IngestOutcomeRepository>>,
}

/// Feeds one pipeline from sources that do not know about vaults
struct BoundSink {
    vault: VaultId,
    vaults: Arc<dyn VaultRepository>,
    inner: Arc<dyn VaultIngest>,
}

impl BoundSink {
    /// A poll that started before its vault was deleted must not import into it
    async fn require_active_vault(&self) -> Result<()> {
        match self.vaults.find(self.vault).await? {
            Some(_) => Ok(()),
            None => Err(Error::NotFound {
                entity_type: "Vault",
                id: self.vault.to_string(),
            }),
        }
    }
}

#[async_trait]
impl IngestSink for BoundSink {
    async fn ingest(&self, source_id: Id, item: IngestItem) -> Result<IngestOutcome> {
        self.require_active_vault().await?;
        self.inner.ingest(self.vault, source_id, item).await
    }

    async fn ingest_observed(
        &self,
        source_id: Id,
        key: IngestItemKey,
        item: IngestItem,
    ) -> Result<IngestOutcome> {
        self.require_active_vault().await?;
        self.inner
            .ingest_observed(self.vault, source_id, key, item)
            .await
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
            outcomes: None,
        }
    }

    pub fn with_outcomes(mut self, outcomes: Arc<dyn IngestOutcomeRepository>) -> Self {
        self.outcomes = Some(outcomes);
        self
    }

    pub fn with_reload_interval(mut self, every: Duration) -> Self {
        self.reload_every = every;
        self
    }

    /// Supervise sources until shutdown is signalled
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let mut running: HashMap<Id, (String, JoinHandle<()>)> = HashMap::new();
        let mut warned = HashSet::new();
        loop {
            match self.managed.list_all().await {
                Ok(managed) => {
                    let (managed, shadowed) =
                        super::split_shadowed_paperless(&self.configured, managed);
                    for def in shadowed {
                        if warned.insert(def.id) {
                            warn!(
                                "Paperless source '{}' is not run: the server configuration defines \
                                 this vault's Paperless source",
                                def.name
                            );
                        }
                    }
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
            match self.vaults.find(vault).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if let Some((_, handle)) = running.remove(&def.id) {
                        handle.abort();
                    }
                    continue;
                }
                Err(e) => {
                    warn!("Could not check the vault of source '{}': {e}", def.name);
                    continue;
                }
            }
            let fingerprint =
                serde_json::to_string(&(&def.name, &def.config, vault)).unwrap_or_default();
            if running
                .get(&def.id)
                .is_some_and(|(f, handle)| *f == fingerprint && !handle.is_finished())
            {
                continue;
            }
            if let Some((_, old)) = running.remove(&def.id) {
                old.abort();
            }
            let def = IngestionSourceDef {
                vault_id: Some(vault),
                ..def
            };
            let run = SourceRun {
                vault,
                id: def.id,
                name: def.name.clone(),
                origin: def.origin,
                kind: def.config.kind(),
                managed: Arc::clone(&self.managed),
                vaults: Arc::clone(&self.vaults),
                outcomes: self.outcomes.clone(),
            };
            let source = match self.factory.build(&def) {
                Ok(source) => source,
                Err(e) => {
                    warn!("Source '{}' cannot start: {e}", def.name);
                    let status = failed_status(&e);
                    tokio::spawn(async move {
                        if run.save_status(&status).await.is_err() {
                            warn!("Could not save source startup failure");
                        }
                    });
                    continue;
                }
            };
            info!("Starting ingestion source '{}'", def.name);
            let handle = tokio::spawn(run_source(
                run,
                source,
                Arc::new(BoundSink {
                    vault,
                    vaults: Arc::clone(&self.vaults),
                    inner: Arc::clone(&self.sink),
                }),
                shutdown.clone(),
            ));
            running.insert(def.id, (fingerprint, handle));
        }
    }
}

/// Raw adapter errors can carry hosts, paths or credentials, so clients only see a fixed reason
fn failure_reason(error: &Error) -> &'static str {
    match error {
        Error::Validation(_) | Error::Duplicate { .. } => "The source configuration is invalid",
        Error::NotFound { .. } => "The source location was not found",
        Error::Storage(_) => "The source could not be reached or read",
        Error::Conflict(_) => "The source is busy",
        Error::Forbidden(_) => "The source is not allowed to run",
        Error::Repository(_) | Error::AiService(_) => "Folio could not process this source run",
    }
}

fn failed_status(error: &Error) -> SourceStatus {
    SourceStatus {
        last_run_at: Some(Utc::now()),
        last_error: Some(failure_reason(error).to_string()),
        ..Default::default()
    }
}

struct SourceRun {
    vault: VaultId,
    id: Id,
    name: String,
    origin: crate::models::SourceOrigin,
    kind: crate::models::SourceKind,
    managed: Arc<dyn SourceRepository>,
    vaults: Arc<dyn VaultRepository>,
    outcomes: Option<Arc<dyn IngestOutcomeRepository>>,
}

impl SourceRun {
    async fn save_status(&self, status: &SourceStatus) -> Result<()> {
        match &self.outcomes {
            Some(outcomes) => {
                outcomes
                    .record_source_status(self.vault, self.id, self.origin, self.kind, status)
                    .await
            }
            None => self.managed.save_status(self.id, status).await,
        }
    }
}

async fn run_source(
    run: SourceRun,
    source: Box<dyn crate::ports::IngestionSource>,
    sink: Arc<dyn IngestSink>,
    mut shutdown: watch::Receiver<bool>,
) {
    let name = &run.name;
    let interval = Duration::from_secs(source.interval_secs().max(1));
    let mut wait = Duration::ZERO;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = source.wake() => {}
            _ = shutdown.changed() => if *shutdown.borrow() { return },
        }

        match run.vaults.find(run.vault).await {
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(error) => {
                warn!("Cannot check vault for source '{name}': {error}");
                wait = interval;
                continue;
            }
        }
        if run.origin == crate::models::SourceOrigin::Managed {
            match run.managed.find(run.vault, run.id).await {
                Ok(Some(def)) if def.enabled => {}
                Ok(_) => return,
                Err(error) => {
                    warn!("Could not verify ingestion source '{name}': {error}");
                    wait = interval;
                    continue;
                }
            }
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
                (failed_status(&e), interval)
            }
        };
        if let Err(e) = run.save_status(&status).await {
            warn!("Could not save status of '{name}': {e}");
        }
        wait = next;
    }
}

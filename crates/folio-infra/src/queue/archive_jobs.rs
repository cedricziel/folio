//! Admission and execution for revision-bound PDF/A copies.

use std::sync::Arc;
use std::time::Duration;

use folio_core::models::{
    ArchiveAttempt, ArchiveCompletion, ArchiveSource, ArchiveState, ArchiveTool, FileIdentity, Id,
    ValidatedArchive, VaultId,
};
use folio_core::ports::{keys, ArchivalRepository, DocumentStore};
use folio_core::{Error, Result};
use futures::{stream, StreamExt};
use sha2::{Digest, Sha256};
use tracing::warn;
use uuid::Uuid;

use crate::archival::{ArchiveConverter, SkipReason, MAX_FILE_BYTES};
use crate::database::{ArchiveOutputCleanup, Database};

const STORAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Application service for archival requests. Callers check vault access
/// before calling these methods (including write access for enqueue/retry).
pub struct ArchiveService {
    database: Database,
    repository: Arc<dyn ArchivalRepository>,
    store: Arc<dyn DocumentStore>,
    converter: ArchiveConverter,
    pub(super) automatic: bool,
}

impl ArchiveService {
    pub fn new(
        database: Database,
        repository: Arc<dyn ArchivalRepository>,
        store: Arc<dyn DocumentStore>,
        converter: ArchiveConverter,
        automatic: bool,
    ) -> Self {
        Self {
            database,
            repository,
            store,
            converter,
            automatic,
        }
    }

    /// Look up the current attempt for display or download admission.
    pub async fn status(&self, vault: VaultId, document: Id) -> Result<Option<ArchiveAttempt>> {
        self.repository.current_attempt(vault, document).await
    }

    /// Read the exact validated bytes for the current original revision.
    pub async fn download(&self, vault: VaultId, document: Id) -> Result<Option<Vec<u8>>> {
        let Some(attempt) = self.status(vault, document).await? else {
            return Ok(None);
        };
        let ArchiveState::Completed(ArchiveCompletion::Ready(output)) = &attempt.state else {
            return Ok(None);
        };
        let file = &output.file;
        if output
            .validate_metadata(&attempt.source, attempt.id)
            .is_err()
            || file.byte_size > MAX_FILE_BYTES
        {
            return Ok(None);
        }
        let bytes = tokio::time::timeout(STORAGE_TIMEOUT, async {
            self.store
                .get(&file.storage_key)
                .await?
                .into_bytes_limited(file.byte_size as usize)
                .await
        })
        .await
        .map_err(|_| Error::Storage("Archival copy read timed out".into()))?;
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(Error::NotFound { .. } | Error::Validation(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        if bytes.len() as u64 != file.byte_size
            || hex::encode(Sha256::digest(&bytes)) != file.sha256
            || self.status(vault, document).await?.as_ref() != Some(&attempt)
        {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Enqueue a first attempt, or return the existing attempt ID.
    pub async fn enqueue(&self, vault: VaultId, document: Id) -> Result<Id> {
        self.admit(vault, document, None).await
    }

    /// Retry the observed failed or skipped attempt. A stale status or a
    /// concurrent successful retry is a conflict. Refresh status before retrying.
    pub async fn retry(&self, vault: VaultId, document: Id, expected: Id) -> Result<Id> {
        self.admit(vault, document, Some(expected)).await
    }

    async fn admit(&self, vault: VaultId, document: Id, retry: Option<Id>) -> Result<Id> {
        let source = self
            .repository
            .current_source(vault, document)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity_type: "Verified original",
                id: document.to_string(),
            })?;
        let current = self.status(vault, document).await?;
        if let Some(existing) = current.filter(|existing| existing.source == source) {
            if retry.is_none() {
                return Ok(existing.id);
            }
            if retry != Some(existing.id)
                || !matches!(
                    existing.state,
                    ArchiveState::Completed(
                        ArchiveCompletion::Failed(_) | ArchiveCompletion::Skipped(_)
                    )
                )
            {
                return Err(Error::Conflict("Archive attempt cannot be retried".into()));
            }
        } else if retry.is_some() {
            return Err(Error::Conflict(
                "Archive attempt is no longer current".into(),
            ));
        }
        let attempt = Uuid::new_v4();
        if self
            .repository
            .enqueue_archive(&source, attempt, retry)
            .await?
        {
            return Ok(attempt);
        }
        if retry.is_none() {
            if let Some(current) = self
                .status(vault, document)
                .await?
                .filter(|current| current.source == source)
            {
                return Ok(current.id);
            }
        }
        Err(Error::Conflict("Archive source or attempt changed".into()))
    }

    /// Enqueue up to 32 supported verified originals without an attempt in
    /// this vault. Repeat until zero. Existing failed attempts require retry.
    pub async fn backfill(&self, vault: VaultId, limit: u32) -> Result<usize> {
        self.discover(Some(vault), limit.min(32)).await
    }

    pub(super) async fn discover(&self, vault: Option<VaultId>, limit: u32) -> Result<usize> {
        let mut count = 0;
        for (vault, document) in self.database.archive_candidates(vault, limit).await? {
            match self.enqueue(vault, document).await {
                Ok(_) => count += 1,
                Err(Error::NotFound { .. } | Error::Conflict(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(count)
    }

    pub(super) async fn repair(&self) -> Result<()> {
        self.database.retire_interrupted_archives().await?;
        stream::iter(self.database.take_archive_cleanup_batch().await?)
            .for_each_concurrent(8, |entry| async move {
                if let Err(error) = self.cleanup(&entry).await {
                    warn!(attempt = %entry.attempt, %error, "Archive cleanup will be retried");
                }
            })
            .await;
        Ok(())
    }

    pub(super) async fn generate(&self, queue: &super::JobQueue, job: &super::Job) -> Result<()> {
        let super::JobType::GenerateArchive {
            vault_id: vault,
            document_id: document,
            source_revision_id: revision,
            attempt_id: attempt,
        } = job.job_type
        else {
            return Err(Error::Repository("Expected an archival job".into()));
        };
        if attempt != job.id {
            return Err(Error::Repository(
                "Archival job does not match its attempt".into(),
            ));
        }
        let Some(current) = self.status(vault, document).await?.filter(|current| {
            current.id == attempt
                && current.source.revision_id == revision
                && current.state == ArchiveState::Queued
        }) else {
            return Ok(());
        };
        let source = current.source;
        let completion = match self.convert(&source).await? {
            Ok(output) => {
                return self.publish(&source, queue, job, output).await;
            }
            Err(completion) => completion,
        };
        if queue.renew(job).await? {
            self.repository
                .complete_archive(&source, attempt, &completion)
                .await?;
        }
        Ok(())
    }

    async fn convert(
        &self,
        source: &ArchiveSource,
    ) -> Result<std::result::Result<crate::archival::ValidatedArchive, ArchiveCompletion>> {
        let original = &source.original;
        if original.byte_size > MAX_FILE_BYTES {
            return Ok(Err(ArchiveCompletion::Skipped(
                SkipReason::ResourceLimit.to_string(),
            )));
        }
        let bytes = tokio::time::timeout(STORAGE_TIMEOUT, async {
            let object = self.store.get(&original.storage_key).await?;
            object.into_bytes_limited(original.byte_size as usize).await
        })
        .await
        .map_err(|_| Error::Storage("Archival original read timed out".into()))?;
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(Error::Validation(_)) => return Ok(Err(original_mismatch())),
            Err(error) => return Err(error),
        };
        if bytes.len() as u64 != original.byte_size
            || hex::encode(Sha256::digest(&bytes)) != original.sha256
        {
            return Ok(Err(original_mismatch()));
        }
        Ok(self
            .converter
            .convert(&bytes, &original.mime_type)
            .await
            .map_err(|reason| match reason {
                SkipReason::UnsupportedMediaType
                | SkipReason::ResourceLimit
                | SkipReason::InvalidPdf
                | SkipReason::EncryptedPdf
                | SkipReason::SignedPdf
                | SkipReason::EmbeddedAttachment => ArchiveCompletion::Skipped(reason.to_string()),
                _ => ArchiveCompletion::Failed(reason.to_string()),
            }))
    }

    async fn publish(
        &self,
        source: &ArchiveSource,
        queue: &super::JobQueue,
        job: &super::Job,
        output: crate::archival::ValidatedArchive,
    ) -> Result<()> {
        let attempt = job.id;
        let (document, revision) = (source.document_id, source.revision_id);
        if !self.database.reserve_archive_output(source, job).await? {
            // A replay can never adopt another writer's bytes. The terminal
            // transition fences the reserved writer, and repair owns cleanup.
            if queue.renew(job).await? {
                self.repository
                    .complete_archive(
                        source,
                        attempt,
                        &ArchiveCompletion::Failed("Archival publication was interrupted".into()),
                    )
                    .await?;
            }
            return Ok(());
        }
        let key = keys::revision_artifact(document, revision, attempt);
        let metadata = ValidatedArchive {
            file: FileIdentity::from_bytes(&key, "application/pdf", &output.bytes),
            profile: output.profile.into(),
            converter: match output.ocrmypdf_version {
                Some(version) => ArchiveTool {
                    name: "OCRmyPDF".into(),
                    version,
                },
                None => ArchiveTool {
                    name: "Folio (original reuse)".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
            },
            validator: ArchiveTool {
                name: "veraPDF".into(),
                version: output.verapdf_version,
            },
        };
        let write = tokio::time::timeout(STORAGE_TIMEOUT, async {
            let created = self.store.put_if_absent(&key, output.bytes.into()).await?;
            if created {
                self.store.clear_create_staging(&key).await?;
            }
            Ok::<_, Error>(created)
        })
        .await;
        let completion = match write {
            Ok(Ok(true)) => {
                self.database.settle_archive_write(attempt).await?;
                ArchiveCompletion::Ready(metadata)
            }
            _ => ArchiveCompletion::Failed("Could not stage the archival copy".into()),
        };
        self.repository
            .complete_archive(source, attempt, &completion)
            .await
            .map(|_| ())
    }

    async fn cleanup(&self, entry: &ArchiveOutputCleanup) -> Result<()> {
        if !self
            .repository
            .retains_archive(entry.document, entry.revision, entry.attempt)
            .await?
        {
            let key = keys::revision_artifact(entry.document, entry.revision, entry.attempt);
            tokio::time::timeout(
                STORAGE_TIMEOUT,
                self.store.discard_create(&key, entry.write_settled),
            )
            .await
            .map_err(|_| Error::Storage("Archival cleanup timed out".into()))??;
            if entry.write_settled {
                self.database
                    .acknowledge_archive_cleanup(entry.attempt)
                    .await?;
            }
        }
        Ok(())
    }
}

fn original_mismatch() -> ArchiveCompletion {
    ArchiveCompletion::Failed("Stored original does not match the verified identity".into())
}

#[cfg(test)]
mod tests;

//! Uploads that are safe to retry with an `Idempotency-Key`

use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::NewUpload;
use crate::error::Result;
use crate::models::{Document, Id, VaultId};
use crate::ports::{
    upload_fingerprint, DocumentImporter, DocumentRepository, UploadClaim, UploadReceiptKey,
    UploadReceiptRepository, UPLOAD_CLAIM_HEARTBEAT, UPLOAD_CLAIM_TIMEOUT,
    UPLOAD_RECEIPT_RETENTION,
};

/// What a keyed upload did.
#[derive(Debug)]
pub enum UploadOutcome {
    /// This request stored the document
    Created(Document),
    /// An earlier request with the same key and content stored it
    Replayed(Document),
    /// Another request with the key is still storing the document; retry shortly
    InProgress,
    /// An earlier request with the same key stored a document that has since been deleted
    Deleted,
    /// The key was already used for a different upload; retrying with it never succeeds
    KeyMismatch,
}

/// How long a claim lives without renewal, and how often an import renews it.
#[derive(Debug, Clone, Copy)]
pub struct ClaimTiming {
    pub stale_after: chrono::Duration,
    pub heartbeat: std::time::Duration,
}

impl Default for ClaimTiming {
    fn default() -> Self {
        Self {
            stale_after: UPLOAD_CLAIM_TIMEOUT,
            heartbeat: UPLOAD_CLAIM_HEARTBEAT,
        }
    }
}

/// Imports an upload at most once per key: the first request reserves a document id, and every later attempt
/// with the same key and content resumes or replays that document. An attempt renews its claim while it imports;
/// one that is cut off leaves the reservation behind, so its retry resumes the same document instead of creating
/// a second one, and a document deleted since is reported as deleted rather than created again.
pub struct IdempotentUploads {
    receipts: Arc<dyn UploadReceiptRepository>,
    importer: Arc<dyn DocumentImporter>,
    documents: Arc<dyn DocumentRepository>,
    timing: ClaimTiming,
}

impl IdempotentUploads {
    pub fn new(
        receipts: Arc<dyn UploadReceiptRepository>,
        importer: Arc<dyn DocumentImporter>,
        documents: Arc<dyn DocumentRepository>,
    ) -> Self {
        Self {
            receipts,
            importer,
            documents,
            timing: ClaimTiming::default(),
        }
    }

    pub fn with_timing(mut self, timing: ClaimTiming) -> Self {
        self.timing = timing;
        self
    }

    pub async fn upload(&self, key: UploadReceiptKey, upload: NewUpload) -> Result<UploadOutcome> {
        let fingerprint = upload_fingerprint(
            &upload.filename,
            upload.mime_type.as_deref().unwrap_or_default(),
            upload.title.as_deref().unwrap_or_default(),
            &upload.data,
        );
        let now = Utc::now();
        match self
            .receipts
            .claim(&key, &fingerprint, now, now - self.timing.stale_after)
            .await?
        {
            UploadClaim::Claimed {
                document,
                token,
                created,
            } => {
                if created
                    && self
                        .documents
                        .find_by_id(key.vault, document)
                        .await?
                        .is_none()
                {
                    // An earlier attempt created the document and it has been deleted since; never create it again.
                    self.receipts.complete(&key, token, Utc::now()).await?;
                    return Ok(UploadOutcome::Deleted);
                }
                self.import(&key, document, token, upload).await
            }
            UploadClaim::Completed(id) => self.replay(key.vault, id).await,
            UploadClaim::InProgress(id) => match self.documents.find_by_id(key.vault, id).await? {
                // The holder stored the document but never recorded it, e.g. because its request was dropped.
                Some(document) if document.file_path.is_some() => {
                    self.receipts.complete_stored(&key, Utc::now()).await?;
                    Ok(UploadOutcome::Replayed(document))
                }
                _ => Ok(UploadOutcome::InProgress),
            },
            UploadClaim::Mismatch => Ok(UploadOutcome::KeyMismatch),
        }
    }

    /// Imports under the reserved id while renewing the claim.
    async fn import(
        &self,
        key: &UploadReceiptKey,
        document: Id,
        token: Id,
        upload: NewUpload,
    ) -> Result<UploadOutcome> {
        let work = async {
            self.importer.begin_as(key.vault, document, &upload).await?;
            self.receipts.mark_created(key, token, Utc::now()).await?;
            self.importer.import_as(key.vault, document, upload).await
        };
        tokio::pin!(work);
        let mut heartbeat = tokio::time::interval(self.timing.heartbeat);
        heartbeat.tick().await;
        let imported = loop {
            tokio::select! {
                imported = &mut work => break imported,
                _ = heartbeat.tick() => {
                    match self.receipts.heartbeat(key, token, Utc::now()).await {
                        Ok(true) => {}
                        Ok(false) => tracing::warn!(
                            document = %document,
                            "Upload claim was taken over by a retry while this attempt was still importing"
                        ),
                        Err(e) => tracing::warn!("Failed to renew upload claim: {e}"),
                    }
                }
            }
        };
        match imported {
            Ok(stored) => {
                if self.receipts.complete(key, token, Utc::now()).await? {
                    Ok(UploadOutcome::Created(stored))
                } else {
                    // Another attempt took the claim over and recorded the same document.
                    Ok(UploadOutcome::Replayed(stored))
                }
            }
            Err(e) => {
                // Only the attempt still holding the claim cleans up, so it never removes a document another
                // attempt is finishing.
                match self.receipts.release(key, token).await {
                    Ok(true) => self.discard(key.vault, document).await,
                    Ok(false) => {}
                    Err(release) => tracing::error!("Failed to release upload claim: {release}"),
                }
                Err(e)
            }
        }
    }

    async fn replay(&self, vault: VaultId, id: Id) -> Result<UploadOutcome> {
        Ok(match self.documents.find_by_id(vault, id).await? {
            Some(document) => UploadOutcome::Replayed(document),
            None => UploadOutcome::Deleted,
        })
    }

    /// Removes a reserved document that never got its original, like a failed plain import does.
    async fn discard(&self, vault: VaultId, id: Id) {
        let unfinished = match self.documents.find_by_id(vault, id).await {
            Ok(Some(document)) => document.file_path.is_none(),
            Ok(None) => false,
            Err(e) => {
                tracing::error!("Failed to look up unfinished upload {id}: {e}");
                false
            }
        };
        if !unfinished {
            return;
        }
        let cleanup = async {
            self.documents.delete(vault, id).await?;
            self.documents.request_purge(vault, id).await
        };
        if let Err(e) = cleanup.await {
            tracing::error!("Failed to schedule cleanup of unfinished upload {id}: {e}");
        }
    }

    /// Forgets receipts without activity for [`UPLOAD_RECEIPT_RETENTION`], removing the unfinished documents of
    /// those that never completed. Returns how many receipts expired without completing.
    pub async fn sweep(&self, now: DateTime<Utc>) -> Result<usize> {
        let expired = self.receipts.expire(now - UPLOAD_RECEIPT_RETENTION).await?;
        for reservation in &expired {
            self.discard(reservation.vault, reservation.document).await;
        }
        Ok(expired.len())
    }
}

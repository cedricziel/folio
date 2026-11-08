//! Uploads that are safe to retry with an `Idempotency-Key`

use std::sync::Arc;

use chrono::Utc;

use super::NewUpload;
use crate::error::{Error, Result};
use crate::models::{Document, Id, VaultId};
use crate::ports::{
    upload_fingerprint, DocumentImporter, DocumentRepository, UploadClaim, UploadReceiptKey,
    UploadReceiptRepository,
};

/// What a keyed upload did.
#[derive(Debug)]
pub enum UploadOutcome {
    /// This request stored the document
    Created(Document),
    /// An earlier request with the same key and content stored it
    Replayed(Document),
    /// An earlier request with the same key stored a document that has since been deleted
    Deleted,
}

/// Imports an upload at most once per key: the first request reserves a document id, and every later attempt
/// with the same key and content resumes or replays that document. An attempt that is dropped, crashes or fails
/// part-way leaves the reservation behind, so its retry cannot create a second document.
pub struct IdempotentUploads {
    receipts: Arc<dyn UploadReceiptRepository>,
    importer: Arc<dyn DocumentImporter>,
    documents: Arc<dyn DocumentRepository>,
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
        }
    }

    pub async fn upload(&self, key: UploadReceiptKey, upload: NewUpload) -> Result<UploadOutcome> {
        let fingerprint = upload_fingerprint(
            &upload.filename,
            upload.mime_type.as_deref().unwrap_or_default(),
            upload.title.as_deref().unwrap_or_default(),
            &upload.data,
        );
        match self.receipts.claim(&key, &fingerprint, Utc::now()).await? {
            UploadClaim::Claimed { document, token } => {
                match self.importer.import_as(key.vault, document, upload).await {
                    Ok(document) => {
                        self.receipts.complete(&key, token).await?;
                        Ok(UploadOutcome::Created(document))
                    }
                    Err(e) => {
                        if let Err(abandon) = self.receipts.abandon(&key, token).await {
                            tracing::error!("Failed to release upload receipt: {abandon}");
                        }
                        Err(e)
                    }
                }
            }
            UploadClaim::Completed(id) => self.replay(key.vault, id).await,
            UploadClaim::InProgress(id) => match self.documents.find_by_id(key.vault, id).await? {
                // The holder stored the document but never recorded it, e.g. because its request was dropped.
                Some(document) if document.file_path.is_some() => {
                    Ok(UploadOutcome::Replayed(document))
                }
                _ => Err(Error::Conflict(
                    "An upload with this Idempotency-Key is still in progress; retry shortly"
                        .into(),
                )),
            },
            UploadClaim::Mismatch => Err(Error::Conflict(
                "This Idempotency-Key was already used for a different upload".into(),
            )),
        }
    }

    async fn replay(&self, vault: VaultId, id: Id) -> Result<UploadOutcome> {
        Ok(match self.documents.find_by_id(vault, id).await? {
            Some(document) => UploadOutcome::Replayed(document),
            None => UploadOutcome::Deleted,
        })
    }
}

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::models::{Id, VaultId};

/// How long an upload's `Idempotency-Key` keeps returning the document it created.
pub const UPLOAD_RECEIPT_RETENTION: Duration = Duration::days(7);

/// A claim that never completed (the server stopped mid-upload) stops blocking its key after this.
pub const UPLOAD_CLAIM_TIMEOUT: Duration = Duration::minutes(15);

/// One `Idempotency-Key` as one caller used it in one vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadReceiptKey {
    pub vault: VaultId,
    /// `user:<id>` for a signed-in caller, `system` otherwise
    pub actor_scope: String,
    pub key: Id,
}

/// What an earlier upload with the same key left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadClaim {
    /// No earlier upload: store the document, then `complete` or `release` the claim
    Claimed,
    /// An earlier request with the same content created this document
    Completed(Id),
    /// The key was already used for different content
    Mismatch,
    /// Another request with this key is still storing its document
    InProgress,
}

/// Receipts that make document uploads safe to retry after an uncertain outcome.
#[async_trait]
pub trait UploadReceiptRepository: Send + Sync {
    /// Claims `key` for content identified by `fingerprint`, or reports what already holds it.
    /// Forgets receipts older than [`UPLOAD_RECEIPT_RETENTION`] and abandoned claims older than
    /// [`UPLOAD_CLAIM_TIMEOUT`].
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> Result<UploadClaim>;

    /// Records the document a claimed upload created.
    async fn complete(&self, key: &UploadReceiptKey, document: Id) -> Result<()>;

    /// Drops a claim whose upload failed, so the same key can be retried.
    async fn release(&self, key: &UploadReceiptKey) -> Result<()>;
}

/// Identifies what an upload sends, so a key reused for a different file is refused.
pub fn upload_fingerprint(filename: &str, mime_type: &str, title: &str, data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    for part in [filename.as_bytes(), mime_type.as_bytes(), title.as_bytes()] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher.update(Sha256::digest(data));
    hex::encode(hasher.finalize())
}

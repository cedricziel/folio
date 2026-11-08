use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::models::{Id, VaultId};

/// How long after its last activity an upload's `Idempotency-Key` keeps returning the document it created.
pub const UPLOAD_RECEIPT_RETENTION: Duration = Duration::days(7);

/// An attempt renews its claim this often while it imports.
pub const UPLOAD_CLAIM_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);

/// A claim not renewed for this long (its request was cut off, the server stopped) may be taken over by a retry,
/// which resumes the import under the same reserved document.
pub const UPLOAD_CLAIM_TIMEOUT: Duration = Duration::minutes(2);

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
    /// This request holds the key: import the document under the reserved `document` id, renewing the claim with
    /// `token`, then `complete` it. A taken-over claim keeps the id an earlier attempt reserved; `created` says
    /// whether that attempt already created the document.
    Claimed {
        document: Id,
        token: Id,
        created: bool,
    },
    /// An earlier request with the same content completed and created this document. The replay counts as
    /// activity, so the receipt is kept for another [`UPLOAD_RECEIPT_RETENTION`].
    Completed(Id),
    /// Another request holds the key and may still be storing this document
    InProgress(Id),
    /// The key was already used for different content
    Mismatch,
}

/// A receipt that expired before its upload completed, and the document it had reserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredReservation {
    pub vault: VaultId,
    pub document: Id,
}

/// Receipts that make document uploads safe to retry after an uncertain outcome.
///
/// The document id is reserved when the key is first claimed, so every attempt for one key imports the same
/// document, however often it is interrupted.
#[async_trait]
pub trait UploadReceiptRepository: Send + Sync {
    /// Claims `key` for content identified by `fingerprint`, takes over an uncompleted claim not renewed since
    /// `stale_before`, or reports what holds it.
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> Result<UploadClaim>;

    /// Renews the claim held with `token`. False when another attempt has taken it over.
    async fn heartbeat(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool>;

    /// Records that the attempt holding `token` created the reserved document.
    async fn mark_created(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool>;

    /// Marks the claim held with `token` as completed. False when another attempt has taken it over.
    async fn complete(&self, key: &UploadReceiptKey, token: Id, now: DateTime<Utc>)
        -> Result<bool>;

    /// Completes an open claim whose reserved document was found stored, whoever holds it.
    async fn complete_stored(&self, key: &UploadReceiptKey, now: DateTime<Utc>) -> Result<()>;

    /// Drops the uncompleted claim held with `token`, so the next request starts over. False when another attempt
    /// has taken it over.
    async fn release(&self, key: &UploadReceiptKey, token: Id) -> Result<bool>;

    /// Deletes receipts without activity since `before` and returns the documents reserved by those that never
    /// completed.
    async fn expire(&self, before: DateTime<Utc>) -> Result<Vec<ExpiredReservation>>;
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

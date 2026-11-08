use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::models::{Id, VaultId};

/// How long an upload's `Idempotency-Key` keeps returning the document it created.
pub const UPLOAD_RECEIPT_RETENTION: Duration = Duration::days(7);

/// A claim not completed by then (the request was dropped, the server stopped) may be taken over by a retry,
/// which resumes the import under the same reserved document.
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
    /// This request holds the key: import the document under the reserved `document` id, then `complete` with
    /// `token`. A taken-over stale claim keeps the id an earlier attempt reserved.
    Claimed { document: Id, token: Id },
    /// An earlier request with the same content completed and created this document
    Completed(Id),
    /// Another request holds the key and may still be storing this document
    InProgress(Id),
    /// The key was already used for different content
    Mismatch,
}

/// Receipts that make document uploads safe to retry after an uncertain outcome.
///
/// The document id is reserved when the key is first claimed, so every attempt for one key imports the same
/// document, however often it is interrupted.
#[async_trait]
pub trait UploadReceiptRepository: Send + Sync {
    /// Claims `key` for content identified by `fingerprint`, takes over a claim whose holder has not completed it
    /// within [`UPLOAD_CLAIM_TIMEOUT`], or reports what holds it. Forgets receipts older than
    /// [`UPLOAD_RECEIPT_RETENTION`].
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> Result<UploadClaim>;

    /// Marks the claim held with `token` as completed. False when another attempt has taken it over.
    async fn complete(&self, key: &UploadReceiptKey, token: Id) -> Result<bool>;

    /// Lets the next request with this key resume at once after a failed attempt, under the same document.
    async fn abandon(&self, key: &UploadReceiptKey, token: Id) -> Result<()>;
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

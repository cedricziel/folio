use crate::models::{FileIdentity, UnverifiedOriginal};
use crate::ports::{DocumentRepository, DocumentStore};
use crate::{Error, Result};

/// Verify each unverified original once per run. The cursor is local so a later
/// run retries unavailable originals, while persisted successful reads are skipped.
pub async fn backfill_original_identities(
    documents: &dyn DocumentRepository,
    store: &dyn DocumentStore,
    batch: u32,
) -> Result<u64> {
    if batch == 0 {
        return Err(Error::Validation(
            "Verification batch must be positive".into(),
        ));
    }
    let mut verified = 0;
    let mut after = None;
    loop {
        let originals = documents.list_unverified_originals(after, batch).await?;
        let Some(last) = originals.last() else {
            return Ok(verified);
        };
        after = Some(last.id);
        for original in &originals {
            let identity = match read_identity(store, original).await {
                Ok(identity) => Some(identity),
                Err(error) => {
                    tracing::warn!(file_id = %original.id, %error, "Original verification failed");
                    None
                }
            };
            let applied = documents
                .record_original_verification(original, identity.as_ref())
                .await?;
            if applied && identity.is_some() {
                verified += 1;
            }
        }
    }
}

async fn read_identity(
    store: &dyn DocumentStore,
    original: &UnverifiedOriginal,
) -> Result<FileIdentity> {
    store
        .get(&original.storage_key)
        .await?
        .file_identity(&original.storage_key, &original.mime_type)
        .await
}

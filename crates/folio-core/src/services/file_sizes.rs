//! Records the size of originals stored before Folio kept track of it

use crate::error::Result;
use crate::ports::{DocumentRepository, DocumentStore};

const BATCH: u32 = 200;

/// Reads each unrecorded size from storage; returns how many it recorded.
/// A missing object or a failed read leaves that document for the next run.
pub async fn backfill_file_sizes(
    documents: &dyn DocumentRepository,
    store: &dyn DocumentStore,
) -> Result<u64> {
    let mut recorded = 0;
    let mut after = None;
    loop {
        let batch = documents.list_missing_file_size(after, BATCH).await?;
        let Some(last) = batch.last() else {
            return Ok(recorded);
        };
        after = Some(last.id);
        for document in &batch {
            let Some(path) = document.file_path.as_deref() else {
                continue;
            };
            match store.size(path).await {
                Ok(Some(size)) => {
                    documents
                        .set_file_size(document.vault_id, document.id, path, size)
                        .await?;
                    recorded += 1;
                }
                Ok(None) => tracing::warn!("Original of {} is missing from storage", document.id),
                Err(e) => tracing::warn!("Cannot read the size of {}: {e}", document.id),
            }
        }
    }
}

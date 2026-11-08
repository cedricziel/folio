//! Records the size of originals stored before Folio kept track of it

use crate::error::Result;
use crate::ports::{DocumentRepository, DocumentStore};

/// Reads each unrecorded size from storage, `batch` documents at a time, and
/// returns how many it recorded. A document whose object is missing or whose
/// size cannot be read or saved is left for the next run.
pub async fn backfill_file_sizes(
    documents: &dyn DocumentRepository,
    store: &dyn DocumentStore,
    batch: u32,
) -> Result<u64> {
    let mut recorded = 0;
    let mut missing = 0;
    let mut after = None;
    loop {
        let page = documents.list_missing_file_size(after, batch).await?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.id);
        for document in &page {
            let Some(path) = document.file_path.as_deref() else {
                continue;
            };
            let size = match store.size(path).await {
                Ok(Some(size)) => size,
                Ok(None) => {
                    missing += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Cannot read the size of {}: {e}", document.id);
                    continue;
                }
            };
            match documents
                .set_file_size(document.vault_id, document.id, path, size)
                .await
            {
                Ok(()) => recorded += 1,
                Err(e) => tracing::warn!("Cannot record the size of {}: {e}", document.id),
            }
        }
    }
    if missing > 0 {
        tracing::warn!("{missing} documents have no original in storage");
    }
    Ok(recorded)
}

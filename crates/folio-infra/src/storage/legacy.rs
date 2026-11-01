//! One-off move from the pre-key layout to object keys
//!
//! Before object keys, `documents.file_path` held `{id}/{sanitized name}` and
//! thumbnails lived in `thumbnails/`. Reads keep working on those rows; this
//! pass copies the objects to their `documents/{id}/...` keys and updates the
//! row. It is idempotent and safe to run on every start.

use tracing::{info, warn};

use folio_core::error::Result;
use folio_core::models::Document;
use folio_core::ports::{keys, DocumentRepository, DocumentStore};

/// Returns how many documents were moved.
pub async fn migrate_legacy_layout(
    store: &dyn DocumentStore,
    documents: &dyn DocumentRepository,
) -> Result<usize> {
    let mut moved = 0;
    for document in documents.list(None).await? {
        match migrate_document(store, documents, &document).await {
            Ok(true) => moved += 1,
            Ok(false) => {}
            Err(e) => warn!("Could not migrate storage layout of {}: {}", document.id, e),
        }
    }
    if moved > 0 {
        info!("Moved {moved} documents to the object-key storage layout");
    }
    Ok(moved)
}

async fn migrate_document(
    store: &dyn DocumentStore,
    documents: &dyn DocumentRepository,
    document: &Document,
) -> Result<bool> {
    let id = document.id;
    let mut changed = false;

    if let Some(legacy) = document
        .file_path
        .as_deref()
        .filter(|p| !p.starts_with("documents/"))
    {
        if store.exists(legacy).await? {
            let new_key = keys::original(id);
            let data = store.get_bytes(legacy).await?;
            store.put(&new_key, data.into()).await?;
            documents.set_file_path(id, &new_key).await?;
            store.delete(legacy).await?;
            changed = true;
        }
    }

    for page in 1..=document.page_count.unwrap_or(1) {
        let legacy = keys::legacy::page(id, page);
        if store.exists(&legacy).await? {
            let data = store.get_bytes(&legacy).await?;
            store.put(&keys::page(id, page), data.into()).await?;
            store.delete(&legacy).await?;
            changed = true;
        }
    }

    Ok(changed)
}

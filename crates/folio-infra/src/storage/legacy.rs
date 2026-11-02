//! One-off move from the pre-key layout to object keys
//!
//! Before object keys, `documents.file_path` held `{id}/{sanitized name}` and
//! thumbnails lived in `thumbnails/`. Reads keep working on those rows; this
//! pass copies the objects to their `documents/{id}/...` keys and updates the
//! row. The old objects stay in place, so rolling back to an older release
//! still finds its files; [`cleanup_legacy_layout`] removes them on request.
//!
//! After a pass without errors the `storage.layout_version` setting records
//! that it is done, and later starts skip it.

use tracing::{info, warn};

use folio_core::error::Result;
use folio_core::models::Document;
use folio_core::ports::{keys, DocumentRepository, DocumentStore, SettingsRepository};

const LAYOUT_VERSION_KEY: &str = "storage.layout_version";
const LAYOUT_VERSION: &str = "2";

/// Copy old-layout objects to their keys. Returns how many documents changed.
pub async fn migrate_legacy_layout(
    store: &dyn DocumentStore,
    documents: &dyn DocumentRepository,
    settings: &dyn SettingsRepository,
) -> Result<usize> {
    if settings
        .get(LAYOUT_VERSION_KEY)
        .await?
        .is_some_and(|s| s.value == LAYOUT_VERSION)
    {
        return Ok(0);
    }

    let mut moved = 0;
    let mut failed = false;
    for document in documents.list_all().await? {
        match migrate_document(store, documents, &document).await {
            Ok(true) => moved += 1,
            Ok(false) => {}
            Err(e) => {
                failed = true;
                warn!("Could not migrate storage layout of {}: {}", document.id, e);
            }
        }
    }
    if moved > 0 {
        info!("Copied {moved} documents to the object-key storage layout");
    }
    if !failed {
        settings.set(LAYOUT_VERSION_KEY, LAYOUT_VERSION).await?;
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
        let new_key = keys::original(id);
        if store.exists(legacy).await? {
            if !store.exists(&new_key).await? {
                store.copy(legacy, &new_key).await?;
            }
            documents
                .set_file_path(document.vault_id, id, &new_key)
                .await?;
            changed = true;
        }
    }

    for page in 1..=document.page_count.unwrap_or(1) {
        let (legacy, new_key) = (keys::legacy::page(id, page), keys::page(id, page));
        if store.exists(&legacy).await? && !store.exists(&new_key).await? {
            store.copy(&legacy, &new_key).await?;
            changed = true;
        }
    }

    Ok(changed)
}

/// Delete the old-layout copies of documents whose new-layout original exists.
/// Run it only after the rollback window has passed. Returns how many
/// documents were cleaned.
pub async fn cleanup_legacy_layout(
    store: &dyn DocumentStore,
    documents: &dyn DocumentRepository,
) -> Result<usize> {
    let mut cleaned = 0;
    for document in documents.list_all().await? {
        let id = document.id;
        if !store.exists(&keys::original(id)).await? {
            continue;
        }
        let mut removed = store
            .delete_prefix(&keys::legacy::document_prefix(id))
            .await?;
        for page in 1..=document.page_count.unwrap_or(1) {
            let legacy = keys::legacy::page(id, page);
            if store.exists(&legacy).await? {
                store.delete(&legacy).await?;
                removed += 1;
            }
        }
        if removed > 0 {
            cleaned += 1;
        }
    }
    Ok(cleaned)
}

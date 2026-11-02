use std::sync::Arc;

use async_trait::async_trait;
use folio_core::error::{Error, Result};
use folio_core::models::VaultId;
use folio_core::ports::{DocumentImporter, DocumentStore, StoredObject};
use folio_core::services::{Importer, NewUpload};
use folio_infra::{Database, JobType, ObjectStoreDocumentStore, Repositories};

struct FailedPutStore;

#[async_trait]
impl DocumentStore for FailedPutStore {
    fn backend(&self) -> &'static str {
        "unavailable"
    }

    async fn put(&self, _key: &str, _data: bytes::Bytes) -> Result<()> {
        Err(Error::Storage("object storage unavailable".into()))
    }

    async fn get(&self, _key: &str) -> Result<StoredObject> {
        unreachable!()
    }

    async fn size(&self, _key: &str) -> Result<Option<u64>> {
        unreachable!()
    }

    async fn exists(&self, _key: &str) -> Result<bool> {
        unreachable!()
    }

    async fn delete(&self, _key: &str) -> Result<()> {
        unreachable!()
    }

    async fn delete_prefix(&self, _prefix: &str) -> Result<u64> {
        unreachable!()
    }

    async fn list(&self, _prefix: &str) -> Result<Vec<String>> {
        unreachable!()
    }
}

async fn fixture() -> (Database, Repositories, VaultId) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    let vault = repositories.vaults.instance_default().await.unwrap();
    (db, repositories, vault)
}

fn upload() -> NewUpload {
    NewUpload {
        filename: "invoice.pdf".into(),
        data: b"uploaded document".to_vec(),
        mime_type: Some("application/pdf".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn failed_object_put_schedules_durable_cleanup_of_the_incomplete_upload() {
    let (_db, r, vault) = fixture().await;
    let importer = Importer::new(
        r.documents.clone(),
        Arc::new(FailedPutStore),
        Arc::new(r.jobs.clone()),
    );

    let error = importer.import(vault, upload()).await.unwrap_err();

    assert!(matches!(error, Error::Storage(_)));
    assert!(r.documents.list(vault, None).await.unwrap().is_empty());
    let trash = r.documents.list_trash(vault).await.unwrap();
    assert_eq!(trash.len(), 1);
    assert!(trash[0].purge_requested_at.is_some());
    let jobs = r
        .jobs
        .find_active_jobs_for_document(trash[0].id)
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(matches!(jobs[0].job_type, JobType::PurgeDocument { .. }));
}

#[tokio::test]
async fn failed_file_path_update_keeps_cleanup_intent_for_the_stored_object() {
    let (db, r, vault) = fixture().await;
    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    sqlx::query("CREATE TRIGGER refuse_file_path BEFORE UPDATE OF file_path ON documents BEGIN SELECT RAISE(ABORT, 'refused'); END")
        .execute(pool).await.unwrap();
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));

    assert!(importer.import(vault, upload()).await.is_err());

    assert!(r.documents.list(vault, None).await.unwrap().is_empty());
    let trash = r.documents.list_trash(vault).await.unwrap();
    assert_eq!(trash.len(), 1);
    assert!(trash[0].file_path.is_none());
    assert!(trash[0].purge_requested_at.is_some());
    assert!(store
        .exists(&folio_core::ports::keys::original(trash[0].id))
        .await
        .unwrap());
    assert!(matches!(
        r.jobs
            .find_active_jobs_for_document(trash[0].id)
            .await
            .unwrap()[0]
            .job_type,
        JobType::PurgeDocument { .. }
    ));
}

#[tokio::test]
async fn failed_processing_enqueue_keeps_the_successfully_stored_upload() {
    let (db, r, vault) = fixture().await;
    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    sqlx::query("CREATE TRIGGER refuse_jobs BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT, 'refused'); END")
        .execute(pool).await.unwrap();
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));

    let document = importer.import(vault, upload()).await.unwrap();

    assert!(r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .is_some());
    assert!(r.documents.list_trash(vault).await.unwrap().is_empty());
    assert!(store
        .exists(&folio_core::ports::keys::original(document.id))
        .await
        .unwrap());
}

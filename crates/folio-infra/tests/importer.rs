use std::sync::Arc;

use async_trait::async_trait;
use folio_core::error::{Error, Result};
use folio_core::models::VaultId;
use folio_core::ports::{DocumentImporter, DocumentStore, StoredObject};
use folio_core::services::{Importer, NewUpload};
use folio_infra::{Database, JobType, ObjectStoreDocumentStore, Repositories};

async fn created_events(r: &Repositories) -> Vec<folio_core::OutboxEvent> {
    r.outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::days(1), 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| matches!(event.event, Some(folio_core::DocumentEvent::Created { .. })))
        .collect()
}

async fn upload_listener(
    r: &Repositories,
    vault: VaultId,
    store: Arc<dyn DocumentStore>,
) -> (folio_core::services::WorkflowEngine, uuid::Uuid) {
    use folio_core::models::{WorkflowAction, WorkflowInput, WorkflowTrigger};
    let workflow = r
        .workflows
        .create(
            vault,
            WorkflowInput {
                name: "uploaded".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentUploaded,
                conditions: Default::default(),
                actions: vec![WorkflowAction::Webhook {
                    url: "http://127.0.0.1:1/unused".into(),
                    method: Default::default(),
                    headers: Default::default(),
                    secret: None,
                }],
            },
        )
        .await
        .unwrap();
    (
        folio_core::services::WorkflowEngine::new(
            r.workflows.clone(),
            r.documents.clone(),
            r.correspondents.clone(),
            r.document_types.clone(),
            Arc::new(folio_infra::StorageFileMetadata::new(store)),
            "http://folio.test",
        ),
        workflow.id,
    )
}

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
        Ok(false)
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
async fn imports_record_the_size_of_the_original() {
    let (_db, r, vault) = fixture().await;
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let importer = Importer::new(r.documents.clone(), store, Arc::new(r.jobs.clone()));

    let document = importer.import(vault, upload()).await.unwrap();

    assert_eq!(document.file_size, Some(b"uploaded document".len() as u64));
}

#[tokio::test]
async fn backfill_records_sizes_from_storage_and_skips_missing_objects() {
    let (_db, r, vault) = fixture().await;
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let stored = r
        .documents
        .create_for_upload(vault, Default::default())
        .await
        .unwrap();
    let lost = r
        .documents
        .create_for_upload(vault, Default::default())
        .await
        .unwrap();
    store
        .put("stored/original", b"twelve bytes".as_slice().into())
        .await
        .unwrap();
    for (id, path) in [(stored.id, "stored/original"), (lost.id, "lost/original")] {
        r.documents.set_file_path(vault, id, path).await.unwrap();
    }

    let recorded =
        folio_core::services::backfill_file_sizes(r.documents.as_ref(), store.as_ref(), 1)
            .await
            .unwrap();

    assert_eq!(recorded, 1);
    for (id, size) in [(stored.id, Some(12)), (lost.id, None)] {
        let document = r.documents.find_by_id(vault, id).await.unwrap().unwrap();
        assert_eq!(document.file_size, size);
    }
}

#[tokio::test]
async fn backfill_pages_past_every_batch_and_skips_documents_being_purged() {
    let (_db, r, vault) = fixture().await;
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let mut ids = Vec::new();
    for n in 0..5 {
        let document = r
            .documents
            .create_for_upload(vault, Default::default())
            .await
            .unwrap();
        let path = format!("doc-{n}/original");
        store.put(&path, vec![0; n + 1].into()).await.unwrap();
        r.documents
            .set_file_path(vault, document.id, &path)
            .await
            .unwrap();
        ids.push((document.id, n as u64 + 1));
    }
    let (purging, _) = ids.pop().unwrap();
    r.documents.delete(vault, purging).await.unwrap();
    r.documents.request_purge(vault, purging).await.unwrap();

    let recorded =
        folio_core::services::backfill_file_sizes(r.documents.as_ref(), store.as_ref(), 2)
            .await
            .unwrap();

    assert_eq!(recorded, 4);
    for (id, size) in ids {
        let document = r.documents.find_by_id(vault, id).await.unwrap().unwrap();
        assert_eq!(document.file_size, Some(size));
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
    assert!(created_events(&r).await.is_empty());
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
    assert!(created_events(&r).await.is_empty());

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

#[tokio::test]
async fn failed_reserved_upload_keeps_the_document_for_retry_without_cleanup() {
    let (_db, r, vault) = fixture().await;
    let claim = folio_core::models::LedgerEntry {
        vault_id: vault,
        key: folio_core::services::content_key(&upload().data),
        source_id: uuid::Uuid::new_v4(),
        document_id: Some(uuid::Uuid::new_v4()),
        outcome: "claimed".into(),
        created_at: chrono::DateTime::from_timestamp_micros(chrono::Utc::now().timestamp_micros())
            .unwrap(),
    };
    assert!(r.ledger.insert(vault, &claim).await.unwrap());
    let failed = Importer::new(
        r.documents.clone(),
        Arc::new(FailedPutStore),
        Arc::new(r.jobs.clone()),
    );

    assert!(matches!(
        failed.import_reserved(vault, &claim, upload()).await,
        Err(Error::Storage(_))
    ));

    assert!(created_events(&r).await.is_empty());
    let retained = r.documents.list(vault, None).await.unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(Some(retained[0].id), claim.document_id);
    assert!(r.documents.list_trash(vault).await.unwrap().is_empty());
    assert!(r
        .jobs
        .find_active_jobs_for_document(retained[0].id)
        .await
        .unwrap()
        .is_empty());
    let retry = Importer::new(
        r.documents.clone(),
        Arc::new(ObjectStoreDocumentStore::in_memory()),
        Arc::new(r.jobs.clone()),
    );
    let document = retry
        .import_reserved(vault, &claim, upload())
        .await
        .unwrap();
    assert_eq!(Some(document.id), claim.document_id);
    assert_eq!(created_events(&r).await.len(), 1);
    assert_eq!(r.documents.list(vault, None).await.unwrap().len(), 1);
}

struct PausedPutStore {
    inner: ObjectStoreDocumentStore,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl DocumentStore for PausedPutStore {
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }
    async fn put(&self, key: &str, data: bytes::Bytes) -> Result<()> {
        self.started.notify_one();
        self.release.notified().await;
        self.inner.put(key, data).await
    }
    async fn get(&self, key: &str) -> Result<StoredObject> {
        self.inner.get(key).await
    }
    async fn size(&self, key: &str) -> Result<Option<u64>> {
        self.inner.size(key).await
    }
    async fn exists(&self, key: &str) -> Result<bool> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(key).await
    }
    async fn delete_prefix(&self, prefix: &str) -> Result<u64> {
        self.inner.delete_prefix(prefix).await
    }
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        self.inner.list(prefix).await
    }
}

#[tokio::test]
async fn original_written_after_purge_is_removed_without_resurrecting_the_document() {
    let (_db, r, vault) = fixture().await;
    let claim = folio_core::models::LedgerEntry {
        vault_id: vault,
        key: folio_core::services::content_key(&upload().data),
        source_id: uuid::Uuid::new_v4(),
        document_id: Some(uuid::Uuid::new_v4()),
        outcome: "claimed".into(),
        created_at: chrono::DateTime::from_timestamp_micros(chrono::Utc::now().timestamp_micros())
            .unwrap(),
    };
    assert!(r.ledger.insert(vault, &claim).await.unwrap());
    let store = Arc::new(PausedPutStore {
        inner: ObjectStoreDocumentStore::in_memory(),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));
    let attempt = claim.clone();
    let owner =
        tokio::spawn(async move { importer.import_reserved(vault, &attempt, upload()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), store.started.notified())
        .await
        .unwrap();
    let id = claim.document_id.unwrap();
    r.documents.delete(vault, id).await.unwrap();
    r.documents.request_purge(vault, id).await.unwrap();
    folio_core::ports::purge_document(store.as_ref(), id)
        .await
        .unwrap();
    r.documents.finalize_purge(vault, id).await.unwrap();

    store.release.notify_one();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), owner)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );

    assert!(r.documents.locate_retained(id).await.unwrap().is_none());
    assert!(!store
        .exists(&folio_core::ports::keys::original(id))
        .await
        .unwrap());
    assert_eq!(
        r.ledger
            .find(vault, &claim.key)
            .await
            .unwrap()
            .unwrap()
            .outcome,
        "deleted"
    );
}

#[tokio::test]
async fn original_written_between_purge_and_finalization_is_removed() {
    let (_db, r, vault) = fixture().await;
    let claim = folio_core::models::LedgerEntry {
        vault_id: vault,
        key: folio_core::services::content_key(&upload().data),
        source_id: uuid::Uuid::new_v4(),
        document_id: Some(uuid::Uuid::new_v4()),
        outcome: "claimed".into(),
        created_at: chrono::DateTime::from_timestamp_micros(chrono::Utc::now().timestamp_micros())
            .unwrap(),
    };
    assert!(r.ledger.insert(vault, &claim).await.unwrap());
    let store = Arc::new(PausedPutStore {
        inner: ObjectStoreDocumentStore::in_memory(),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));
    let attempt = claim.clone();
    let owner =
        tokio::spawn(async move { importer.import_reserved(vault, &attempt, upload()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), store.started.notified())
        .await
        .unwrap();
    let id = claim.document_id.unwrap();
    r.documents.delete(vault, id).await.unwrap();
    r.documents.request_purge(vault, id).await.unwrap();
    folio_core::ports::purge_document(store.as_ref(), id)
        .await
        .unwrap();

    store.release.notify_one();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), owner)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );

    assert!(r
        .documents
        .find_trashed_by_id(vault, id)
        .await
        .unwrap()
        .unwrap()
        .purge_requested_at
        .is_some());
    r.documents.finalize_purge(vault, id).await.unwrap();
    assert!(r.documents.locate_retained(id).await.unwrap().is_none());
    assert!(!store
        .exists(&folio_core::ports::keys::original(id))
        .await
        .unwrap());
    assert_eq!(
        r.ledger
            .find(vault, &claim.key)
            .await
            .unwrap()
            .unwrap()
            .outcome,
        "deleted"
    );
}

#[tokio::test]
async fn original_written_after_soft_delete_is_preserved_for_restore() {
    let (_db, r, vault) = fixture().await;
    let claim = folio_core::models::LedgerEntry {
        vault_id: vault,
        key: folio_core::services::content_key(&upload().data),
        source_id: uuid::Uuid::new_v4(),
        document_id: Some(uuid::Uuid::new_v4()),
        outcome: "claimed".into(),
        created_at: chrono::DateTime::from_timestamp_micros(chrono::Utc::now().timestamp_micros())
            .unwrap(),
    };
    assert!(r.ledger.insert(vault, &claim).await.unwrap());
    let store = Arc::new(PausedPutStore {
        inner: ObjectStoreDocumentStore::in_memory(),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));
    let attempt = claim.clone();
    let owner =
        tokio::spawn(async move { importer.import_reserved(vault, &attempt, upload()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), store.started.notified())
        .await
        .unwrap();
    let id = claim.document_id.unwrap();
    r.documents.delete(vault, id).await.unwrap();

    store.release.notify_one();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), owner)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );

    r.documents.restore(vault, id).await.unwrap();
    assert!(r.documents.find_by_id(vault, id).await.unwrap().is_some());
    assert_eq!(
        store
            .get(&folio_core::ports::keys::original(id))
            .await
            .unwrap()
            .into_bytes()
            .await
            .unwrap(),
        b"uploaded document"
    );
}

#[tokio::test]
async fn blocked_original_put_emits_no_created_event_until_atomic_finalization() {
    let (_db, r, vault) = fixture().await;
    let store = Arc::new(PausedPutStore {
        inner: ObjectStoreDocumentStore::in_memory(),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let (engine, workflow_id) = upload_listener(&r, vault, store.clone()).await;
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));
    let owner = tokio::spawn(async move { importer.import(vault, upload()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), store.started.notified())
        .await
        .unwrap();

    assert!(created_events(&r).await.is_empty());
    assert!(r
        .workflows
        .list_runs(vault, workflow_id, 10)
        .await
        .unwrap()
        .is_empty());
    store.release.notify_one();
    let document = tokio::time::timeout(std::time::Duration::from_secs(5), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let events = created_events(&r).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.as_ref().unwrap().document_id(), document.id);
    assert_eq!(engine.handle(&events[0]).await.unwrap().len(), 1);
    r.documents
        .finalize_upload(vault, document.id, document.file_path.as_ref().unwrap(), 0)
        .await
        .unwrap();
    assert_eq!(created_events(&r).await.len(), 1);
    assert!(engine.handle(&events[0]).await.unwrap().is_empty());
    assert_eq!(
        r.workflows
            .list_runs(vault, workflow_id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_outbox_insert_rolls_back_upload_path_and_can_be_retried_once() {
    let (db, r, vault) = fixture().await;
    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    let claim = folio_core::models::LedgerEntry {
        vault_id: vault,
        key: folio_core::services::content_key(&upload().data),
        source_id: uuid::Uuid::new_v4(),
        document_id: Some(uuid::Uuid::new_v4()),
        outcome: "claimed".into(),
        created_at: chrono::DateTime::from_timestamp_micros(chrono::Utc::now().timestamp_micros())
            .unwrap(),
    };
    assert!(r.ledger.insert(vault, &claim).await.unwrap());
    let store = Arc::new(ObjectStoreDocumentStore::in_memory());
    let importer = Importer::new(r.documents.clone(), store.clone(), Arc::new(r.jobs.clone()));
    sqlx::query("CREATE TRIGGER refuse_created BEFORE INSERT ON outbox_events WHEN NEW.type = 'created' BEGIN SELECT RAISE(ABORT, 'refused'); END").execute(pool).await.unwrap();

    assert!(importer
        .import_reserved(vault, &claim, upload())
        .await
        .is_err());

    let document = r
        .documents
        .find_by_id(vault, claim.document_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(document.file_path.is_none());
    assert!(store
        .exists(&folio_core::ports::keys::original(document.id))
        .await
        .unwrap());
    assert!(created_events(&r).await.is_empty());
    sqlx::query("DROP TRIGGER refuse_created")
        .execute(pool)
        .await
        .unwrap();
    importer
        .import_reserved(vault, &claim, upload())
        .await
        .unwrap();
    importer
        .import_reserved(vault, &claim, upload())
        .await
        .unwrap();
    assert_eq!(created_events(&r).await.len(), 1);
}

#[tokio::test]
async fn awaited_upload_preserves_authenticated_actor_without_reusing_uploader_identity() {
    use folio_core::models::UserId;
    use folio_core::mutation::with_actor;

    let (_db, r, vault) = fixture().await;
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let importer = Importer::new(r.documents.clone(), store, Arc::new(r.jobs.clone()));
    for id in ["authenticated-importer", "contributor-owner"] {
        r.auth
            .create_user(folio_auth::User {
                id: id.into(),
                username: id.into(),
                password_hash: "!".into(),
                created_at: 1,
                email: None,
                display_name: None,
                is_admin: false,
                is_owner: false,
                auth_source: "local".into(),
                external_id: None,
            })
            .await
            .unwrap();
    }
    let actor = UserId::from("authenticated-importer");
    let mut input = upload();
    input.uploaded_by = Some(UserId::from("contributor-owner"));
    let document = with_actor(Some(actor.clone()), importer.import(vault, input))
        .await
        .unwrap();
    assert_eq!(document.created_by, Some(actor.clone()));
    assert_eq!(document.updated_by, Some(actor.clone()));
    assert_eq!(
        document.uploaded_by,
        Some(UserId::from("contributor-owner"))
    );
    let events = created_events(&r).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].envelope.as_ref().unwrap().actor_id(),
        Some(&actor)
    );
    with_actor(
        Some(UserId::from("retrying-user")),
        r.documents.finalize_upload(
            vault,
            document.id,
            document.file_path.as_deref().unwrap(),
            0,
        ),
    )
    .await
    .unwrap();
    let retried = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retried.updated_by, document.updated_by);
    assert_eq!(retried.updated_at, document.updated_at);
    assert_eq!(created_events(&r).await.len(), 1);
}

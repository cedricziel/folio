mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use folio_core::models::Id;
use folio_core::ports::{DocumentImporter, DocumentStore, UploadClaim, UploadReceiptKey};
use folio_core::services::{IdempotentUploads, Importer, NewUpload, UploadOutcome};
use folio_infra::{Database, ObjectStoreDocumentStore, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn key(r_vault: folio_core::models::VaultId, scope: &str) -> UploadReceiptKey {
    UploadReceiptKey {
        vault: r_vault,
        actor_scope: scope.into(),
        key: Id::new_v4(),
    }
}

fn claimed(claim: UploadClaim) -> (Id, Id) {
    match claim {
        UploadClaim::Claimed { document, token } => (document, token),
        other => panic!("expected a claim, got {other:?}"),
    }
}

async fn receipts_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let receipts = &r.upload_receipts;
    let now = Utc::now();

    let first = key(vault, "user:alice");
    let (document, token) = claimed(receipts.claim(&first, "scan", now).await.unwrap());
    assert_eq!(
        receipts.claim(&first, "scan", now).await.unwrap(),
        UploadClaim::InProgress(document),
        "a second request must not store the file while the first still may"
    );
    assert!(
        !receipts.complete(&first, Id::new_v4()).await.unwrap(),
        "only the claim's holder completes it"
    );
    assert!(receipts.complete(&first, token).await.unwrap());
    assert_eq!(
        receipts.claim(&first, "scan", now).await.unwrap(),
        UploadClaim::Completed(document)
    );
    assert_eq!(
        receipts.claim(&first, "other content", now).await.unwrap(),
        UploadClaim::Mismatch
    );

    let other_caller = UploadReceiptKey {
        actor_scope: "user:bob".into(),
        ..first.clone()
    };
    let (other_document, _) = claimed(receipts.claim(&other_caller, "scan", now).await.unwrap());
    assert_ne!(other_document, document, "keys are scoped to the caller");

    let abandoned = key(vault, "system");
    let (reserved, stale_token) = claimed(
        receipts
            .claim(&abandoned, "scan", now - Duration::minutes(20))
            .await
            .unwrap(),
    );
    let (resumed, token) = claimed(receipts.claim(&abandoned, "scan", now).await.unwrap());
    assert_eq!(
        resumed, reserved,
        "taking over a stale claim keeps its reserved document"
    );
    assert_ne!(token, stale_token);
    assert!(
        !receipts.complete(&abandoned, stale_token).await.unwrap(),
        "the stale holder lost the claim"
    );
    assert_eq!(
        receipts.claim(&abandoned, "scan", now).await.unwrap(),
        UploadClaim::InProgress(reserved)
    );

    let failed = key(vault, "system");
    let (reserved, token) = claimed(receipts.claim(&failed, "scan", now).await.unwrap());
    receipts.abandon(&failed, token).await.unwrap();
    assert_eq!(
        claimed(receipts.claim(&failed, "scan", now).await.unwrap()).0,
        reserved,
        "a failed attempt can be resumed at once, under the same document"
    );

    let expired = key(vault, "system");
    let (old, token) = claimed(
        receipts
            .claim(&expired, "scan", now - Duration::days(8))
            .await
            .unwrap(),
    );
    receipts.complete(&expired, token).await.unwrap();
    assert_ne!(
        claimed(receipts.claim(&expired, "scan", now).await.unwrap()).0,
        old,
        "receipts are kept for a week"
    );
}

fn upload(data: &str) -> NewUpload {
    NewUpload {
        title: Some("Scan".into()),
        filename: "scan.pdf".into(),
        mime_type: Some("application/pdf".into()),
        data: data.as_bytes().to_vec(),
        ..Default::default()
    }
}

struct Flow {
    uploads: Arc<IdempotentUploads>,
    importer: Arc<Importer>,
}

fn flow(r: &Repositories) -> Flow {
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let importer = Arc::new(Importer::new(
        r.documents.clone(),
        store,
        Arc::new(r.jobs.clone()),
    ));
    Flow {
        uploads: Arc::new(IdempotentUploads::new(
            r.upload_receipts.clone(),
            importer.clone(),
            r.documents.clone(),
        )),
        importer,
    }
}

async fn documents(r: &Repositories, vault: folio_core::models::VaultId) -> usize {
    r.documents.list_ids(vault, None).await.unwrap().len()
}

fn document(outcome: UploadOutcome) -> folio_core::models::Document {
    match outcome {
        UploadOutcome::Created(document) | UploadOutcome::Replayed(document) => document,
        UploadOutcome::Deleted => panic!("expected a document"),
    }
}

async fn uploads_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let f = flow(r);

    // A repeat returns the first document.
    let repeat = key(vault, "user:alice");
    let first = f
        .uploads
        .upload(repeat.clone(), upload("pixels"))
        .await
        .unwrap();
    assert!(matches!(first, UploadOutcome::Created(_)));
    let first = document(first);
    let again = f
        .uploads
        .upload(repeat.clone(), upload("pixels"))
        .await
        .unwrap();
    assert!(matches!(&again, UploadOutcome::Replayed(d) if d.id == first.id));
    assert!(
        f.uploads
            .upload(repeat.clone(), upload("other pixels"))
            .await
            .is_err(),
        "a key reused for other content conflicts"
    );
    assert_eq!(documents(r, vault).await, 1);

    // Two concurrent requests with one key store one document.
    let concurrent = key(vault, "user:alice");
    let (a, b) = tokio::join!(
        tokio::spawn({
            let (uploads, key) = (f.uploads.clone(), concurrent.clone());
            async move { uploads.upload(key, upload("concurrent")).await }
        }),
        tokio::spawn({
            let (uploads, key) = (f.uploads.clone(), concurrent.clone());
            async move { uploads.upload(key, upload("concurrent")).await }
        }),
    );
    let ids: Vec<Id> = [a.unwrap(), b.unwrap()]
        .into_iter()
        .filter_map(|r| r.ok())
        .map(|o| document(o).id)
        .collect();
    assert!(!ids.is_empty());
    assert!(ids.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(documents(r, vault).await, 2);

    // The request was dropped after its import committed but before it completed the receipt.
    let dropped = key(vault, "user:alice");
    let (reserved, _) = claimed(
        r.upload_receipts
            .claim(&dropped, &fingerprint("dropped"), Utc::now())
            .await
            .unwrap(),
    );
    f.importer
        .import_as(vault, reserved, upload("dropped"))
        .await
        .unwrap();
    let retried = f
        .uploads
        .upload(dropped.clone(), upload("dropped"))
        .await
        .unwrap();
    assert!(matches!(&retried, UploadOutcome::Replayed(d) if d.id == reserved));
    assert_eq!(documents(r, vault).await, 3);

    // The server stopped before importing anything; a retry after the claim went stale imports under the reserved id.
    let stale = key(vault, "user:alice");
    let (reserved, _) = claimed(
        r.upload_receipts
            .claim(
                &stale,
                &fingerprint("stale"),
                Utc::now() - Duration::minutes(20),
            )
            .await
            .unwrap(),
    );
    let resumed = f
        .uploads
        .upload(stale.clone(), upload("stale"))
        .await
        .unwrap();
    assert!(matches!(&resumed, UploadOutcome::Created(d) if d.id == reserved));
    assert_eq!(documents(r, vault).await, 4);

    // A deleted original is reported as gone rather than stored again.
    r.documents.delete(vault, first.id).await.unwrap();
    assert!(matches!(
        f.uploads.upload(repeat, upload("pixels")).await.unwrap(),
        UploadOutcome::Deleted
    ));
}

fn fingerprint(data: &str) -> String {
    folio_core::ports::upload_fingerprint("scan.pdf", "application/pdf", "Scan", data.as_bytes())
}

async fn sqlite() -> (tempfile::TempDir, Repositories) {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}",
        directory.path().join("receipts.db").display()
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    (directory, db.repositories().await)
}

async fn postgres() -> Option<(impl Drop, Repositories)> {
    let container = common::container_or_skip(
        "PostgreSQL",
        Postgres::default().with_tag("17-alpine").start().await,
    )?;
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    Some((container, db.repositories().await))
}

#[tokio::test]
async fn sqlite_upload_receipts_reserve_take_over_and_expire() {
    let (_dir, r) = sqlite().await;
    receipts_contract(&r).await;
}

#[tokio::test]
async fn sqlite_keyed_uploads_never_store_a_document_twice() {
    let (_dir, r) = sqlite().await;
    uploads_contract(&r).await;
}

#[tokio::test]
async fn postgres_upload_receipts_reserve_take_over_and_expire() {
    let Some((_container, r)) = postgres().await else {
        return;
    };
    receipts_contract(&r).await;
}

#[tokio::test]
async fn postgres_keyed_uploads_never_store_a_document_twice() {
    let Some((_container, r)) = postgres().await else {
        return;
    };
    uploads_contract(&r).await;
}

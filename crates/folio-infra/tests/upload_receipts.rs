mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{Id, VaultId};
use folio_core::ports::{
    DocumentImporter, DocumentStore, StoredObject, UploadClaim, UploadReceiptKey,
    UPLOAD_RECEIPT_RETENTION,
};
use folio_core::services::{ClaimTiming, IdempotentUploads, Importer, NewUpload, UploadOutcome};
use folio_infra::{Database, ObjectStoreDocumentStore, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
use tokio::sync::Notify;

fn key(vault: VaultId, scope: &str) -> UploadReceiptKey {
    UploadReceiptKey {
        vault,
        actor_scope: scope.into(),
        key: Id::new_v4(),
    }
}

fn claimed(claim: UploadClaim) -> (Id, Id, bool) {
    match claim {
        UploadClaim::Claimed {
            document,
            token,
            created,
        } => (document, token, created),
        other => panic!("expected a claim, got {other:?}"),
    }
}

fn stale(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    now - Duration::minutes(2)
}

async fn receipts_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let receipts = &r.upload_receipts;
    let now = Utc::now();

    let first = key(vault, "user:alice");
    let (document, token, created) = claimed(
        receipts
            .claim(&first, "scan", now, stale(now))
            .await
            .unwrap(),
    );
    assert!(!created);
    assert_eq!(
        receipts
            .claim(&first, "scan", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::InProgress(document),
        "a second request must not store the file while the first still may"
    );
    assert!(
        !receipts.complete(&first, Id::new_v4(), now).await.unwrap(),
        "only the claim's holder completes it"
    );
    assert!(receipts.complete(&first, token, now).await.unwrap());
    assert_eq!(
        receipts
            .claim(&first, "scan", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Completed(document)
    );
    assert_eq!(
        receipts
            .claim(&first, "other content", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Mismatch
    );

    let other_caller = UploadReceiptKey {
        actor_scope: "user:bob".into(),
        ..first.clone()
    };
    let (other_document, _, _) = claimed(
        receipts
            .claim(&other_caller, "scan", now, stale(now))
            .await
            .unwrap(),
    );
    assert_ne!(other_document, document, "keys are scoped to the caller");

    // A claim renewed by its heartbeat is not taken over; one left alone is, keeping its document.
    let renewed = key(vault, "system");
    let (reserved, holder, _) = claimed(
        receipts
            .claim(
                &renewed,
                "scan",
                now - Duration::minutes(5),
                stale(now - Duration::minutes(5)),
            )
            .await
            .unwrap(),
    );
    assert!(receipts.heartbeat(&renewed, holder, now).await.unwrap());
    assert_eq!(
        receipts
            .claim(&renewed, "scan", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::InProgress(reserved)
    );
    let later = now + Duration::minutes(5);
    assert!(receipts.mark_created(&renewed, holder, now).await.unwrap());
    let (resumed, taker, created) = claimed(
        receipts
            .claim(&renewed, "scan", later, stale(later))
            .await
            .unwrap(),
    );
    assert_eq!(
        resumed, reserved,
        "taking over a stale claim keeps its reserved document"
    );
    assert!(
        created,
        "the takeover learns that the document was already created"
    );
    assert!(
        !receipts.heartbeat(&renewed, holder, later).await.unwrap(),
        "the stale holder lost the claim"
    );
    assert!(!receipts.complete(&renewed, holder, later).await.unwrap());
    assert!(receipts.complete(&renewed, taker, later).await.unwrap());

    // A released claim starts over with a new document.
    let failed = key(vault, "system");
    let (reserved, token, _) = claimed(
        receipts
            .claim(&failed, "scan", now, stale(now))
            .await
            .unwrap(),
    );
    assert!(receipts.release(&failed, token).await.unwrap());
    assert_ne!(
        claimed(
            receipts
                .claim(&failed, "scan", now, stale(now))
                .await
                .unwrap()
        )
        .0,
        reserved
    );

    // A stored document found by a replay completes the open claim.
    let replayed = key(vault, "system");
    let (reserved, _, _) = claimed(
        receipts
            .claim(&replayed, "scan", now, stale(now))
            .await
            .unwrap(),
    );
    receipts.complete_stored(&replayed, now).await.unwrap();
    assert_eq!(
        receipts
            .claim(&replayed, "scan", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Completed(reserved)
    );

    // Receipts expire a week after their last activity; only uncompleted ones report their reservation.
    let quiet = now - UPLOAD_RECEIPT_RETENTION - Duration::hours(1);
    let abandoned = key(vault, "system");
    let (abandoned_document, _, _) = claimed(
        receipts
            .claim(&abandoned, "scan", quiet, stale(quiet))
            .await
            .unwrap(),
    );
    let finished = key(vault, "system");
    let (_, finished_token, _) = claimed(
        receipts
            .claim(&finished, "scan", quiet, stale(quiet))
            .await
            .unwrap(),
    );
    receipts
        .complete(&finished, finished_token, quiet)
        .await
        .unwrap();
    let expired = receipts
        .expire(now - UPLOAD_RECEIPT_RETENTION)
        .await
        .unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].document, abandoned_document);
    assert_eq!(expired[0].vault, vault);
    assert!(!matches!(
        receipts
            .claim(&finished, "scan", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Completed(_)
    ));

    // A replay counts as activity, so a client retrying within a week of its last attempt still gets its document.
    let old = now - UPLOAD_RECEIPT_RETENTION + Duration::hours(1);
    let retried = key(vault, "system");
    let (retried_document, retried_token, _) = claimed(
        receipts
            .claim(&retried, "scan", old, stale(old))
            .await
            .unwrap(),
    );
    receipts
        .complete(&retried, retried_token, old)
        .await
        .unwrap();
    assert_eq!(
        receipts
            .claim(&retried, "other content", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Mismatch
    );
    assert_eq!(
        receipts
            .claim(&retried, "scan", now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Completed(retried_document)
    );
    let sweep = now + Duration::hours(2);
    receipts
        .expire(sweep - UPLOAD_RECEIPT_RETENTION)
        .await
        .unwrap();
    assert_eq!(
        receipts
            .claim(&retried, "scan", sweep, stale(sweep))
            .await
            .unwrap(),
        UploadClaim::Completed(retried_document),
        "the replay refreshed the receipt"
    );

    // A mismatched key does not keep an old receipt alive.
    let reused = key(vault, "system");
    let (_, reused_token, _) = claimed(
        receipts
            .claim(&reused, "scan", old, stale(old))
            .await
            .unwrap(),
    );
    receipts.complete(&reused, reused_token, old).await.unwrap();
    receipts
        .claim(&reused, "other content", now, stale(now))
        .await
        .unwrap();
    receipts
        .expire(sweep - UPLOAD_RECEIPT_RETENTION)
        .await
        .unwrap();
    assert!(matches!(
        receipts
            .claim(&reused, "scan", sweep, stale(sweep))
            .await
            .unwrap(),
        UploadClaim::Claimed { .. }
    ));
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

/// A store whose first `gated` puts wait for `release`, and whose first `failing` puts fail.
#[derive(Default)]
struct Gate {
    started: Notify,
    release: Notify,
    gated: AtomicUsize,
    failing: AtomicUsize,
}

/// Counts one use off `remaining`; false once none are left.
fn take(remaining: &AtomicUsize) -> bool {
    let mut current = remaining.load(Ordering::SeqCst);
    while current > 0 {
        match remaining.compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
    false
}

struct GatedStore {
    inner: ObjectStoreDocumentStore,
    gate: Arc<Gate>,
}

#[async_trait]
impl DocumentStore for GatedStore {
    fn backend(&self) -> &'static str {
        "gated"
    }
    async fn put(&self, key: &str, data: bytes::Bytes) -> Result<()> {
        if take(&self.gate.gated) {
            self.gate.started.notify_one();
            self.gate.release.notified().await;
        }
        if take(&self.gate.failing) {
            return Err(Error::Storage("object storage unavailable".into()));
        }
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

struct Flow {
    uploads: Arc<IdempotentUploads>,
    importer: Arc<Importer>,
    gate: Arc<Gate>,
}

fn flow(r: &Repositories, timing: ClaimTiming) -> Flow {
    let gate = Arc::new(Gate::default());
    let store: Arc<dyn DocumentStore> = Arc::new(GatedStore {
        inner: ObjectStoreDocumentStore::in_memory(),
        gate: gate.clone(),
    });
    let importer = Arc::new(Importer::new(
        r.documents.clone(),
        store,
        Arc::new(r.jobs.clone()),
    ));
    Flow {
        uploads: Arc::new(
            IdempotentUploads::new(
                r.upload_receipts.clone(),
                importer.clone(),
                r.documents.clone(),
            )
            .with_timing(timing),
        ),
        importer,
        gate,
    }
}

async fn documents(r: &Repositories, vault: VaultId) -> usize {
    r.documents.list_ids(vault, None).await.unwrap().len()
}

fn document(outcome: UploadOutcome) -> folio_core::models::Document {
    match outcome {
        UploadOutcome::Created(document) | UploadOutcome::Replayed(document) => document,
        other => panic!("expected a document, got {other:?}"),
    }
}

fn fingerprint(data: &str) -> String {
    folio_core::ports::upload_fingerprint("scan.pdf", "application/pdf", "Scan", data.as_bytes())
}

async fn uploads_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let f = flow(r, ClaimTiming::default());

    // A repeat returns the first document; other content with the same key conflicts.
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
    assert!(matches!(
        f.uploads
            .upload(repeat.clone(), upload("other pixels"))
            .await
            .unwrap(),
        UploadOutcome::KeyMismatch
    ));
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
    let ids: Vec<Id> = [a.unwrap().unwrap(), b.unwrap().unwrap()]
        .into_iter()
        .filter(|o| !matches!(o, UploadOutcome::InProgress))
        .map(|o| document(o).id)
        .collect();
    assert!(!ids.is_empty());
    assert!(ids.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(documents(r, vault).await, 2);

    // Dropped after the import committed, before the receipt: the retry replays and completes the receipt, so a
    // deleted document is then reported as gone instead of being created again.
    let dropped = key(vault, "user:alice");
    let now = Utc::now();
    let (reserved, _, _) = claimed(
        r.upload_receipts
            .claim(&dropped, &fingerprint("dropped"), now, stale(now))
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
    assert_eq!(
        r.upload_receipts
            .claim(&dropped, &fingerprint("dropped"), now, stale(now))
            .await
            .unwrap(),
        UploadClaim::Completed(reserved)
    );
    r.documents.delete(vault, reserved).await.unwrap();
    assert!(matches!(
        f.uploads.upload(dropped, upload("dropped")).await.unwrap(),
        UploadOutcome::Deleted
    ));
    assert_eq!(documents(r, vault).await, 2);

    // Stopped after creating the record, before storing the original: a stale retry finishes the same document.
    let unfinished = key(vault, "user:alice");
    let long_ago = Utc::now() - Duration::minutes(20);
    let (reserved, token, _) = claimed(
        r.upload_receipts
            .claim(
                &unfinished,
                &fingerprint("unfinished"),
                long_ago,
                stale(long_ago),
            )
            .await
            .unwrap(),
    );
    f.importer
        .begin_as(vault, reserved, &upload("unfinished"))
        .await
        .unwrap();
    r.upload_receipts
        .mark_created(&unfinished, token, long_ago)
        .await
        .unwrap();
    let resumed = f
        .uploads
        .upload(unfinished.clone(), upload("unfinished"))
        .await
        .unwrap();
    assert!(
        matches!(&resumed, UploadOutcome::Created(d) if d.id == reserved && d.file_path.is_some())
    );
    assert_eq!(documents(r, vault).await, 3);

    // A takeover after the reserved document was deleted reports it gone instead of creating it again.
    let resurrect = key(vault, "user:alice");
    let (reserved, token, _) = claimed(
        r.upload_receipts
            .claim(
                &resurrect,
                &fingerprint("resurrect"),
                long_ago,
                stale(long_ago),
            )
            .await
            .unwrap(),
    );
    f.importer
        .begin_as(vault, reserved, &upload("resurrect"))
        .await
        .unwrap();
    r.upload_receipts
        .mark_created(&resurrect, token, long_ago)
        .await
        .unwrap();
    r.documents.delete(vault, reserved).await.unwrap();
    assert!(matches!(
        f.uploads
            .upload(resurrect, upload("resurrect"))
            .await
            .unwrap(),
        UploadOutcome::Deleted
    ));
    assert_eq!(documents(r, vault).await, 3);

    // A failed import removes its unfinished document and lets the retry start over.
    f.gate.failing.store(1, Ordering::SeqCst);
    let failing = key(vault, "user:alice");
    assert!(f
        .uploads
        .upload(failing.clone(), upload("failing"))
        .await
        .is_err());
    assert_eq!(
        documents(r, vault).await,
        3,
        "the failed attempt leaves no document behind"
    );
    assert!(matches!(
        f.uploads.upload(failing, upload("failing")).await.unwrap(),
        UploadOutcome::Created(_)
    ));
    assert_eq!(documents(r, vault).await, 4);

    // The sweep removes the unfinished document of a receipt that expired uncompleted, and keeps finished ones.
    let expired = key(vault, "user:alice");
    let quiet = Utc::now() - UPLOAD_RECEIPT_RETENTION - Duration::hours(1);
    let (orphan, _, _) = claimed(
        r.upload_receipts
            .claim(&expired, &fingerprint("orphan"), quiet, stale(quiet))
            .await
            .unwrap(),
    );
    f.importer
        .begin_as(vault, orphan, &upload("orphan"))
        .await
        .unwrap();
    assert_eq!(f.uploads.sweep(Utc::now()).await.unwrap(), 1);
    assert!(r
        .documents
        .find_by_id(vault, orphan)
        .await
        .unwrap()
        .is_none());
    assert!(r
        .documents
        .find_by_id(vault, first.id)
        .await
        .unwrap()
        .is_some());
}

/// While an import runs, its heartbeat keeps the key from being taken over.
async fn heartbeat_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let f = flow(
        r,
        ClaimTiming {
            stale_after: Duration::milliseconds(300),
            heartbeat: StdDuration::from_millis(50),
        },
    );
    f.gate.gated.store(1, Ordering::SeqCst);
    let slow = key(vault, "user:alice");
    let running = tokio::spawn({
        let (uploads, key) = (f.uploads.clone(), slow.clone());
        async move { uploads.upload(key, upload("slow")).await }
    });
    f.gate.started.notified().await;
    tokio::time::sleep(StdDuration::from_millis(700)).await;

    let meanwhile = f
        .uploads
        .upload(slow.clone(), upload("slow"))
        .await
        .unwrap();
    assert!(
        matches!(meanwhile, UploadOutcome::InProgress),
        "got {meanwhile:?}"
    );

    f.gate.release.notify_one();
    let finished = document(running.await.unwrap().unwrap());
    let replay = f.uploads.upload(slow, upload("slow")).await.unwrap();
    assert!(matches!(&replay, UploadOutcome::Replayed(d) if d.id == finished.id));
    assert_eq!(documents(r, vault).await, 1);
}

/// Without a heartbeat in time, a retry takes the key over while the first import is still running; both finish
/// the same document and only one reports creating it.
async fn takeover_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let f = flow(
        r,
        ClaimTiming {
            stale_after: Duration::milliseconds(100),
            heartbeat: StdDuration::from_secs(60),
        },
    );
    f.gate.gated.store(1, Ordering::SeqCst);
    let slow = key(vault, "user:alice");
    let running = tokio::spawn({
        let (uploads, key) = (f.uploads.clone(), slow.clone());
        async move { uploads.upload(key, upload("slow")).await }
    });
    f.gate.started.notified().await;
    tokio::time::sleep(StdDuration::from_millis(300)).await;

    let taken_over = f
        .uploads
        .upload(slow.clone(), upload("slow"))
        .await
        .unwrap();
    f.gate.release.notify_one();
    let first = running.await.unwrap().unwrap();

    let created = [&taken_over, &first]
        .iter()
        .filter(|o| matches!(o, UploadOutcome::Created(_)))
        .count();
    assert_eq!(created, 1, "{taken_over:?} / {first:?}");
    assert_eq!(document(taken_over).id, document(first).id);
    assert_eq!(documents(r, vault).await, 1);
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
async fn sqlite_upload_receipts_reserve_renew_take_over_and_expire() {
    let (_dir, r) = sqlite().await;
    receipts_contract(&r).await;
}

#[tokio::test]
async fn sqlite_keyed_uploads_never_store_a_document_twice() {
    let (_dir, r) = sqlite().await;
    uploads_contract(&r).await;
}

#[tokio::test]
async fn sqlite_a_running_import_keeps_its_key() {
    let (_dir, r) = sqlite().await;
    heartbeat_contract(&r).await;
}

#[tokio::test]
async fn sqlite_a_takeover_during_a_slow_import_creates_one_document() {
    let (_dir, r) = sqlite().await;
    takeover_contract(&r).await;
}

#[tokio::test]
async fn postgres_upload_receipts_reserve_renew_take_over_and_expire() {
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

#[tokio::test]
async fn postgres_a_running_import_keeps_its_key() {
    let Some((_container, r)) = postgres().await else {
        return;
    };
    heartbeat_contract(&r).await;
}

#[tokio::test]
async fn postgres_a_takeover_during_a_slow_import_creates_one_document() {
    let Some((_container, r)) = postgres().await else {
        return;
    };
    takeover_contract(&r).await;
}

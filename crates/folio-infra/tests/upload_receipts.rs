mod common;

use chrono::{Duration, Utc};
use folio_core::models::Id;
use folio_core::ports::{UploadClaim, UploadReceiptKey};
use folio_infra::{Database, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let receipts = &r.upload_receipts;
    let now = Utc::now();
    let key = |scope: &str| UploadReceiptKey {
        vault,
        actor_scope: scope.into(),
        key: Id::new_v4(),
    };

    let first = key("user:alice");
    assert_eq!(
        receipts.claim(&first, "scan", now).await.unwrap(),
        UploadClaim::Claimed
    );
    assert_eq!(
        receipts.claim(&first, "scan", now).await.unwrap(),
        UploadClaim::InProgress,
        "a second request must not store the file while the first still is"
    );
    let document = Id::new_v4();
    receipts.complete(&first, document).await.unwrap();
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
    assert_eq!(
        receipts.claim(&other_caller, "scan", now).await.unwrap(),
        UploadClaim::Claimed,
        "keys are scoped to the caller"
    );

    let failed = key("system");
    receipts.claim(&failed, "scan", now).await.unwrap();
    receipts.release(&failed).await.unwrap();
    assert_eq!(
        receipts.claim(&failed, "scan", now).await.unwrap(),
        UploadClaim::Claimed,
        "a released claim can be retried"
    );
    receipts.complete(&failed, document).await.unwrap();
    receipts.release(&failed).await.unwrap();
    assert_eq!(
        receipts.claim(&failed, "scan", now).await.unwrap(),
        UploadClaim::Completed(document),
        "releasing never forgets a completed receipt"
    );

    let abandoned = key("system");
    receipts
        .claim(&abandoned, "scan", now - Duration::minutes(20))
        .await
        .unwrap();
    assert_eq!(
        receipts.claim(&abandoned, "scan", now).await.unwrap(),
        UploadClaim::Claimed,
        "an abandoned claim stops blocking its key"
    );

    let expired = key("system");
    receipts
        .claim(&expired, "scan", now - Duration::days(8))
        .await
        .unwrap();
    receipts.complete(&expired, document).await.unwrap();
    assert_eq!(
        receipts.claim(&expired, "scan", now).await.unwrap(),
        UploadClaim::Claimed,
        "receipts are kept for a week"
    );
}

#[tokio::test]
async fn sqlite_upload_receipts_replay_and_expire() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}",
        directory.path().join("receipts.db").display()
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    contract(&db.repositories().await).await;
}

#[tokio::test]
async fn postgres_upload_receipts_replay_and_expire() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    contract(&db.repositories().await).await;
}

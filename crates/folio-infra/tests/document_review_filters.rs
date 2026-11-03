mod common;

use folio_core::models::{CreateDocument, DocumentFilter, DocumentPageQuery, UpdateDocument};
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let day = "2000-02-29".parse().unwrap();
    let mut matching = Vec::new();
    for (title, date, reviewed) in [
        ("First boundary", Some(day), false),
        ("Second boundary", Some(day), false),
        ("Reviewed boundary", Some(day), true),
        ("Undated", None, false),
        ("Outside", Some("2000-03-01".parse().unwrap()), false),
    ] {
        let doc = r
            .documents
            .create(
                vault,
                CreateDocument {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        r.documents
            .update(
                vault,
                doc.id,
                UpdateDocument {
                    document_date: Some(date),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        r.documents
            .set_reviewed(vault, doc.id, reviewed)
            .await
            .unwrap();
        if title.ends_with("boundary") && !reviewed {
            matching.push(doc.id);
        }
    }
    let filter = DocumentFilter {
        reviewed: Some(false),
        document_date_from: Some(day),
        document_date_to: Some(day),
        ..Default::default()
    };
    let documents = r.documents.list(vault, Some(filter.clone())).await.unwrap();
    assert_eq!(documents.len(), 2);
    assert!(documents.iter().all(|doc| matching.contains(&doc.id)));
    let ids = r
        .documents
        .list_ids(vault, Some(filter.clone()))
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| matching.contains(id)));
    let mut query = DocumentPageQuery {
        filter,
        limit: 1,
        ..Default::default()
    };
    let first = r.documents.page(vault, query.clone()).await.unwrap();
    assert_eq!(first.documents.len(), 1);
    query.cursor = first.next_cursor;
    assert!(query.cursor.is_some());
    let second = r.documents.page(vault, query.clone()).await.unwrap();
    assert_eq!(second.documents.len(), 1);
    assert_ne!(first.documents[0].id, second.documents[0].id);
    assert!(second.next_cursor.is_none());
    query.filter.reviewed = Some(true);
    assert!(r.documents.page(vault, query).await.is_err());
    let reversed = DocumentFilter {
        document_date_from: Some("2000-03-01".parse().unwrap()),
        document_date_to: Some(day),
        ..Default::default()
    };
    assert!(r
        .documents
        .list(vault, Some(reversed.clone()))
        .await
        .is_err());
    assert!(r.documents.list_ids(vault, Some(reversed)).await.is_err());
}

#[tokio::test]
async fn sqlite_review_and_inclusive_date_filters_agree_across_reads() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("filters.db").display()
    ))
    .await
    .unwrap();
    contract(db).await;
}

#[tokio::test]
async fn postgres_review_and_inclusive_date_filters_agree_across_reads() {
    let Some(container) = common::container_or_skip(
        "review filters",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        container.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    contract(db).await;
}

//! Filtering behaviour of the SQLite document repository

use chrono::{Duration, Utc};
use folio_core::models::{CreateCorrespondent, CreateDocument, CreateTag, DocumentFilter};
use folio_core::ports::{CorrespondentRepository, DocumentRepository, TagRepository};
use folio_infra::{
    create_pool, run_migrations, SqliteCorrespondentRepository, SqliteDocumentRepository,
    SqliteTagRepository,
};

fn new_doc(title: &str, content: &str) -> CreateDocument {
    CreateDocument {
        title: title.to_string(),
        content: Some(content.to_string()),
        original_filename: None,
        mime_type: None,
        correspondent_id: None,
        document_type_id: None,
    }
}

async fn setup(dir: &tempfile::TempDir) -> sqlx::SqlitePool {
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("t.db").display());
    let pool = create_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn combines_text_tag_correspondent_and_date_filters() {
    let dir = tempfile::tempdir().unwrap();
    let pool = setup(&dir).await;
    let docs = SqliteDocumentRepository::new(pool.clone());
    let tags = SqliteTagRepository::new(pool.clone());
    let corrs = SqliteCorrespondentRepository::new(pool);

    let tag = tags
        .create(CreateTag {
            name: "tax".into(),
            color: None,
        })
        .await
        .unwrap();
    let acme = corrs
        .create(CreateCorrespondent {
            name: "Acme".into(),
        })
        .await
        .unwrap();

    let invoice = docs
        .create(new_doc("Invoice", "Total due: 100 EUR"))
        .await
        .unwrap();
    let letter = docs.create(new_doc("Letter", "Dear Sir")).await.unwrap();
    docs.add_tag(invoice.id, tag.id).await.unwrap();
    docs.update(
        invoice.id,
        folio_core::models::UpdateDocument {
            correspondent_id: Some(Some(acme.id)),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let ids =
        |v: Vec<folio_core::models::Document>| v.into_iter().map(|d| d.id).collect::<Vec<_>>();

    let by_text = docs
        .list(Some(DocumentFilter {
            search_query: Some("total due".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(ids(by_text), vec![invoice.id]);

    let by_tag = docs
        .list(Some(DocumentFilter {
            tag_ids: Some(vec![tag.id]),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(ids(by_tag), vec![invoice.id]);

    let tag_and_wrong_corr = docs
        .list(Some(DocumentFilter {
            tag_ids: Some(vec![tag.id]),
            correspondent_id: Some(uuid::Uuid::new_v4()),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert!(tag_and_wrong_corr.is_empty());

    let future = docs
        .list(Some(DocumentFilter {
            created_after: Some(Utc::now() + Duration::days(1)),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert!(future.is_empty());

    let past = docs
        .list(Some(DocumentFilter {
            created_before: Some(Utc::now() + Duration::days(1)),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(past.len(), 2);
    assert!(past.iter().any(|d| d.id == letter.id));
}

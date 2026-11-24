mod common;

use folio_core::models::{
    CreateDocument, FileIdentity, Id, RevisionPublication, RevisionPublicationSource,
};
use folio_core::ports::{keys, DocumentStore};
use folio_core::services::{replace_document_original, NewRevisionOriginal};
use folio_core::Error;
use folio_infra::queue::REVISION_DISPATCH_ENABLED;
use folio_infra::{Database, ObjectStoreDocumentStore};

async fn persistence_counts(db: &Database) -> (i64, i64, i64) {
    let sql = "SELECT (SELECT COUNT(*) FROM document_revisions),
        (SELECT COUNT(*) FROM document_files), (SELECT COUNT(*) FROM outbox_events)";
    match db {
        Database::Sqlite(pool) => sqlx::query_as(sql).fetch_one(pool).await,
        Database::Postgres(pool) => sqlx::query_as(sql).fetch_one(pool).await,
    }
    .unwrap()
}

async fn publication_contract(db: Database) {
    let vault = common::legacy_vault(&db).await;
    let other_vault = common::legacy_vault(&db).await;
    let repos = db.repositories().await;
    let documents = repos.documents;
    let document = documents
        .create_for_upload(
            vault,
            CreateDocument {
                title: "Invoice".into(),
                original_filename: Some("invoice.pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let original = FileIdentity::from_bytes(keys::original(document.id), "application/pdf", b"old");
    documents
        .finalize_verified_upload(vault, document.id, &original)
        .await
        .unwrap();
    documents
        .set_reviewed(vault, document.id, true)
        .await
        .unwrap();

    let key = Id::new_v4();
    let replacement = |candidate, bytes: &[u8]| RevisionPublication {
        key,
        expected_current: document.id,
        source: RevisionPublicationSource::Replacement {
            revision: candidate,
            filename: "revision.pdf".into(),
            identity: FileIdentity::from_bytes(
                keys::revision_original(document.id, candidate),
                "application/pdf",
                bytes,
            ),
        },
    };
    let store = ObjectStoreDocumentStore::in_memory();
    store
        .put(&original.storage_key, bytes::Bytes::from_static(b"old"))
        .await
        .unwrap();
    let upload = || NewRevisionOriginal {
        key,
        expected_current: document.id,
        filename: "revision.pdf".into(),
        mime_type: "application/pdf".into(),
        bytes: bytes::Bytes::from_static(b"new"),
    };
    let before = persistence_counts(&db).await;
    let publication =
        replace_document_original(documents.as_ref(), &store, vault, document.id, upload()).await;
    let published = if REVISION_DISPATCH_ENABLED {
        let result = publication.unwrap();
        assert!(result.created);
        assert_eq!(
            persistence_counts(&db).await,
            (before.0 + 1, before.1 + 1, before.2 + 3)
        );
        let updated = documents
            .find_by_id(vault, document.id)
            .await
            .unwrap()
            .unwrap();
        assert!(updated.reviewed_at.is_none());
        assert_eq!(updated.title, "Invoice");
        let path = keys::revision_original(document.id, result.revision_id);
        assert_eq!(updated.file_path.as_deref(), Some(path.as_str()));
        assert_eq!(store.get_bytes(&path).await.unwrap(), b"new".as_slice());
        result.revision_id
    } else {
        let candidate = Id::new_v4();
        let request = replacement(candidate, b"new");
        match documents
            .publish_revision(vault, document.id, &request)
            .await
            .unwrap_err()
        {
            Error::Conflict(message) => {
                assert_eq!(message, "File revision processing is not yet enabled")
            }
            error => panic!("expected the dispatch gate, got {error:?}"),
        }
        let unchanged = documents
            .find_by_id(vault, document.id)
            .await
            .unwrap()
            .unwrap();
        assert!(unchanged.reviewed_at.is_some());
        assert_eq!(
            unchanged.file_path.as_deref(),
            Some(original.storage_key.as_str())
        );

        assert_eq!(persistence_counts(&db).await, before);

        assert!(matches!(publication, Err(Error::Conflict(_))));
        assert_eq!(
            store
                .list(&keys::document_prefix(document.id))
                .await
                .unwrap()
                .as_slice(),
            std::slice::from_ref(&original.storage_key)
        );
        assert_eq!(persistence_counts(&db).await, before);

        let published = Id::new_v4();
        let digest = request.fingerprint();
        let id = document.id;
        let sql = sqlx::AssertSqlSafe(format!(
            "INSERT INTO document_revisions (
                 id, document_id, vault_id, number, original_file_id, created_at,
                 publication_key, publication_fingerprint
             ) VALUES ('{published}', '{id}', '{vault}', 2, '{id}', '2001-02-03T04:05:07Z',
                       '{key}', '{digest}');
             UPDATE documents SET current_revision_id = '{published}' WHERE id = '{id}';"
        ));
        match &db {
            Database::Sqlite(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
            Database::Postgres(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
        }
        .unwrap();
        published
    };
    assert_eq!(
        store.get_bytes(&original.storage_key).await.unwrap(),
        b"old".as_slice()
    );
    let original_keys = |mut keys: Vec<String>| {
        keys.sort_unstable();
        keys
    };
    let prefix = keys::document_prefix(document.id);
    let keys_before_retry = original_keys(store.list(&prefix).await.unwrap());
    assert_eq!(
        keys_before_retry.len(),
        if REVISION_DISPATCH_ENABLED { 2 } else { 1 }
    );
    let before = persistence_counts(&db).await;
    let retry = replacement(Id::new_v4(), b"new");
    let result = documents
        .publish_revision(vault, document.id, &retry)
        .await
        .unwrap();
    assert_eq!(result.revision_id, published);
    assert!(!result.created);
    assert_eq!(
        replace_document_original(documents.as_ref(), &store, vault, document.id, upload())
            .await
            .unwrap(),
        result
    );
    assert_eq!(
        original_keys(store.list(&prefix).await.unwrap()),
        keys_before_retry,
    );
    assert!(matches!(
        documents
            .publish_revision(other_vault, document.id, &retry)
            .await,
        Err(Error::NotFound { .. })
    ));
    assert!(matches!(
        documents
            .publish_revision(
                vault,
                document.id,
                &replacement(Id::new_v4(), b"other bytes")
            )
            .await,
        Err(Error::Conflict(_))
    ));
    let stale = RevisionPublication {
        key: Id::new_v4(),
        ..retry
    };
    match documents
        .publish_revision(vault, document.id, &stale)
        .await
        .unwrap_err()
    {
        Error::Conflict(message) => assert_eq!(message, "The current file revision changed"),
        error => panic!("expected a stale revision conflict, got {error:?}"),
    }
    assert_eq!(persistence_counts(&db).await, before);
}

#[tokio::test]
async fn sqlite_revision_publication_preserves_originals_and_replays_receipts() {
    let (_dir, db) = common::sqlite_database().await;
    publication_contract(db).await;
}

#[tokio::test]
async fn postgres_revision_publication_preserves_originals_and_replays_receipts() {
    let Some((_container, db)) = common::postgres_database("revision publication").await else {
        return;
    };
    publication_contract(db).await;
}

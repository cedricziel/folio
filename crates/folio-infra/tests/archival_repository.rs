mod common;

use bytes::Bytes;
use folio_core::models::{
    ArchiveCompletion, ArchiveSource, ArchiveState, ArchiveTool, CreateDocument, FileIdentity, Id,
    ValidatedArchive,
};
use folio_core::ports::{keys, purge_document, DocumentStore};
use folio_core::Error;
use folio_infra::{Database, ObjectStoreDocumentStore};

fn output(source: &ArchiveSource, attempt: Id) -> ValidatedArchive {
    ValidatedArchive {
        file: FileIdentity::from_bytes(
            keys::revision_artifact(source.document_id, source.revision_id, attempt),
            "application/pdf",
            b"validated archive fixture",
        ),
        profile: "PDF/A-2b".into(),
        converter: ArchiveTool {
            name: "converter".into(),
            version: "1.0".into(),
        },
        validator: ArchiveTool {
            name: "validator".into(),
            version: "2.0".into(),
        },
    }
}

async fn archival_contract(db: Database) {
    let vault = common::legacy_vault(&db).await;
    let other_vault = common::legacy_vault(&db).await;
    let repos = db.repositories().await;
    let documents = repos.documents;
    let archives = repos.archival;
    let id = documents
        .create_for_upload(
            vault,
            CreateDocument {
                title: "Archival source".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;
    let store = ObjectStoreDocumentStore::in_memory();
    let bytes = Bytes::from_static(b"immutable original");
    let original = FileIdentity::from_bytes(keys::original(id), "application/pdf", &bytes);
    assert!(archives.current_source(vault, id).await.unwrap().is_none());
    store
        .put(&original.storage_key, bytes.clone())
        .await
        .unwrap();
    documents
        .finalize_verified_upload(vault, id, &original)
        .await
        .unwrap();

    let source = archives.current_source(vault, id).await.unwrap().unwrap();
    assert_eq!(source.original, original);
    documents.complete_ocr(vault, id, "Grüße").await.unwrap();
    let exported = archives
        .export_document(&source, 7, 1024)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exported.content.as_deref(), Some("Grüße"));
    assert!(archives
        .export_document(&source, 6, 1024)
        .await
        .unwrap()
        .is_none());
    assert!(archives
        .export_document(&source, 7, 1)
        .await
        .unwrap()
        .is_none());

    assert!(archives
        .current_source(other_vault, id)
        .await
        .unwrap()
        .is_none());
    assert!(archives.current_attempt(vault, id).await.unwrap().is_none());
    let first = Id::new_v4();
    let wrong_source = ArchiveSource {
        original: FileIdentity::from_bytes(
            &original.storage_key,
            &original.mime_type,
            b"other bytes",
        ),
        ..source.clone()
    };
    assert!(!archives
        .enqueue_archive(&wrong_source, first, None)
        .await
        .unwrap());
    assert!(archives
        .enqueue_archive(&source, first, None)
        .await
        .unwrap());
    assert!(archives
        .enqueue_archive(&source, first, None)
        .await
        .unwrap());
    let queued = archives.current_attempt(vault, id).await.unwrap().unwrap();
    assert_eq!(queued.id, first);
    assert_eq!(queued.state, ArchiveState::Queued);

    let second = Id::new_v4();
    let competing = Id::new_v4();
    let (a, b) = tokio::join!(
        archives.enqueue_archive(&source, second, Some(first)),
        archives.enqueue_archive(&source, competing, Some(first)),
    );
    let second = match (a.unwrap(), b.unwrap()) {
        (true, false) => second,
        (false, true) => competing,
        result => panic!("expected one retry to win, got {result:?}"),
    };
    let failed = ArchiveCompletion::Failed("conversion unavailable".into());
    let skipped = ArchiveCompletion::Skipped("unsupported input".into());
    assert!(!archives
        .complete_archive(&source, first, &failed)
        .await
        .unwrap());
    assert!(!archives
        .enqueue_archive(&source, first, Some(second))
        .await
        .unwrap());
    assert!(!archives
        .complete_archive(&wrong_source, second, &failed)
        .await
        .unwrap());
    assert!(archives
        .complete_archive(&source, second, &failed)
        .await
        .unwrap());
    assert!(archives
        .complete_archive(&source, second, &failed)
        .await
        .unwrap());
    assert!(!archives
        .complete_archive(&source, second, &skipped)
        .await
        .unwrap());
    assert_eq!(
        archives
            .current_attempt(vault, id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ArchiveState::Completed(failed),
    );

    let canceled = Id::new_v4();
    assert!(archives
        .enqueue_archive(&source, canceled, Some(second))
        .await
        .unwrap());
    documents.delete(vault, id).await.unwrap();
    assert!(archives.current_attempt(vault, id).await.unwrap().is_none());
    assert!(!archives
        .complete_archive(&source, canceled, &skipped)
        .await
        .unwrap());
    documents.restore(vault, id).await.unwrap();
    assert!(!archives
        .complete_archive(&source, canceled, &skipped)
        .await
        .unwrap());
    assert_eq!(
        archives
            .current_attempt(vault, id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ArchiveState::Completed(ArchiveCompletion::Skipped("Document trashed".into())),
    );

    let third = Id::new_v4();
    assert!(archives
        .enqueue_archive(&source, third, Some(canceled))
        .await
        .unwrap());
    let output = output(&source, third);
    let mut invalid = output.clone();
    invalid.file = original.clone();
    assert!(matches!(
        archives
            .complete_archive(&source, third, &ArchiveCompletion::Ready(invalid))
            .await,
        Err(Error::Validation(_))
    ));
    let mut invalid = output.clone();
    invalid.validator.version = " ".into();
    assert!(matches!(
        archives
            .complete_archive(&source, third, &ArchiveCompletion::Ready(invalid))
            .await,
        Err(Error::Validation(_))
    ));
    let mut invalid = output.clone();
    invalid.profile = "PDF 1.7".into();
    assert!(matches!(
        archives
            .complete_archive(&source, third, &ArchiveCompletion::Ready(invalid))
            .await,
        Err(Error::Validation(_))
    ));
    assert!(store
        .put_if_absent(
            &output.file.storage_key,
            Bytes::from_static(b"validated archive fixture"),
        )
        .await
        .unwrap());
    let ready = ArchiveCompletion::Ready(output.clone());
    assert!(archives
        .complete_archive(&source, third, &ready)
        .await
        .unwrap());
    assert!(archives
        .complete_archive(&source, third, &ready)
        .await
        .unwrap());
    assert_eq!(
        archives
            .current_attempt(vault, id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ArchiveState::Completed(ready.clone()),
    );
    assert!(archives
        .current_attempt(other_vault, id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .get(&original.storage_key)
            .await
            .unwrap()
            .into_bytes()
            .await
            .unwrap(),
        bytes
    );

    // The replacement dispatch gate is still closed. Set up the same-checksum
    // case by creating a new revision that selects the already verified file.
    let replacement = Id::new_v4();
    let sql = sqlx::AssertSqlSafe(format!(
        "INSERT INTO document_revisions (
            id, document_id, vault_id, number, original_file_id, created_at, restored_from_revision_id
         ) VALUES ('{replacement}', '{id}', '{vault}', 2, '{id}', '2001-02-03T04:05:06Z', '{id}');
         UPDATE documents SET current_revision_id = '{replacement}' WHERE id = '{id}';"
    ));
    match &db {
        Database::Sqlite(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
    }
    .unwrap();
    assert!(archives.current_attempt(vault, id).await.unwrap().is_none());
    assert!(!archives
        .complete_archive(&source, third, &ready)
        .await
        .unwrap());
    assert!(!archives
        .enqueue_archive(&source, Id::new_v4(), Some(third))
        .await
        .unwrap());
    assert!(archives
        .retains_archive(id, source.revision_id, third)
        .await
        .unwrap());
    // A rejected replay does not authorize removing the previously committed bytes.
    assert_eq!(
        store
            .get(&output.file.storage_key)
            .await
            .unwrap()
            .into_bytes()
            .await
            .unwrap(),
        Bytes::from_static(b"validated archive fixture")
    );
    let new_source = archives.current_source(vault, id).await.unwrap().unwrap();
    assert_eq!(new_source.revision_id, replacement);
    assert_eq!(new_source.original, source.original);

    // Move schedules processing. Restore the initial fixture revision before
    // exercising lifecycle behavior: dispatch of replacement revisions is gated.
    let sql = sqlx::AssertSqlSafe(format!(
        "UPDATE documents SET current_revision_id = '{id}' WHERE id = '{id}'"
    ));
    match &db {
        Database::Sqlite(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
    }
    .unwrap();
    let new_source = archives.current_source(vault, id).await.unwrap().unwrap();
    assert_eq!(new_source, source);

    let pending = Id::new_v4();
    assert!(archives
        .enqueue_archive(&new_source, pending, Some(third))
        .await
        .unwrap());
    documents
        .move_to(id, vault, other_vault, None)
        .await
        .unwrap();
    assert!(archives.current_attempt(vault, id).await.unwrap().is_none());
    assert!(!archives
        .complete_archive(&new_source, pending, &skipped)
        .await
        .unwrap());
    let moved = archives
        .current_attempt(other_vault, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        moved.state,
        ArchiveState::Completed(ArchiveCompletion::Skipped("Document moved".into()))
    );
    // Restoring a vault must not resurrect work admitted before its deletion.
    let vault_pending = Id::new_v4();
    assert!(archives
        .enqueue_archive(&moved.source, vault_pending, Some(moved.id))
        .await
        .unwrap());
    repos.vaults.delete(other_vault).await.unwrap();
    assert!(archives
        .current_attempt(other_vault, id)
        .await
        .unwrap()
        .is_none());
    repos.vaults.restore(other_vault).await.unwrap();
    assert!(!archives
        .complete_archive(&moved.source, vault_pending, &skipped)
        .await
        .unwrap());
    assert_eq!(
        archives
            .current_attempt(other_vault, id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ArchiveState::Completed(ArchiveCompletion::Skipped("Vault deleted".into()))
    );

    let final_attempt = Id::new_v4();
    assert!(archives
        .enqueue_archive(&moved.source, final_attempt, Some(vault_pending))
        .await
        .unwrap());
    assert!(archives
        .complete_archive(&moved.source, final_attempt, &skipped)
        .await
        .unwrap());
    assert_eq!(
        archives
            .current_attempt(other_vault, id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ArchiveState::Completed(skipped),
    );

    documents.delete(other_vault, id).await.unwrap();
    documents.request_purge(other_vault, id).await.unwrap();
    assert!(!archives
        .enqueue_archive(&moved.source, Id::new_v4(), Some(final_attempt))
        .await
        .unwrap());
    purge_document(&store, id).await.unwrap();
    documents.finalize_purge(other_vault, id).await.unwrap();
    assert!(!archives
        .retains_archive(id, source.revision_id, third)
        .await
        .unwrap());
    assert!(store
        .list(&keys::document_prefix(id))
        .await
        .unwrap()
        .is_empty());
    let remaining: i64 = match &db {
        Database::Sqlite(pool) => {
            sqlx::query_scalar("SELECT count(*) FROM archival_attempts")
                .fetch_one(pool)
                .await
        }
        Database::Postgres(pool) => {
            sqlx::query_scalar("SELECT count(*) FROM archival_attempts")
                .fetch_one(pool)
                .await
        }
    }
    .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn sqlite_archival_repository_contract() {
    let (_dir, db) = common::sqlite_database().await;
    archival_contract(db).await;
}

#[tokio::test]
async fn postgres_archival_repository_contract() {
    let Some((_container, db)) = common::postgres_database("archival repository").await else {
        return;
    };
    archival_contract(db).await;
}

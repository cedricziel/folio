mod common;

use chrono::{Duration, SubsecRound, Utc};
use folio_core::models::{
    CreateCorrespondent, CreateDocumentType, CreateTag, DocumentStatus, EntryBody, EntryKind,
    EntryPageQuery, FileIdentity, Id, ImportedDocument, ImportedNote, MetadataFieldInput,
    MetadataKind, MetadataValue, ProcessingStage, ProcessingStatus, ReviewOrigin, SourceKind,
    UserId,
};
use folio_core::mutation::with_actor;
use folio_core::ports::{MetadataFieldRepository, MetadataValueRepository};
use folio_core::{Error, EventOrigin};
use folio_infra::database::{
    postgres::{PostgresMetadataFieldRepository, PostgresMetadataValueRepository},
    sqlite::{SqliteMetadataFieldRepository, SqliteMetadataValueRepository},
};
use folio_infra::{Database, JobType};

/// Breaks an import given the ASN and a retired field
type Spoil = fn(&mut ImportedDocument, Id, Id);

async fn note_external_id(db: &Database, id: Id) -> Option<String> {
    let sql = "SELECT external_id FROM document_entries WHERE id = $1";
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar(sql)
            .bind(id.to_string())
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap(),
    }
}

async fn original_revision_identity(db: &Database, id: Id) -> (String, String, i64, String, i64) {
    let sql = "SELECT f.storage_key, f.sha256, f.byte_size, f.verification_state, r.number
               FROM documents d
               JOIN document_revisions r ON r.id = d.current_revision_id
               JOIN document_files f ON f.id = r.original_file_id
               WHERE d.id = $1";
    match db {
        Database::Sqlite(pool) => sqlx::query_as(sql)
            .bind(id.to_string())
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_as(sql).bind(id).fetch_one(pool).await.unwrap(),
    }
}

async fn execute_failure_trigger(db: &Database) {
    let (sqlite, postgres) = (
        "CREATE TRIGGER refuse_import_embedding BEFORE INSERT ON jobs
         WHEN NEW.job_type = 'generate_embedding' BEGIN SELECT RAISE(ABORT, 'refused'); END",
        "CREATE FUNCTION refuse_import_embedding() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.job_type = 'generate_embedding' THEN RAISE EXCEPTION 'refused'; END IF;
         RETURN NEW; END $$;
         CREATE TRIGGER refuse_import_embedding BEFORE INSERT ON jobs
         FOR EACH ROW EXECUTE FUNCTION refuse_import_embedding()",
    );
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlite)
            .execute(pool)
            .await
            .map(|_| ())
            .unwrap(),
        Database::Postgres(pool) => sqlx::raw_sql(postgres)
            .execute(pool)
            .await
            .map(|_| ())
            .unwrap(),
    };
}

async fn imported_storage_counts(db: &Database, id: Id) -> (i64, i64, i64) {
    let sql = "SELECT
                 (SELECT COUNT(*) FROM documents WHERE id = $1),
                 (SELECT COUNT(*) FROM document_files WHERE document_id = $1),
                 (SELECT COUNT(*) FROM document_revisions WHERE document_id = $1)";
    match db {
        Database::Sqlite(pool) => sqlx::query_as(sql)
            .bind(id.to_string())
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_as(sql).bind(id).fetch_one(pool).await.unwrap(),
    }
}

async fn contract(db: Database) {
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let (fields, values): (
        Box<dyn MetadataFieldRepository>,
        Box<dyn MetadataValueRepository>,
    ) = match &db {
        Database::Sqlite(pool) => (
            Box::new(SqliteMetadataFieldRepository::new(pool.clone())),
            Box::new(SqliteMetadataValueRepository::new(pool.clone())),
        ),
        Database::Postgres(pool) => (
            Box::new(PostgresMetadataFieldRepository::new(pool.clone())),
            Box::new(PostgresMetadataValueRepository::new(pool.clone())),
        ),
    };
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "Utilities".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let correspondent = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Power Co".into(),
            },
        )
        .await
        .unwrap();
    let kind = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Bill".into(),
            },
        )
        .await
        .unwrap();
    let asn = fields
        .create(
            vault,
            MetadataFieldInput {
                name: "ASN".into(),
                kind: MetadataKind::Integer,
                required: false,
                unique: true,
            },
        )
        .await
        .unwrap();

    let added = (Utc::now() - Duration::days(2000)).trunc_subsecs(6);
    let noted = added + Duration::hours(5);
    let mut document = common::imported_document(vault, "7", added);
    let original = b"verified Paperless original";
    let identity =
        FileIdentity::from_bytes(document.file_path.clone(), "application/pdf", original);
    document.file_size = identity.byte_size;
    document.content_sha256 = identity.sha256.clone();
    document.correspondent_id = Some(correspondent.id);
    document.document_type_id = Some(kind.id);
    document.tag_ids = vec![tag.id];
    document
        .metadata
        .insert(asn.id, MetadataValue::Integer("1042".into()));
    document.notes = vec![ImportedNote {
        external_id: "31".into(),
        body: EntryBody::parse("Paid by transfer".into()).unwrap(),
        created_at: noted,
    }];
    let admin = UserId("admin".into());
    let created = with_actor(
        Some(admin.clone()),
        r.documents.create_imported(vault, document.clone()),
    )
    .await
    .unwrap();
    assert_eq!(created.id, document.id);

    let read = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.title, "Electricity bill");
    assert_eq!(read.content.as_deref(), Some("Total due 42.00"));
    assert_eq!(read.document_date, document.document_date);
    assert_eq!(read.created_at, added);
    assert_eq!(read.updated_at, added + Duration::days(2));
    assert_eq!(read.correspondent_id, Some(correspondent.id));
    assert_eq!(read.document_type_id, Some(kind.id));
    assert_eq!(read.file_path.as_deref(), Some("documents/7/original"));
    assert_eq!(read.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(read.original_filename.as_deref(), Some("bill.pdf"));
    assert_eq!(read.source.kind, SourceKind::Import);
    assert_eq!(read.file_size, Some(identity.byte_size));
    assert_eq!(
        original_revision_identity(&db, document.id).await,
        (
            identity.storage_key.clone(),
            identity.sha256.clone(),
            identity.byte_size as i64,
            "verified".into(),
            1,
        )
    );
    assert_eq!(read.created_by, Some(admin.clone()));
    assert_eq!(read.ocr_status, ProcessingStatus::Done);
    assert_eq!(read.embed_status, ProcessingStatus::Pending);
    assert_eq!(read.review_origin, Some(ReviewOrigin::System));
    assert_eq!(read.reviewed_by, None);
    assert_eq!(
        r.documents
            .find_by_external_ref(vault, &document.external_ref)
            .await
            .unwrap(),
        Some(document.id)
    );
    assert_eq!(read.status(), DocumentStatus::Processing);
    let job = r.jobs.dequeue().await.unwrap().expect("an embedding job");
    assert!(
        matches!(
            job.job_type,
            JobType::GenerateEmbedding { document_id, origin: EventOrigin::Import }
                if document_id == document.id
        ),
        "{:?}",
        job.job_type
    );
    r.jobs.complete(&job).await.unwrap();
    r.documents
        .set_stage_status(
            vault,
            document.id,
            ProcessingStage::Embed,
            ProcessingStatus::Done,
        )
        .await
        .unwrap();
    let embedded = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(embedded.status(), DocumentStatus::Filed);
    let tags = r.documents.get_tags(vault, document.id).await.unwrap();
    assert_eq!(tags.iter().map(|t| t.id).collect::<Vec<_>>(), vec![tag.id]);
    let metadata = values.get(vault, document.id).await.unwrap();
    assert!(metadata.revision.is_some());
    assert_eq!(metadata.values.len(), 1);
    assert_eq!(
        metadata.values[0].value,
        Some(MetadataValue::Integer("1042".into()))
    );
    let notes = r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind: EntryKind::Note,
                limit: 10,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(notes.entries.len(), 1);
    assert_eq!(notes.entries[0].body, "Paid by transfer");
    assert_eq!(notes.entries[0].created_at, noted);
    assert_eq!(notes.entries[0].created_by, Some(admin.clone()));
    assert_eq!(
        note_external_id(&db, notes.entries[0].id).await.as_deref(),
        Some("31")
    );

    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::minutes(5), 100)
        .await
        .unwrap();
    let revision_events = events
        .iter()
        .filter(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|envelope| envelope.event_type() == "document.revision_created")
        })
        .collect::<Vec<_>>();
    assert_eq!(revision_events.len(), 1);
    assert_eq!(revision_events[0].origin, EventOrigin::Import);
    let created_events = events
        .iter()
        .filter(|e| {
            matches!(&e.event, Some(folio_core::DocumentEvent::Created { document_id }) if *document_id == document.id)
        })
        .map(|e| e.origin)
        .collect::<Vec<_>>();
    assert_eq!(created_events, vec![EventOrigin::Import]);

    let again = r.documents.create_imported(vault, document.clone()).await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
    let mut renamed = document.clone();
    renamed.id = Id::new_v4();
    let duplicate_ref = r.documents.create_imported(vault, renamed).await;
    assert!(
        matches!(duplicate_ref, Err(Error::Conflict(_))),
        "{duplicate_ref:?}"
    );
    let all = r.documents.list(vault, None).await.unwrap();
    assert_eq!(all.len(), 1);

    let mut inbox = common::imported_document(vault, "8", added);
    inbox.content = None;
    inbox.in_inbox = true;
    let inbox = r.documents.create_imported(vault, inbox).await.unwrap();
    assert_eq!(inbox.ocr_status, ProcessingStatus::Done);
    assert_eq!(inbox.embed_status, ProcessingStatus::Skipped);
    assert_eq!(inbox.reviewed_at, None);
    assert_eq!(inbox.status(), DocumentStatus::NeedsReview);
    assert!(r.jobs.dequeue().await.unwrap().is_none());

    let retired = fields
        .create(
            vault,
            MetadataFieldInput {
                name: "Old".into(),
                kind: MetadataKind::Text,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    fields.set_deleted(vault, retired.id, true).await.unwrap();
    let rejected: Vec<(&str, Spoil)> = vec![
        ("bad value", |d, asn, _| {
            d.metadata
                .insert(asn, MetadataValue::Integer("many".into()));
        }),
        ("taken ASN", |d, asn, _| {
            d.metadata
                .insert(asn, MetadataValue::Integer("1042".into()));
        }),
        ("retired field", |d, _, retired| {
            d.metadata.insert(retired, MetadataValue::Text("x".into()));
        }),
        ("duplicate notes", |d, _, _| {
            let note = ImportedNote {
                external_id: "1".into(),
                body: EntryBody::parse("twice".into()).unwrap(),
                created_at: Utc::now(),
            };
            d.notes = vec![note.clone(), note];
        }),
    ];
    for (n, (case, spoil)) in rejected.into_iter().enumerate() {
        let mut document = common::imported_document(vault, &format!("9{n}"), added);
        spoil(&mut document, asn.id, retired.id);
        let result = r.documents.create_imported(vault, document.clone()).await;
        assert!(
            matches!(result, Err(Error::Validation(_))),
            "{case}: {result:?}"
        );
        assert_eq!(
            r.documents
                .find_by_external_ref(vault, &document.external_ref)
                .await
                .unwrap(),
            None,
            "{case}"
        );
        assert!(r
            .documents
            .locate_retained(document.id)
            .await
            .unwrap()
            .is_none());
        let announced = r
            .outbox
            .replay(0, Utc::now() - Duration::minutes(5), 500)
            .await
            .unwrap()
            .into_iter()
            .any(|e| {
                e.event
                    .as_ref()
                    .is_some_and(|e| e.document_id() == document.id)
            });
        assert!(!announced, "{case}");
    }
    assert!(r.jobs.dequeue().await.unwrap().is_none());

    execute_failure_trigger(&db).await;
    let failed = common::imported_document(vault, "transaction-failure", added);
    let failed_id = failed.id;
    assert!(r
        .documents
        .create_imported(vault, failed.clone())
        .await
        .is_err());
    assert_eq!(imported_storage_counts(&db, failed_id).await, (0, 0, 0));
    assert_eq!(
        r.documents
            .find_by_external_ref(vault, &failed.external_ref)
            .await
            .unwrap(),
        None
    );
    let failed_events = r
        .outbox
        .replay(0, Utc::now() - Duration::minutes(5), 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| {
            event
                .event
                .as_ref()
                .is_some_and(|event| event.document_id() == failed_id)
                || event
                    .envelope
                    .as_ref()
                    .and_then(|envelope| envelope.document_id())
                    == Some(failed_id)
        });
    assert_eq!(failed_events.count(), 0);
    assert!(r.jobs.dequeue().await.unwrap().is_none());
}

#[tokio::test]
async fn sqlite_imported_documents_keep_their_history_in_one_write() {
    let (_dir, db) = common::sqlite_database().await;
    contract(db).await;
}

#[tokio::test]
async fn postgres_imported_documents_keep_their_history_in_one_write() {
    let Some((_container, db)) = common::postgres_database("imported documents").await else {
        return;
    };
    contract(db).await;
}

mod common;

use folio_core::events::decode_stored_outbox_payload;
use folio_core::models::CreateDocument;
use folio_core::ports::EmbeddedChunk;
use folio_core::processing_events::{ProcessingEventStage, ProcessingReason};
use folio_core::EventOrigin;
use folio_infra::{Database, JobType};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn execute(db: &Database, sql: String) {
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .unwrap();
}

async fn events(db: &Database) -> Vec<folio_core::events::DecodedOutboxPayload> {
    let payloads: Vec<String> = match db {
        Database::Sqlite(pool) => {
            sqlx::query_scalar("SELECT payload FROM outbox_events ORDER BY id")
                .fetch_all(pool)
                .await
                .unwrap()
        }
        Database::Postgres(pool) => {
            sqlx::query_scalar("SELECT payload::text FROM outbox_events ORDER BY id")
                .fetch_all(pool)
                .await
                .unwrap()
        }
    };
    payloads
        .iter()
        .map(|payload| decode_stored_outbox_payload(payload).unwrap())
        .collect()
}

async fn count_chunks(db: &Database) -> i64 {
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar("SELECT COUNT(*) FROM document_chunks")
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar("SELECT COUNT(*) FROM document_chunks")
            .fetch_one(pool)
            .await
            .unwrap(),
    }
}

async fn refuse_outbox(db: &Database) {
    let sql = match db {
        Database::Sqlite(_) => "CREATE TRIGGER refuse_derived_outbox BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END",
        Database::Postgres(_) => "CREATE FUNCTION refuse_derived_outbox() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql; CREATE TRIGGER refuse_derived_outbox BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_derived_outbox()",
    };
    execute(db, sql.into()).await;
}

async fn allow_outbox(db: &Database) {
    let sql = match db {
        Database::Sqlite(_) => "DROP TRIGGER refuse_derived_outbox",
        Database::Postgres(_) => "DROP TRIGGER refuse_derived_outbox ON outbox_events; DROP FUNCTION refuse_derived_outbox()",
    };
    execute(db, sql.into()).await;
}

async fn stored_chunk_model(db: &Database) -> String {
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar("SELECT model FROM document_chunks LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar("SELECT model FROM document_chunks LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap(),
    }
}

async fn contract(db: &Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let vault = repos
        .vaults
        .create("Derived stages", None)
        .await
        .unwrap()
        .id;
    let doc = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Synthetic document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;
    repos
        .jobs
        .enqueue(JobType::GenerateEmbedding {
            document_id: doc,
            origin: EventOrigin::Import,
        })
        .await
        .unwrap();
    execute(
        db,
        format!("UPDATE documents SET content = 'Synthetic text' WHERE id = '{doc}'"),
    )
    .await;
    let job = repos.jobs.dequeue().await.unwrap().unwrap();
    execute(db, "DELETE FROM outbox_events".into()).await;
    let fence = repos
        .jobs
        .start_derived(&job, ProcessingEventStage::Embedding)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repos
            .jobs
            .start_derived(&job, ProcessingEventStage::Embedding)
            .await
            .unwrap(),
        Some(fence.clone())
    );
    assert_eq!(events(db).await.len(), 1);
    assert_eq!(events(db).await[0].origin, EventOrigin::Import);
    assert!(!repos
        .jobs
        .complete_ocr(&fence, "must not become OCR", true)
        .await
        .unwrap());
    assert!(!repos.jobs.skip_ocr(&fence, true).await.unwrap());
    assert_eq!(events(db).await.len(), 1);
    let chunks = [EmbeddedChunk {
        index: 0,
        text: "Synthetic text".into(),
        embedding: vec![0.5, 0.25],
    }];
    assert!(repos
        .jobs
        .complete_embedding(&fence, "Synthetic text", "test-model", &chunks)
        .await
        .unwrap());
    assert!(!repos
        .jobs
        .complete_embedding(&fence, "Synthetic text", "test-model", &chunks)
        .await
        .unwrap());
    assert!(!repos
        .jobs
        .complete_embedding(&fence, "obsolete input", "", &[])
        .await
        .unwrap());
    assert_eq!(count_chunks(db).await, 1);
    let completed = events(db).await;
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[1].origin, EventOrigin::Import);
    assert_eq!(
        completed[1].envelope.as_ref().unwrap().event_type(),
        "document.processing.embedding.completed"
    );
    repos.jobs.complete(&job).await.unwrap();

    repos
        .jobs
        .enqueue(JobType::GenerateEmbedding {
            document_id: doc,
            origin: EventOrigin::Import,
        })
        .await
        .unwrap();
    let expired = repos.jobs.dequeue().await.unwrap().unwrap();
    let expired_fence = repos
        .jobs
        .start_derived(&expired, ProcessingEventStage::Embedding)
        .await
        .unwrap()
        .unwrap();
    let before_rollback = events(db).await.len();
    refuse_outbox(db).await;
    assert!(repos
        .jobs
        .complete_embedding(
            &expired_fence,
            "Synthetic text",
            "replacement-model",
            &chunks,
        )
        .await
        .is_err());
    assert_eq!(stored_chunk_model(db).await, "test-model");
    assert_eq!(
        repos
            .documents
            .find_by_id(vault, doc)
            .await
            .unwrap()
            .unwrap()
            .embed_status,
        folio_core::models::ProcessingStatus::Running
    );
    let state_sql = format!(
        "SELECT state FROM processing_stage_runs WHERE job_id = '{}'",
        expired.id
    );
    let running: String = match db {
        Database::Sqlite(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(state_sql))
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(state_sql))
            .fetch_one(pool)
            .await
            .unwrap(),
    };
    assert_eq!(running, "running");
    assert_eq!(events(db).await.len(), before_rollback);
    allow_outbox(db).await;
    execute(
        db,
        format!("UPDATE documents SET content = 'Changed concurrently' WHERE id = '{doc}'"),
    )
    .await;
    assert!(repos
        .jobs
        .complete_embedding(
            &expired_fence,
            "Synthetic text",
            "replacement-model",
            &chunks
        )
        .await
        .is_err());
    assert_eq!(stored_chunk_model(db).await, "test-model");
    assert_eq!(events(db).await.len(), before_rollback);
    execute(
        db,
        format!("UPDATE documents SET content = 'Synthetic text' WHERE id = '{doc}'"),
    )
    .await;
    let destination = repos
        .vaults
        .create("Moved destination", None)
        .await
        .unwrap()
        .id;
    execute(
        db,
        format!("UPDATE documents SET vault_id = '{destination}' WHERE id = '{doc}'"),
    )
    .await;
    let before_moved = events(db).await.len();
    assert!(!repos
        .jobs
        .complete_embedding(
            &expired_fence,
            "Synthetic text",
            "replacement-model",
            &chunks,
        )
        .await
        .unwrap());
    assert_eq!(stored_chunk_model(db).await, "test-model");
    assert_eq!(events(db).await.len(), before_moved);
    execute(db, format!("UPDATE documents SET vault_id = '{vault}', deleted_at = '2026-11-03T00:00:00Z' WHERE id = '{doc}'")).await;
    assert!(!repos
        .jobs
        .complete_embedding(
            &expired_fence,
            "Synthetic text",
            "replacement-model",
            &chunks,
        )
        .await
        .unwrap());
    assert_eq!(events(db).await.len(), before_moved);
    execute(
        db,
        format!("UPDATE documents SET deleted_at = NULL WHERE id = '{doc}'"),
    )
    .await;
    execute(
        db,
        format!(
            "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'",
            expired.id
        ),
    )
    .await;
    let before = events(db).await.len();
    assert!(!repos
        .jobs
        .complete_embedding(&expired_fence, "Synthetic text", "other-model", &chunks)
        .await
        .unwrap());
    assert_eq!(events(db).await.len(), before);
    assert_eq!(count_chunks(db).await, 1);
    repos.jobs.reset_stuck().await.unwrap();
    let retried = events(db).await;
    assert_eq!(retried.last().unwrap().origin, EventOrigin::Import);
    assert_eq!(
        retried
            .last()
            .unwrap()
            .envelope
            .as_ref()
            .unwrap()
            .event_type(),
        "document.processing.embedding.retry_scheduled"
    );
    execute(
        db,
        format!(
            "UPDATE jobs SET status = 'completed' WHERE id = '{}'",
            expired.id
        ),
    )
    .await;

    repos
        .jobs
        .enqueue(JobType::generate_embedding(doc))
        .await
        .unwrap();
    execute(
        db,
        format!("UPDATE documents SET content = '' WHERE id = '{doc}'"),
    )
    .await;
    let empty_job = repos.jobs.dequeue().await.unwrap().unwrap();
    let empty = repos
        .jobs
        .start_derived(&empty_job, ProcessingEventStage::Embedding)
        .await
        .unwrap()
        .unwrap();
    execute(
        db,
        format!(
            "UPDATE documents SET content = 'New text before no-text commit' WHERE id = '{doc}'"
        ),
    )
    .await;
    let before_empty_race = events(db).await.len();
    assert!(repos
        .jobs
        .skip_derived(&empty, ProcessingReason::NoText)
        .await
        .is_err());
    assert_eq!(count_chunks(db).await, 1);
    assert_eq!(events(db).await.len(), before_empty_race);
    execute(
        db,
        format!("UPDATE documents SET content = '   ' WHERE id = '{doc}'"),
    )
    .await;
    refuse_outbox(db).await;
    assert!(repos
        .jobs
        .skip_derived(&empty, ProcessingReason::NoText)
        .await
        .is_err());
    assert_eq!(count_chunks(db).await, 1);
    allow_outbox(db).await;
    assert!(repos
        .jobs
        .skip_derived(&empty, ProcessingReason::NoText)
        .await
        .unwrap());
    assert_eq!(count_chunks(db).await, 0);
    repos.jobs.complete(&empty_job).await.unwrap();

    repos
        .jobs
        .enqueue(JobType::GenerateThumbnail { document_id: doc })
        .await
        .unwrap();
    let thumbnail = repos.jobs.dequeue().await.unwrap().unwrap();
    let preview = repos
        .jobs
        .start_derived(&thumbnail, ProcessingEventStage::Thumbnail)
        .await
        .unwrap()
        .unwrap();
    refuse_outbox(db).await;
    assert!(repos
        .jobs
        .complete_thumbnail(&preview, Some(9))
        .await
        .is_err());
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc)
            .await
            .unwrap(),
        Some(folio_core::ports::ThumbnailLocation::Legacy)
    );
    assert_eq!(
        repos
            .documents
            .find_by_id(vault, doc)
            .await
            .unwrap()
            .unwrap()
            .page_count,
        None
    );
    allow_outbox(db).await;
    let before_trashed_preview = events(db).await.len();
    execute(
        db,
        format!("UPDATE documents SET deleted_at = '2026-11-03T00:00:00Z' WHERE id = '{doc}'"),
    )
    .await;
    assert!(!repos
        .jobs
        .complete_thumbnail(&preview, Some(9))
        .await
        .unwrap());
    assert_eq!(events(db).await.len(), before_trashed_preview);
    execute(
        db,
        format!("UPDATE documents SET deleted_at = NULL WHERE id = '{doc}'"),
    )
    .await;
    execute(
        db,
        format!("UPDATE vaults SET deleted_at = '2026-11-03T00:00:00Z' WHERE id = '{vault}'"),
    )
    .await;
    assert!(!repos
        .jobs
        .complete_thumbnail(&preview, Some(9))
        .await
        .unwrap());
    assert_eq!(events(db).await.len(), before_trashed_preview);
    execute(
        db,
        format!("UPDATE vaults SET deleted_at = NULL WHERE id = '{vault}'"),
    )
    .await;
    let expiry_trigger = match db {
        Database::Sqlite(_) => format!("CREATE TRIGGER expire_derived_claim BEFORE INSERT ON outbox_events BEGIN UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'; END", thumbnail.id),
        Database::Postgres(_) => format!("CREATE FUNCTION expire_derived_claim() RETURNS trigger AS $$ BEGIN UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'; RETURN NEW; END $$ LANGUAGE plpgsql; CREATE TRIGGER expire_derived_claim BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION expire_derived_claim()", thumbnail.id),
    };
    execute(db, expiry_trigger).await;
    let before_expiry = events(db).await.len();
    assert!(!repos
        .jobs
        .complete_thumbnail(&preview, Some(9))
        .await
        .unwrap());
    assert_eq!(events(db).await.len(), before_expiry);
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc)
            .await
            .unwrap(),
        Some(folio_core::ports::ThumbnailLocation::Legacy)
    );
    assert_eq!(
        repos
            .documents
            .find_by_id(vault, doc)
            .await
            .unwrap()
            .unwrap()
            .page_count,
        None
    );
    let drop_expiry = match db {
        Database::Sqlite(_) => "DROP TRIGGER expire_derived_claim",
        Database::Postgres(_) => "DROP TRIGGER expire_derived_claim ON outbox_events; DROP FUNCTION expire_derived_claim()",
    };
    execute(db, drop_expiry.into()).await;
    assert!(repos
        .jobs
        .complete_thumbnail(&preview, Some(2))
        .await
        .unwrap());
    assert!(!repos
        .jobs
        .complete_thumbnail(&preview, Some(9))
        .await
        .unwrap());
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc)
            .await
            .unwrap(),
        Some(folio_core::ports::ThumbnailLocation::Committed(
            preview.result_prefix()
        ))
    );
    assert_eq!(
        repos
            .documents
            .find_by_id(vault, doc)
            .await
            .unwrap()
            .unwrap()
            .page_count,
        Some(2)
    );
    repos.jobs.complete(&thumbnail).await.unwrap();

    repos
        .jobs
        .enqueue(JobType::GenerateThumbnail { document_id: doc })
        .await
        .unwrap();
    let skip = repos.jobs.dequeue().await.unwrap().unwrap();
    let skip_fence = repos
        .jobs
        .start_derived(&skip, ProcessingEventStage::Thumbnail)
        .await
        .unwrap()
        .unwrap();
    assert!(!repos
        .jobs
        .skip_derived(&skip_fence, ProcessingReason::NoText)
        .await
        .unwrap());
    assert!(repos
        .jobs
        .skip_derived(&skip_fence, ProcessingReason::UnsupportedFormat,)
        .await
        .unwrap());
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc)
            .await
            .unwrap(),
        Some(folio_core::ports::ThumbnailLocation::Committed(
            preview.result_prefix()
        ))
    );

    legacy_claim_is_fenced_by_replacement(db).await;
}

async fn legacy_claim_is_fenced_by_replacement(db: &Database) {
    execute(db, "DELETE FROM jobs; DELETE FROM outbox_events;".into()).await;
    let repos = db.repositories().await;
    let vault = repos
        .vaults
        .create("Revision fencing", None)
        .await
        .unwrap()
        .id;
    let doc = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Replaced".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;
    let file = uuid::Uuid::new_v4();
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    execute(db, format!(
        "INSERT INTO document_files (id, document_id, vault_id, role, storage_key, mime_type, verification_state)
         VALUES ('{file}', '{doc}', '{vault}', 'original', 'synthetic-original', 'application/pdf', 'pending');
         INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at)
         VALUES ('{first}', '{doc}', '{vault}', 1, '{file}', '2026-11-22T00:00:00Z');
         UPDATE documents SET current_revision_id = '{first}' WHERE id = '{doc}';"
    )).await;
    repos
        .jobs
        .enqueue(JobType::GenerateThumbnail { document_id: doc })
        .await
        .unwrap();
    let job = repos.jobs.dequeue().await.unwrap().unwrap();
    let fence = repos
        .jobs
        .start_derived(&job, ProcessingEventStage::Thumbnail)
        .await
        .unwrap()
        .expect("revision one accepts a legacy job");
    let before = events(db).await.len();

    execute(db, format!(
        "INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at)
         VALUES ('{second}', '{doc}', '{vault}', 2, '{file}', '2026-11-22T00:01:00Z');
         UPDATE documents SET current_revision_id = '{second}' WHERE id = '{doc}';"
    )).await;

    assert!(repos
        .jobs
        .start_derived(&job, ProcessingEventStage::Thumbnail)
        .await
        .unwrap()
        .is_none());
    assert!(!repos
        .jobs
        .complete_thumbnail(&fence, Some(42))
        .await
        .unwrap());
    assert!(!repos
        .jobs
        .skip_derived(&fence, ProcessingReason::ProviderDisabled)
        .await
        .unwrap());
    repos
        .jobs
        .fail(&job, "synthetic expired work", true)
        .await
        .unwrap();
    assert_eq!(events(db).await.len(), before);
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repos
            .documents
            .find_by_id(vault, doc)
            .await
            .unwrap()
            .unwrap()
            .page_count,
        None
    );
}

#[tokio::test]
async fn sqlite_derived_stage_transactions() {
    contract(&Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_derived_stage_transactions() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    contract(
        &Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}

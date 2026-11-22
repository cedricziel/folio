mod common;

use chrono::{DateTime, Utc};
use folio_core::models::{CreateDocument, Id, ProcessingStageSummary, StageOutcome, VaultId};
use folio_core::processing_events::{ProcessingEventStage, ProcessingReason};
use folio_infra::{Database, JobType};

async fn execute(db: &Database, sql: String) {
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map(|_| ())
            .unwrap(),
        Database::Postgres(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map(|_| ())
            .unwrap(),
    };
}

async fn insert(db: &Database, document: Id, vault: VaultId, summary: &ProcessingStageSummary) {
    let stage = serde_json::to_value(summary.stage).unwrap();
    let state = serde_json::to_value(summary.state).unwrap();
    let reason = summary
        .reason_code
        .map(|reason| {
            format!(
                "'{}'",
                serde_json::to_value(reason).unwrap().as_str().unwrap()
            )
        })
        .unwrap_or_else(|| "NULL".into());
    let retry_at = summary
        .retry_at
        .map(|time| format!("'{}'", time.to_rfc3339()))
        .unwrap_or_else(|| "NULL".into());
    let revision = summary
        .revision_id
        .map(|id| format!("'{id}'"))
        .unwrap_or_else(|| "NULL".into());
    let has_text = summary
        .has_text
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".into());
    execute(db, format!("INSERT INTO processing_stage_runs (id, document_id, vault_id, revision_id, group_id, stage, state, attempt, queue_attempt, reason_code, retry_at, has_text, created_at, updated_at) VALUES ('{}', '{document}', '{vault}', {revision}, '{}', '{}', '{}', {}, 0, {reason}, {retry_at}, {has_text}, '2026-11-03T00:00:00Z', '2026-11-03T00:00:00Z')", summary.run_id, summary.run_id, stage.as_str().unwrap(), state.as_str().unwrap(), summary.attempt)).await;
}

async fn summary_contract(db: &Database) {
    let repos = db.repositories().await;
    let other = repos.vaults.create("Default", None).await.unwrap().id;
    let vault = repos.vaults.create("Stages", None).await.unwrap().id;
    let doc = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Tracked".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        repos
            .documents
            .processing_stage_summaries(vault, doc.id)
            .await
            .unwrap(),
        vec![]
    );
    let file = Id::new_v4();
    let first_revision = Id::new_v4();
    execute(db, format!("INSERT INTO document_files (id, document_id, vault_id, role, storage_key, mime_type, verification_state)
        VALUES ('{file}', '{}', '{vault}', 'original', 'fixture', 'application/pdf', 'pending');
        INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at)
        VALUES ('{first_revision}', '{}', '{vault}', 1, '{file}', '2026-11-22T00:00:00Z');
        UPDATE documents SET current_revision_id = '{first_revision}' WHERE id = '{}';", doc.id, doc.id, doc.id)).await;
    let retry_at: DateTime<Utc> = "2026-11-03T00:00:10Z".parse().unwrap();
    let retry = ProcessingStageSummary {
        run_id: Id::new_v4(),
        revision_id: Some(first_revision),
        stage: ProcessingEventStage::Ocr,
        attempt: 2,
        state: StageOutcome::WaitingToRetry,
        reason_code: Some(ProcessingReason::ProviderFailed),
        retry_at: Some(retry_at),
        has_text: None,
    };
    let skipped = ProcessingStageSummary {
        run_id: Id::new_v4(),
        revision_id: None,
        stage: ProcessingEventStage::Embedding,
        attempt: 1,
        state: StageOutcome::Skipped,
        reason_code: Some(ProcessingReason::NoText),
        retry_at: None,
        has_text: None,
    };
    insert(db, doc.id, vault, &skipped).await;
    insert(db, doc.id, vault, &retry).await;
    let hidden = ProcessingStageSummary {
        run_id: Id::new_v4(),
        stage: ProcessingEventStage::Thumbnail,
        state: StageOutcome::Pending,
        attempt: 0,
        reason_code: None,
        ..skipped.clone()
    };
    insert(db, doc.id, other, &hidden).await;
    assert_eq!(
        repos
            .documents
            .processing_stage_summaries(vault, doc.id)
            .await
            .unwrap(),
        vec![retry.clone(), skipped.clone()]
    );
    assert!(repos
        .documents
        .processing_stage_summaries(other, doc.id)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .documents
        .processing_stage_summaries(vault, Id::new_v4())
        .await
        .unwrap()
        .is_empty());

    execute(
        db,
        format!(
            "UPDATE processing_stage_runs SET is_current = FALSE WHERE id = '{}'",
            retry.run_id
        ),
    )
    .await;
    let completed = ProcessingStageSummary {
        run_id: Id::new_v4(),
        revision_id: None,
        state: StageOutcome::Completed,
        attempt: 1,
        reason_code: None,
        retry_at: None,
        has_text: Some(false),
        ..retry
    };
    insert(db, doc.id, vault, &completed).await;
    assert_eq!(
        repos
            .documents
            .processing_stage_summaries(vault, doc.id)
            .await
            .unwrap(),
        vec![completed.clone(), skipped]
    );
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc.id)
            .await
            .unwrap(),
        Some(folio_core::ports::ThumbnailLocation::Legacy)
    );
    execute(
        db,
        format!(
            "UPDATE documents SET current_revision_id = NULL WHERE id = '{}'",
            doc.id
        ),
    )
    .await;
    assert!(repos
        .documents
        .processing_stage_summaries(vault, doc.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        repos
            .documents
            .thumbnail_location(vault, doc.id)
            .await
            .unwrap(),
        None
    );
    let next_revision = Id::new_v4();
    execute(db, format!("INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at)
        VALUES ('{next_revision}', '{}', '{vault}', 2, '{file}', '2026-11-22T00:01:00Z');
        UPDATE documents SET current_revision_id = '{next_revision}' WHERE id = '{}';", doc.id, doc.id)).await;
    assert!(repos
        .documents
        .processing_stage_summaries(vault, doc.id)
        .await
        .unwrap()
        .is_empty());
    execute(
        db,
        format!(
            "UPDATE processing_stage_runs SET is_current = FALSE WHERE id = '{}'",
            completed.run_id
        ),
    )
    .await;
    let current = ProcessingStageSummary {
        run_id: Id::new_v4(),
        revision_id: Some(next_revision),
        ..completed
    };
    insert(db, doc.id, vault, &current).await;
    assert_eq!(
        repos
            .documents
            .processing_stage_summaries(vault, doc.id)
            .await
            .unwrap(),
        vec![current]
    );
    execute(
        db,
        format!(
            "UPDATE documents SET purge_requested_at = '2026-11-03T00:00:00Z' WHERE id = '{}'",
            doc.id
        ),
    )
    .await;
    assert!(repos
        .documents
        .processing_stage_summaries(vault, doc.id)
        .await
        .unwrap()
        .is_empty());
    execute(
        db,
        format!(
            "UPDATE documents SET purge_requested_at = NULL WHERE id = '{}'",
            doc.id
        ),
    )
    .await;
    repos.vaults.delete(vault).await.unwrap();
    assert!(repos
        .documents
        .processing_stage_summaries(vault, doc.id)
        .await
        .unwrap()
        .is_empty());
    repos.vaults.restore(vault).await.unwrap();
    repos.documents.delete(vault, doc.id).await.unwrap();
    assert!(repos
        .documents
        .processing_stage_summaries(vault, doc.id)
        .await
        .unwrap()
        .is_empty());
}

async fn pending_plans_require_live_work(db: &Database) {
    let repos = db.repositories().await;
    let vault = repos.vaults.create("Upgrade", None).await.unwrap().id;
    let doc = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Existing archive".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    repos
        .jobs
        .enqueue(JobType::process_document(doc.id))
        .await
        .unwrap();
    let job = repos.jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(
        repos
            .documents
            .processing_stage_summaries(vault, doc.id)
            .await
            .unwrap()
            .len(),
        3
    );

    // Older workers completed the queue job without resolving every planned stage.
    assert!(repos.jobs.complete(&job).await.unwrap());

    assert!(repos
        .documents
        .processing_stage_summaries(vault, doc.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn sqlite_current_processing_summaries() {
    let (_dir, db) = common::sqlite_database().await;
    summary_contract(&db).await;
    pending_plans_require_live_work(&db).await;
}

#[tokio::test]
async fn postgres_current_processing_summaries() {
    let Some((_container, db)) = common::postgres_database("Processing summaries").await else {
        return;
    };
    summary_contract(&db).await;
    pending_plans_require_live_work(&db).await;
}

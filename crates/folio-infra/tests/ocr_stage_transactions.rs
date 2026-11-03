mod common;

use folio_core::events::{decode_stored_outbox_payload, DecodedOutboxPayload};
use folio_core::models::{CreateDocument, UserId, VaultId};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::{DocumentEvent, Result};
use folio_infra::queue::stages::{self, OcrStageFence};
use folio_infra::{Database, Job, JobType};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

async fn execute(db: &Database, sqlite: &str, postgres: &str) {
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sqlite.to_owned()))
            .execute(pool)
            .await
            .map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(postgres.to_owned()))
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .unwrap();
}

struct Fixture<'a> {
    db: &'a Database,
    job: Job,
    document: Uuid,
    vault: VaultId,
    run: Uuid,
    group: Uuid,
    embedding_run: Uuid,
    lineage: MutationLineage,
}

impl<'a> Fixture<'a> {
    async fn new(db: &'a Database) -> Self {
        let repos = db.repositories().await;
        let vault = repos.vaults.create("OCR source", None).await.unwrap().id;
        let document = repos
            .documents
            .create(
                vault,
                CreateDocument {
                    title: "Immutable OCR title".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
        let lineage = MutationLineage {
            correlation_id: Uuid::new_v4(),
            causation_id: Some(Uuid::new_v4()),
        };
        let id = with_lineage(
            Some(lineage),
            repos.jobs.enqueue(JobType::ExtractText {
                document_id: document,
            }),
        )
        .await
        .unwrap();
        let seeded: (String, String, String) = match db {
            Database::Sqlite(pool) => sqlx::query_as("SELECT r.id, r.group_id, e.id FROM processing_stage_runs r JOIN processing_stage_runs e ON e.group_id = r.group_id AND e.stage = 'embedding' WHERE r.job_id = ? AND r.stage = 'ocr'")
                .bind(id.to_string()).fetch_one(pool).await.unwrap(),
            Database::Postgres(pool) => sqlx::query_as("SELECT r.id::text, r.group_id::text, e.id::text FROM processing_stage_runs r JOIN processing_stage_runs e ON e.group_id = r.group_id AND e.stage = 'embedding' WHERE r.job_id = $1 AND r.stage = 'ocr'")
                .bind(id).fetch_one(pool).await.unwrap(),
        };
        let run = seeded.0.parse().unwrap();
        let group = seeded.1.parse().unwrap();
        let embedding_run = seeded.2.parse().unwrap();
        let setup = format!("UPDATE documents SET file_path = 'private/source.pdf' WHERE id = '{document}'; DELETE FROM outbox_events;");
        execute(db, &setup, &setup).await;
        let job = repos.jobs.dequeue().await.unwrap().unwrap();
        assert_eq!(job.id, id);
        Self {
            db,
            job,
            document,
            vault,
            run,
            group,
            embedding_run,
            lineage,
        }
    }

    async fn start_job(&self, job: &Job) -> Result<Option<OcrStageFence>> {
        match self.db {
            Database::Sqlite(pool) => stages::sqlite::start_ocr(pool, job).await,
            Database::Postgres(pool) => stages::postgres::start_ocr(pool, job).await,
        }
    }

    async fn start(&self) -> Result<Option<OcrStageFence>> {
        self.start_job(&self.job).await
    }

    async fn complete(&self, fence: &OcrStageFence, content: &str) -> Result<bool> {
        self.complete_with(fence, content, true).await
    }

    async fn complete_with(
        &self,
        fence: &OcrStageFence,
        content: &str,
        embed: bool,
    ) -> Result<bool> {
        match self.db {
            Database::Sqlite(pool) => {
                stages::sqlite::complete_ocr(pool, fence, content, embed).await
            }
            Database::Postgres(pool) => {
                stages::postgres::complete_ocr(pool, fence, content, embed).await
            }
        }
    }

    async fn skip(&self, fence: &OcrStageFence) -> Result<bool> {
        match self.db {
            Database::Sqlite(pool) => stages::sqlite::skip_ocr(pool, fence, true).await,
            Database::Postgres(pool) => stages::postgres::skip_ocr(pool, fence, true).await,
        }
    }

    async fn events(&self) -> Vec<DecodedOutboxPayload> {
        let payloads: Vec<String> = match self.db {
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
            .filter(|decoded| {
                !decoded
                    .envelope
                    .as_ref()
                    .is_some_and(|envelope| envelope.event_type().starts_with("vault"))
            })
            .collect()
    }

    async fn state(&self) -> (String, i64, Option<String>, String) {
        let sql = format!("SELECT r.state, r.attempt, d.content, d.ocr_status FROM processing_stage_runs r JOIN documents d ON d.id = r.document_id WHERE r.id = '{}'", self.run);
        match self.db {
            Database::Sqlite(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
            Database::Postgres(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
        }
    }

    async fn has_text(&self) -> Option<bool> {
        let sql = format!(
            "SELECT has_text FROM processing_stage_runs WHERE id = '{}'",
            self.run
        );
        match self.db {
            Database::Sqlite(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
            Database::Postgres(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
        }
    }

    async fn update(&self, sql: String) {
        execute(self.db, &sql, &sql).await;
    }

    async fn embedding_jobs(&self) -> Vec<(String, String, String, Option<String>)> {
        let sql = format!("SELECT CAST(id AS TEXT), CAST(processing_group_id AS TEXT), CAST(correlation_id AS TEXT), CAST(causation_id AS TEXT) FROM jobs WHERE processing_group_id = '{}' AND job_type = 'generate_embedding'", self.group);
        match self.db {
            Database::Sqlite(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_all(pool)
                .await
                .unwrap(),
            Database::Postgres(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_all(pool)
                .await
                .unwrap(),
        }
    }

    async fn embedding_state(&self) -> (String, i64, i32, Option<String>, Option<String>, String) {
        let sql = format!("SELECT r.state, r.attempt, r.queue_attempt, CAST(r.job_id AS TEXT), r.reason_code, d.embed_status FROM processing_stage_runs r JOIN documents d ON d.id = r.document_id WHERE r.id = '{}'", self.embedding_run);
        match self.db {
            Database::Sqlite(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
            Database::Postgres(pool) => sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
        }
    }

    async fn finish(&self) {
        self.update(format!("UPDATE jobs SET status = 'completed', claim_id = NULL, lease_expires_at = NULL WHERE processing_group_id = '{}'", self.group)).await;
    }
}

async fn duplicate_empty_completion_is_one_combined_event(db: &Database) {
    let fixture = Fixture::new(db).await;
    let unrelated_lineage = MutationLineage {
        correlation_id: Uuid::new_v4(),
        causation_id: Some(Uuid::new_v4()),
    };
    let (first, duplicate) = with_actor(
        Some(UserId::from("must-not-be-background-actor")),
        with_lineage(Some(unrelated_lineage), async {
            tokio::join!(fixture.start(), fixture.start())
        }),
    )
    .await;
    let fence = first.unwrap().unwrap();
    assert_eq!(fence.document_id(), fixture.document);
    assert_eq!(fence.vault_id(), fixture.vault);
    assert_eq!(Some(fence.clone()), duplicate.unwrap());
    let (first, duplicate) = with_actor(
        Some(UserId::from("must-not-be-background-actor")),
        with_lineage(Some(unrelated_lineage), async {
            tokio::join!(fixture.complete(&fence, ""), fixture.complete(&fence, ""))
        }),
    )
    .await;
    assert_ne!(first.unwrap(), duplicate.unwrap());
    assert_eq!(
        fixture.state().await,
        ("completed".into(), 1, Some("".into()), "done".into())
    );
    assert!(fixture.start().await.unwrap().is_none());
    assert_eq!(fixture.has_text().await, Some(false));
    let events = fixture.events().await;
    assert_eq!(events.len(), 3);
    assert!(events[0].event.is_none());
    assert!(
        matches!(events[1].event, Some(DocumentEvent::OcrComplete { document_id }) if document_id == fixture.document)
    );
    let started = events[0].envelope.as_ref().unwrap();
    let completed = events[1].envelope.as_ref().unwrap();
    assert_eq!(started.event_type(), "document.processing.ocr.started");
    assert_eq!(completed.event_type(), "document.processing.ocr.completed");
    assert_eq!(completed.data()["has_text"], false);
    assert_eq!(completed.data()["run_id"], fixture.run.to_string());
    assert_eq!(completed.data()["revision_id"], serde_json::Value::Null);
    assert_eq!(completed.vault_id(), fixture.vault);
    assert_eq!(completed.correlation_id(), fixture.lineage.correlation_id);
    assert_eq!(completed.causation_id(), fixture.lineage.causation_id);
    assert_eq!(started.correlation_id(), fixture.lineage.correlation_id);
    assert_eq!(started.causation_id(), fixture.lineage.causation_id);
    assert_eq!(completed.data()["title"], "Immutable OCR title");
    assert_eq!(started.actor_id(), None);
    assert_eq!(completed.actor_id(), None);
    let serialized = serde_json::to_string(completed).unwrap();
    assert!(!serialized.contains("private/source.pdf"));
    assert_eq!(
        fixture.embedding_state().await,
        (
            "skipped".into(),
            1,
            fixture.job.attempts,
            Some(fixture.job.id.to_string()),
            Some("no_text".into()),
            "skipped".into()
        )
    );
    let skipped = events[2].envelope.as_ref().unwrap();
    assert!(
        matches!(events[2].event, Some(DocumentEvent::Processed { document_id }) if document_id == fixture.document)
    );
    assert_eq!(
        skipped.event_type(),
        "document.processing.embedding.skipped"
    );
    assert_eq!(skipped.data()["reason_code"], "no_text");
    assert_eq!(skipped.data()["run_id"], fixture.embedding_run.to_string());
    assert_eq!(skipped.correlation_id(), fixture.lineage.correlation_id);
    assert_eq!(skipped.actor_id(), None);
    assert!(fixture.embedding_jobs().await.is_empty());
    fixture.finish().await;
}

async fn outbox_rejection_rolls_back_start_and_completion(db: &Database) {
    let fixture = Fixture::new(db).await;
    execute(db,
        "CREATE TRIGGER refuse_stage_outbox BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END",
        "CREATE FUNCTION refuse_stage_outbox() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql; CREATE TRIGGER refuse_stage_outbox BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_stage_outbox()"
    ).await;
    assert!(fixture.start().await.is_err());
    assert_eq!(
        fixture.state().await,
        ("pending".into(), 0, None, "pending".into())
    );
    assert!(fixture.events().await.is_empty());
    execute(
        db,
        "DROP TRIGGER refuse_stage_outbox",
        "DROP TRIGGER refuse_stage_outbox ON outbox_events; DROP FUNCTION refuse_stage_outbox()",
    )
    .await;
    let fence = fixture.start().await.unwrap().unwrap();
    execute(db,
        "CREATE TRIGGER refuse_stage_outbox BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END",
        "CREATE FUNCTION refuse_stage_outbox() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql; CREATE TRIGGER refuse_stage_outbox BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_stage_outbox()"
    ).await;
    assert!(fixture
        .complete(&fence, "private OCR output")
        .await
        .is_err());
    assert_eq!(
        fixture.state().await,
        ("running".into(), 1, None, "running".into())
    );
    assert_eq!(fixture.events().await.len(), 1);
    execute(
        db,
        "DROP TRIGGER refuse_stage_outbox",
        "DROP TRIGGER refuse_stage_outbox ON outbox_events; DROP FUNCTION refuse_stage_outbox()",
    )
    .await;
    assert!(fixture
        .complete(&fence, "private OCR output")
        .await
        .unwrap());
    assert_eq!(fixture.has_text().await, Some(true));
    let events = fixture.events().await;
    assert_eq!(events.len(), 2);
    let completed = events[1].envelope.as_ref().unwrap();
    assert_eq!(completed.data()["has_text"], true);
    assert!(!serde_json::to_string(completed)
        .unwrap()
        .contains("private OCR output"));
    let children = fixture.embedding_jobs().await;
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].1, fixture.group.to_string());
    assert_eq!(children[0].2, fixture.lineage.correlation_id.to_string());
    assert_eq!(
        children[0].3,
        fixture.lineage.causation_id.map(|id| id.to_string())
    );
    assert_eq!(
        fixture.embedding_state().await.3,
        Some(children[0].0.clone())
    );
    assert!(!fixture.complete(&fence, "duplicate text").await.unwrap());
    assert!(fixture.start().await.unwrap().is_none());
    assert_eq!(fixture.embedding_jobs().await, children);
    fixture.finish().await;
}

async fn expired_and_replaced_claims_cannot_publish(db: &Database) {
    let fixture = Fixture::new(db).await;
    let mut stale = fixture.job.clone();
    stale.job_type = JobType::ProcessDocument {
        document_id: fixture.document,
    };
    assert!(fixture.start_job(&stale).await.unwrap().is_none());
    stale = fixture.job.clone();
    stale.claim_id = Some(Uuid::new_v4());
    assert!(fixture.start_job(&stale).await.unwrap().is_none());
    stale = fixture.job.clone();
    stale.attempts += 1;
    assert!(fixture.start_job(&stale).await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'",
            fixture.job.id
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE jobs SET lease_expires_at = 4102444800 WHERE id = '{}'",
            fixture.job.id
        ))
        .await;
    let fence = fixture.start().await.unwrap().unwrap();
    fixture
        .update(format!(
            "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'",
            fixture.job.id
        ))
        .await;
    assert!(!fixture.complete(&fence, "expired").await.unwrap());
    fixture
        .update(format!(
            "UPDATE jobs SET lease_expires_at = 4102444800, claim_id = '{}' WHERE id = '{}'",
            Uuid::new_v4(),
            fixture.job.id
        ))
        .await;
    assert!(!fixture.complete(&fence, "stale claim").await.unwrap());
    assert_eq!(
        fixture.state().await,
        ("running".into(), 1, None, "running".into())
    );
    assert_eq!(fixture.events().await.len(), 1);
    fixture.finish().await;
}

async fn embedding_enqueue_failure_rolls_back_ocr_and_events(db: &Database) {
    let fixture = Fixture::new(db).await;
    let fence = fixture.start().await.unwrap().unwrap();
    execute(db,
        "CREATE TRIGGER refuse_embedding_job BEFORE INSERT ON jobs WHEN NEW.job_type = 'generate_embedding' BEGIN SELECT RAISE(ABORT, 'refused child'); END",
        "CREATE FUNCTION refuse_embedding_job() RETURNS trigger AS $$ BEGIN IF NEW.job_type = 'generate_embedding' THEN RAISE EXCEPTION 'refused child'; END IF; RETURN NEW; END $$ LANGUAGE plpgsql; CREATE TRIGGER refuse_embedding_job BEFORE INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION refuse_embedding_job()"
    ).await;
    assert!(fixture
        .complete(&fence, "text needing embedding")
        .await
        .is_err());
    assert_eq!(
        fixture.state().await,
        ("running".into(), 1, None, "running".into())
    );
    assert_eq!(fixture.events().await.len(), 1);
    assert!(fixture.embedding_jobs().await.is_empty());
    assert_eq!(
        fixture.embedding_state().await,
        ("pending".into(), 0, 0, None, None, "pending".into())
    );
    execute(
        db,
        "DROP TRIGGER refuse_embedding_job",
        "DROP TRIGGER refuse_embedding_job ON jobs; DROP FUNCTION refuse_embedding_job()",
    )
    .await;
    assert!(fixture
        .complete(&fence, "text needing embedding")
        .await
        .unwrap());
    let children = fixture.embedding_jobs().await;
    assert_eq!(children.len(), 1);
    assert!(!fixture
        .complete(&fence, "text needing embedding")
        .await
        .unwrap());
    assert_eq!(fixture.embedding_jobs().await, children);
    assert_eq!(fixture.events().await.len(), 2);
    fixture.finish().await;
}

async fn disabled_ocr_skips_empty_embedding_without_ocr_complete(db: &Database) {
    let fixture = Fixture::new(db).await;
    let fence = fixture.start().await.unwrap().unwrap();
    assert!(fixture.skip(&fence).await.unwrap());
    assert!(!fixture.skip(&fence).await.unwrap());
    assert!(!fixture.complete(&fence, "late output").await.unwrap());
    assert_eq!(
        fixture.state().await,
        ("skipped".into(), 1, None, "skipped".into())
    );
    assert_eq!(fixture.has_text().await, None);
    assert!(fixture.embedding_jobs().await.is_empty());
    assert_eq!(
        fixture.embedding_state().await,
        (
            "skipped".into(),
            1,
            fixture.job.attempts,
            Some(fixture.job.id.to_string()),
            Some("no_text".into()),
            "skipped".into()
        )
    );
    let events = fixture.events().await;
    assert_eq!(events.len(), 3);
    assert!(events[0].event.is_none());
    assert!(events[1].event.is_none());
    assert_eq!(
        events[1].envelope.as_ref().unwrap().event_type(),
        "document.processing.ocr.skipped"
    );
    assert_eq!(
        events[1].envelope.as_ref().unwrap().data()["reason_code"],
        "provider_disabled"
    );
    assert!(events[1]
        .envelope
        .as_ref()
        .unwrap()
        .data()
        .get("has_text")
        .is_none());
    assert!(
        matches!(events[2].event, Some(DocumentEvent::Processed { document_id }) if document_id == fixture.document)
    );
    assert_eq!(
        events[2].envelope.as_ref().unwrap().event_type(),
        "document.processing.embedding.skipped"
    );
    fixture.finish().await;
}

async fn embedding_skip_outbox_failure_rolls_back_ocr_completion(db: &Database) {
    let fixture = Fixture::new(db).await;
    let fence = fixture.start().await.unwrap().unwrap();
    execute(db,
        "CREATE TRIGGER refuse_embedding_skip BEFORE INSERT ON outbox_events WHEN NEW.type = 'processed' BEGIN SELECT RAISE(ABORT, 'refused skip'); END",
        "CREATE FUNCTION refuse_embedding_skip() RETURNS trigger AS $$ BEGIN IF NEW.type = 'processed' THEN RAISE EXCEPTION 'refused skip'; END IF; RETURN NEW; END $$ LANGUAGE plpgsql; CREATE TRIGGER refuse_embedding_skip BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_embedding_skip()"
    ).await;
    assert!(fixture.complete(&fence, "").await.is_err());
    assert_eq!(
        fixture.state().await,
        ("running".into(), 1, None, "running".into())
    );
    assert_eq!(
        fixture.embedding_state().await,
        ("pending".into(), 0, 0, None, None, "pending".into())
    );
    assert_eq!(fixture.events().await.len(), 1);
    execute(db, "DROP TRIGGER refuse_embedding_skip", "DROP TRIGGER refuse_embedding_skip ON outbox_events; DROP FUNCTION refuse_embedding_skip()").await;
    assert!(fixture.complete(&fence, "").await.unwrap());
    assert_eq!(fixture.events().await.len(), 3);
    fixture.finish().await;
}

async fn disabled_ocr_preserves_content_and_assigns_embedding(db: &Database) {
    let fixture = Fixture::new(db).await;
    fixture
        .update(format!(
            "UPDATE documents SET content = 'existing extracted text' WHERE id = '{}'",
            fixture.document
        ))
        .await;
    let fence = fixture.start().await.unwrap().unwrap();
    assert!(fixture.skip(&fence).await.unwrap());
    assert_eq!(
        fixture.state().await,
        (
            "skipped".into(),
            1,
            Some("existing extracted text".into()),
            "skipped".into()
        )
    );
    assert_eq!(fixture.has_text().await, None);
    let children = fixture.embedding_jobs().await;
    assert_eq!(children.len(), 1);
    assert!(!fixture.skip(&fence).await.unwrap());
    assert_eq!(fixture.embedding_jobs().await, children);
    let events = fixture.events().await;
    assert_eq!(events.len(), 2);
    assert!(events[1].event.is_none());
    assert_eq!(
        events[1].envelope.as_ref().unwrap().event_type(),
        "document.processing.ocr.skipped"
    );
    fixture.finish().await;
}

async fn moved_and_trashed_documents_cannot_publish(db: &Database) {
    let fixture = Fixture::new(db).await;
    let destination = db
        .repositories()
        .await
        .vaults
        .create("Destination", None)
        .await
        .unwrap()
        .id;
    fixture
        .update(format!(
            "UPDATE documents SET vault_id = '{destination}' WHERE id = '{}'",
            fixture.document
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE documents SET vault_id = '{}' WHERE id = '{}'",
            fixture.vault, fixture.document
        ))
        .await;
    fixture
        .update(format!(
            "UPDATE documents SET deleted_at = '2026-11-03T00:00:00Z' WHERE id = '{}'",
            fixture.document
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE documents SET deleted_at = NULL WHERE id = '{}'",
            fixture.document
        ))
        .await;
    fixture
        .update(format!(
            "UPDATE documents SET purge_requested_at = '2026-11-03T00:00:00Z' WHERE id = '{}'",
            fixture.document
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE documents SET purge_requested_at = NULL WHERE id = '{}'",
            fixture.document
        ))
        .await;
    let fence = fixture.start().await.unwrap().unwrap();
    fixture
        .update(format!(
            "UPDATE documents SET vault_id = '{destination}' WHERE id = '{}'",
            fixture.document
        ))
        .await;
    assert!(!fixture.complete(&fence, "moved").await.unwrap());
    fixture.update(format!("UPDATE documents SET vault_id = '{}', purge_requested_at = '2026-11-03T00:00:00Z' WHERE id = '{}'", fixture.vault, fixture.document)).await;
    assert!(!fixture.complete(&fence, "purge requested").await.unwrap());
    fixture.update(format!("UPDATE documents SET vault_id = '{}', deleted_at = '2026-11-03T00:00:00Z' WHERE id = '{}'", fixture.vault, fixture.document)).await;
    assert!(!fixture.complete(&fence, "trashed").await.unwrap());
    assert_eq!(
        fixture.state().await,
        ("running".into(), 1, None, "running".into())
    );
    assert_eq!(fixture.events().await.len(), 1);
    fixture.finish().await;
}

async fn noncurrent_and_revision_backed_runs_cannot_execute(db: &Database) {
    let fixture = Fixture::new(db).await;
    fixture
        .update(format!(
            "UPDATE processing_stage_runs SET revision_id = '{}' WHERE id = '{}'",
            Uuid::new_v4(),
            fixture.run
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE processing_stage_runs SET revision_id = NULL WHERE id = '{}'",
            fixture.run
        ))
        .await;
    let fence = fixture.start().await.unwrap().unwrap();
    execute(
        db,
        &format!(
            "UPDATE processing_stage_runs SET is_current = 0 WHERE id = '{}'",
            fixture.run
        ),
        &format!(
            "UPDATE processing_stage_runs SET is_current = FALSE WHERE id = '{}'",
            fixture.run
        ),
    )
    .await;
    assert!(!fixture.complete(&fence, "superseded run").await.unwrap());
    execute(db, &format!("UPDATE processing_stage_runs SET is_current = 1, revision_id = '{}' WHERE id = '{}'", Uuid::new_v4(), fixture.run), &format!("UPDATE processing_stage_runs SET is_current = TRUE, revision_id = '{}' WHERE id = '{}'", Uuid::new_v4(), fixture.run)).await;
    assert!(!fixture.complete(&fence, "unfenced revision").await.unwrap());
    assert_eq!(fixture.events().await.len(), 1);
    fixture.finish().await;
}

async fn retry_counts_stage_starts_separately_from_queue_attempts(db: &Database) {
    let fixture = Fixture::new(db).await;
    fixture
        .update(format!(
            "UPDATE jobs SET attempts = 3 WHERE id = '{}'",
            fixture.job.id
        ))
        .await;
    let mut job = fixture.job.clone();
    job.attempts = 3;
    let old = fixture.start_job(&job).await.unwrap().unwrap();
    fixture.update(format!("UPDATE processing_stage_runs SET state = 'waiting_to_retry', claim_id = NULL, reason_code = 'provider_failed', retry_at = '2026-11-03T00:00:00Z' WHERE id = '{}'", fixture.run)).await;
    job.attempts = 4;
    job.claim_id = Some(Uuid::new_v4());
    fixture
        .update(format!(
            "UPDATE jobs SET attempts = 4, claim_id = '{}' WHERE id = '{}'",
            job.claim_id.unwrap(),
            job.id
        ))
        .await;
    let new = fixture.start_job(&job).await.unwrap().unwrap();
    assert_ne!(old, new);
    assert!(!fixture.complete(&old, "stale attempt").await.unwrap());
    assert!(fixture.complete(&new, "fresh attempt").await.unwrap());
    let events = fixture.events().await;
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].envelope.as_ref().unwrap().data()["attempt"], 1);
    assert_eq!(events[1].envelope.as_ref().unwrap().data()["attempt"], 2);
    assert_eq!(events[2].envelope.as_ref().unwrap().data()["attempt"], 2);
    fixture.finish().await;
}

async fn changed_job_group_and_missing_lineage_reject_existing_fences(db: &Database) {
    let fixture = Fixture::new(db).await;
    let redirected = serde_json::to_string(&JobType::ExtractText {
        document_id: Uuid::new_v4(),
    })
    .unwrap();
    fixture
        .update(format!(
            "UPDATE jobs SET payload = '{redirected}' WHERE id = '{}'",
            fixture.job.id
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    let original = serde_json::to_string(&fixture.job.job_type).unwrap();
    fixture
        .update(format!(
            "UPDATE jobs SET payload = '{original}' WHERE id = '{}'",
            fixture.job.id
        ))
        .await;
    fixture
        .update(format!(
            "UPDATE processing_stage_runs SET vault_id = '{}' WHERE id = '{}'",
            Uuid::new_v4(),
            fixture.run
        ))
        .await;
    assert!(fixture.start().await.unwrap().is_none());
    fixture
        .update(format!(
            "UPDATE processing_stage_runs SET vault_id = '{}' WHERE id = '{}'",
            fixture.vault, fixture.run
        ))
        .await;
    let fence = fixture.start().await.unwrap().unwrap();
    fixture
        .update(format!(
            "UPDATE jobs SET processing_group_id = '{}' WHERE id = '{}'",
            Uuid::new_v4(),
            fixture.job.id
        ))
        .await;
    assert!(!fixture.complete(&fence, "different group").await.unwrap());
    fixture.update(format!("UPDATE jobs SET processing_group_id = (SELECT group_id FROM processing_stage_runs WHERE id = '{}'), correlation_id = NULL WHERE id = '{}'", fixture.run, fixture.job.id)).await;
    assert!(!fixture
        .complete(&fence, "missing source lineage")
        .await
        .unwrap());
    assert_eq!(
        fixture.state().await,
        ("running".into(), 1, None, "running".into())
    );
    assert_eq!(fixture.events().await.len(), 1);
    fixture.finish().await;
}

async fn disabled_embedding_skips_text_without_queueing(db: &Database) {
    let fixture = Fixture::new(db).await;
    let fence = fixture.start().await.unwrap().unwrap();
    assert!(fixture
        .complete_with(&fence, "text with embedding off", false)
        .await
        .unwrap());
    assert!(fixture.embedding_jobs().await.is_empty());
    assert_eq!(fixture.has_text().await, Some(true));
    assert_eq!(
        fixture.embedding_state().await,
        (
            "skipped".into(),
            1,
            fixture.job.attempts,
            Some(fixture.job.id.to_string()),
            Some("provider_disabled".into()),
            "skipped".into()
        )
    );
    let events = fixture.events().await;
    let skipped = events.last().unwrap().envelope.as_ref().unwrap();
    assert_eq!(
        skipped.event_type(),
        "document.processing.embedding.skipped"
    );
    assert_eq!(skipped.data()["reason_code"], "provider_disabled");
    fixture.finish().await;
}

async fn superseded_embedding_run_keeps_ocr_result(db: &Database) {
    let fixture = Fixture::new(db).await;
    let fence = fixture.start().await.unwrap().unwrap();
    fixture
        .update(format!(
            "UPDATE processing_stage_runs SET is_current = FALSE WHERE id = '{}'",
            fixture.embedding_run
        ))
        .await;
    let embed_status = fixture.embedding_state().await.5;
    assert!(fixture
        .complete(&fence, "text racing a re-embed")
        .await
        .unwrap());
    assert_eq!(
        fixture.state().await,
        (
            "completed".into(),
            1,
            Some("text racing a re-embed".into()),
            "done".into()
        )
    );
    assert!(fixture.embedding_jobs().await.is_empty());
    let embedding = fixture.embedding_state().await;
    assert_eq!(embedding.0, "pending");
    assert_eq!(embedding.5, embed_status);
    assert_eq!(fixture.events().await.len(), 2);
    fixture.finish().await;
}

async fn contract(db: &Database) {
    db.migrate().await.unwrap();
    Box::pin(duplicate_empty_completion_is_one_combined_event(db)).await;
    Box::pin(outbox_rejection_rolls_back_start_and_completion(db)).await;
    Box::pin(embedding_enqueue_failure_rolls_back_ocr_and_events(db)).await;
    Box::pin(disabled_ocr_skips_empty_embedding_without_ocr_complete(db)).await;
    Box::pin(embedding_skip_outbox_failure_rolls_back_ocr_completion(db)).await;
    Box::pin(disabled_ocr_preserves_content_and_assigns_embedding(db)).await;
    Box::pin(disabled_embedding_skips_text_without_queueing(db)).await;
    Box::pin(superseded_embedding_run_keeps_ocr_result(db)).await;
    Box::pin(expired_and_replaced_claims_cannot_publish(db)).await;
    Box::pin(moved_and_trashed_documents_cannot_publish(db)).await;
    Box::pin(noncurrent_and_revision_backed_runs_cannot_execute(db)).await;
    Box::pin(retry_counts_stage_starts_separately_from_queue_attempts(db)).await;
    Box::pin(changed_job_group_and_missing_lineage_reject_existing_fences(db)).await;
}

#[tokio::test]
async fn sqlite_ocr_stage_transactions() {
    contract(&Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_ocr_stage_transactions() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL", started) else {
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

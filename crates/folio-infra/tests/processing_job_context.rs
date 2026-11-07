mod common;

use folio_core::models::{CreateDocument, VaultId};
use folio_core::mutation::{with_lineage, MutationLineage};
use folio_infra::{Database, FailOutcome, JobType};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

async fn contract(db: &Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    let vault = repos.vaults.create("Source", None).await.unwrap().id;
    let destination = repos.vaults.create("Destination", None).await.unwrap().id;
    let document = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Queued document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let lineage = MutationLineage {
        correlation_id: Uuid::new_v4(),
        causation_id: Some(Uuid::new_v4()),
    };
    let id = with_lineage(
        Some(lineage),
        repos.jobs.enqueue(JobType::process_document(document.id)),
    )
    .await
    .unwrap();
    let context = repos.jobs.context(id).await.unwrap().unwrap();
    assert_eq!(context.vault_id, Some(vault));
    assert_eq!(context.lineage, lineage);
    let job = repos.jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(
        repos.jobs.fail(&job, "fixture", true).await.unwrap(),
        FailOutcome::Retrying
    );
    move_document(db, document.id, destination).await;
    assert_eq!(
        repos.jobs.context(id).await.unwrap().unwrap(),
        context,
        "retry context must not follow a moved document"
    );
    let fresh = repos
        .jobs
        .enqueue(JobType::process_document(document.id))
        .await
        .unwrap();
    let fresh_context = repos.jobs.context(fresh).await.unwrap().unwrap();
    assert_eq!(fresh_context.vault_id, Some(destination));
    assert_ne!(fresh_context.lineage.correlation_id, lineage.correlation_id);
    assert_eq!(fresh_context.lineage.causation_id, None);
    assert_eq!(repos.jobs.context(Uuid::new_v4()).await.unwrap(), None);
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE jobs SET correlation_id = NULL, causation_id = NULL, origin_vault_id = NULL WHERE id = ?")
                .bind(id.to_string()).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE jobs SET correlation_id = NULL, causation_id = NULL, origin_vault_id = NULL WHERE id = $1")
                .bind(id).execute(pool).await.unwrap();
        }
    }
    assert_eq!(
        repos.jobs.context(id).await.unwrap(),
        None,
        "legacy rows must not invent source lineage"
    );
}

async fn move_document(db: &Database, document: Uuid, vault: VaultId) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE documents SET vault_id = ? WHERE id = ?")
                .bind(vault.to_string())
                .bind(document.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE documents SET vault_id = $1 WHERE id = $2")
                .bind(vault.0)
                .bind(document)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn sqlite_processing_job_context() {
    contract(&Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_processing_job_context() {
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

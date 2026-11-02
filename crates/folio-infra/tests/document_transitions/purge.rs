use super::*;

async fn purge_transition_contract(db: &Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Retained occurrence".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let created = payloads(db).await;
    assert!(r.documents.request_purge(vault, document.id).await.is_err());
    assert_eq!(payloads(db).await, created);
    r.documents.delete(vault, document.id).await.unwrap();
    r.documents.request_purge(vault, document.id).await.unwrap();
    let requested = payloads(db).await;
    assert_eq!(
        requested
            .iter()
            .filter(|row| row["envelope"]["event_type"] == "document.trashed")
            .count(),
        1
    );
    let request = requested
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.purge_requested")
        .expect("purge request is distinct from trash");
    assert_eq!(request["envelope"]["data"]["title"], "Retained occurrence");
    let queue = match db {
        Database::Sqlite(pool) => folio_infra::queue::JobQueue::sqlite(pool.clone()),
        Database::Postgres(pool) => folio_infra::queue::JobQueue::postgres(pool.clone()),
    };
    let job = queue.latest_purge_job(document.id).await.unwrap().unwrap();
    let job = serde_json::to_value(job).unwrap();
    assert_eq!(
        job["data"]["correlation_id"],
        request["envelope"]["correlation_id"]
    );
    assert_eq!(job["data"]["causation_id"], request["envelope"]["event_id"]);

    r.documents.request_purge(vault, document.id).await.unwrap();
    assert_eq!(payloads(db).await, requested);
    r.documents
        .finalize_purge(vault, document.id)
        .await
        .unwrap();
    let completed = payloads(db).await;
    let purged = completed
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.purged")
        .expect("committed removal retains its own occurrence");
    assert_eq!(purged["envelope"]["document_id"], document.id.to_string());
    assert_eq!(purged["envelope"]["data"]["title"], "Retained occurrence");
    assert_ne!(
        purged["envelope"]["event_id"],
        request["envelope"]["event_id"]
    );
    r.documents
        .finalize_purge(vault, document.id)
        .await
        .unwrap();
    assert_eq!(payloads(db).await, completed);
}

#[tokio::test]
async fn sqlite_purge_request_and_completion_are_distinct_retry_safe_occurrences() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("purge.db").display()
    ))
    .await
    .unwrap();
    purge_transition_contract(&db).await;
}

#[tokio::test]
async fn postgres_purge_request_and_completion_are_distinct_retry_safe_occurrences() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL purge transitions", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    purge_transition_contract(&db).await;
}

#[test]
fn legacy_purge_jobs_decode_without_lineage() {
    let id = Id::new_v4();
    let job: folio_infra::queue::JobType = serde_json::from_value(serde_json::json!({
        "type": "PurgeDocument", "data": { "document_id": id }
    }))
    .unwrap();
    assert!(matches!(job, folio_infra::queue::JobType::PurgeDocument {
        document_id, correlation_id: None, causation_id: None,
    } if document_id == id));
}

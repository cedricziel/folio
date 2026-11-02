use super::*;

async fn classification_contract(db: &Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Classifications".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let correspondent = r
        .correspondents
        .create(
            vault,
            folio_core::models::CreateCorrespondent {
                name: "Sender".into(),
            },
        )
        .await
        .unwrap();
    let kind = r
        .document_types
        .create(
            vault,
            folio_core::models::CreateDocumentType {
                name: "Invoice".into(),
            },
        )
        .await
        .unwrap();
    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                correspondent_id: Some(Some(correspondent.id)),
                document_type_id: Some(Some(kind.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let assigned = payloads(db).await;
    let from = assigned
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.correspondent_changed")
        .unwrap();
    let kind_event = assigned
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.document_type_changed")
        .unwrap();
    assert!(from["envelope"]["data"]["before_id"].is_null());
    assert_eq!(
        from["envelope"]["data"]["after_id"],
        correspondent.id.to_string()
    );
    assert!(kind_event["envelope"]["data"]["before_id"].is_null());
    assert_eq!(
        kind_event["envelope"]["data"]["after_id"],
        kind.id.to_string()
    );
    assert_eq!(
        from["envelope"]["mutation_id"],
        kind_event["envelope"]["mutation_id"]
    );
    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                correspondent_id: Some(None),
                document_type_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let cleared = payloads(db).await;
    let from = cleared
        .iter()
        .rev()
        .find(|row| row["envelope"]["event_type"] == "document.correspondent_changed")
        .unwrap();
    let kind_event = cleared
        .iter()
        .rev()
        .find(|row| row["envelope"]["event_type"] == "document.document_type_changed")
        .unwrap();
    assert_eq!(
        from["envelope"]["data"]["before_id"],
        correspondent.id.to_string()
    );
    assert!(from["envelope"]["data"]["after_id"].is_null());
    assert_eq!(
        kind_event["envelope"]["data"]["before_id"],
        kind.id.to_string()
    );
    assert!(kind_event["envelope"]["data"]["after_id"].is_null());
    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                correspondent_id: Some(None),
                document_type_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(payloads(db).await, cleared);
}

#[tokio::test]
async fn sqlite_classification_assignment_and_clearing_preserve_exact_ids() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("classifications.db").display()
    ))
    .await
    .unwrap();
    classification_contract(&db).await;
}

#[tokio::test]
async fn postgres_classification_assignment_and_clearing_preserve_exact_ids() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) =
        common::container_or_skip("PostgreSQL classification transitions", started)
    else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    classification_contract(&db).await;
}

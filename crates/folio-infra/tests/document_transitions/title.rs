use super::*;

async fn title_transition_contract(db: &Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Before".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let lineage = folio_core::mutation::MutationLineage {
        correlation_id: Id::from_u128(101),
        causation_id: Some(Id::from_u128(102)),
    };
    folio_core::mutation::with_lineage(
        Some(lineage),
        with_actor(
            Some(UserId::from("transition-test")),
            r.documents.update(
                vault,
                document.id,
                UpdateDocument {
                    title: Some("After".into()),
                    ..Default::default()
                },
            ),
        ),
    )
    .await
    .unwrap();

    let stored = payloads(db).await;
    let title_events: Vec<_> = stored
        .iter()
        .filter(|row| row["envelope"]["event_type"] == "document.title_changed")
        .collect();
    assert_eq!(
        title_events.len(),
        1,
        "a committed title edit needs one precise transition"
    );
    let event = &title_events[0]["envelope"];
    assert_eq!(event["actor_id"], "transition-test");
    assert_eq!(event["correlation_id"], lineage.correlation_id.to_string());
    assert_eq!(
        event["causation_id"],
        lineage.causation_id.unwrap().to_string()
    );
    assert_ne!(event["mutation_id"], event["correlation_id"]);
    assert_eq!(event["vault_id"], vault.to_string());
    assert_eq!(event["document_id"], document.id.to_string());
    assert_eq!(event["data"]["before"], "Before");
    assert_eq!(event["data"]["after"], "After");
    let generic = stored
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.updated")
        .expect("legacy update remains available");
    assert_eq!(generic["envelope"]["mutation_id"], event["mutation_id"]);
    assert_ne!(generic["envelope"]["event_id"], event["event_id"]);
    assert_eq!(
        generic["envelope"]["correlation_id"],
        event["correlation_id"]
    );

    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("After".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(payloads(db).await, stored, "repeating a title is a no-op");

    let rejected = r
        .documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("Must roll back".into()),
                correspondent_id: Some(Some(Id::new_v4())),
                ..Default::default()
            },
        )
        .await;
    assert!(rejected.is_err());
    assert_eq!(
        payloads(db).await,
        stored,
        "invalid references publish nothing"
    );
    assert_eq!(
        r.documents
            .find_by_id(vault, document.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "After"
    );
}

#[tokio::test]
async fn sqlite_title_transition_is_atomic_attributed_and_noop_safe() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("transitions.db").display()
    ))
    .await
    .unwrap();
    title_transition_contract(&db).await;
}

#[tokio::test]
async fn postgres_title_transition_is_atomic_attributed_and_noop_safe() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL document transitions", started)
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
    title_transition_contract(&db).await;
}

use super::*;

async fn move_transition_contract(db: &Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let source = r.vaults.instance_default().await.unwrap();
    let target = r.vaults.create("Destination", None).await.unwrap().id;
    let tag = r
        .tags
        .create(
            source,
            folio_core::models::CreateTag {
                name: "Source category".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let document = r
        .documents
        .create(
            source,
            CreateDocument {
                title: "Moving".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.documents
        .add_tag(source, document.id, tag.id)
        .await
        .unwrap();
    let before_move = payloads(db).await;
    assert!(r
        .documents
        .move_to(document.id, target, source, None)
        .await
        .is_err());
    assert!(r
        .documents
        .move_to(
            document.id,
            source,
            folio_core::models::VaultId(Id::new_v4()),
            None
        )
        .await
        .is_err());
    assert_eq!(payloads(db).await, before_move);
    assert!(r
        .documents
        .find_by_id(source, document.id)
        .await
        .unwrap()
        .is_some());
    r.documents
        .move_to(document.id, source, target, None)
        .await
        .unwrap();
    let destination_tags = r.documents.get_tags(target, document.id).await.unwrap();
    assert_eq!(destination_tags.len(), 1);
    assert_ne!(destination_tags[0].id, tag.id);
    let stored = payloads(db).await;
    let outgoing = stored
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.moved_out")
        .expect("source receives a scoped departure");
    let incoming = stored
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.moved_in")
        .expect("destination receives a scoped arrival");
    assert_eq!(outgoing["envelope"]["vault_id"], source.to_string());
    assert_eq!(incoming["envelope"]["vault_id"], target.to_string());
    assert_eq!(
        outgoing["envelope"]["data"]["tags"][0]["id"],
        tag.id.to_string()
    );
    assert_eq!(
        incoming["envelope"]["data"]["tags"][0]["id"],
        destination_tags[0].id.to_string()
    );
    assert!(!outgoing.to_string().contains(&target.to_string()));
    assert!(!outgoing
        .to_string()
        .contains(&destination_tags[0].id.to_string()));
    assert!(!incoming.to_string().contains(&source.to_string()));
    assert!(!incoming.to_string().contains(&tag.id.to_string()));
    assert_eq!(
        outgoing["envelope"]["mutation_id"],
        incoming["envelope"]["mutation_id"]
    );
    r.documents
        .move_to(document.id, target, target, None)
        .await
        .unwrap();
    assert_eq!(payloads(db).await, stored);
}

#[tokio::test]
async fn sqlite_move_events_do_not_disclose_other_vault_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("moves.db").display()
    ))
    .await
    .unwrap();
    move_transition_contract(&db).await;
}

#[tokio::test]
async fn postgres_move_events_do_not_disclose_other_vault_identifiers() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL move transitions", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    move_transition_contract(&db).await;
}

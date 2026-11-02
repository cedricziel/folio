use super::*;

async fn tag_transition_contract(db: &Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Tag changes".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let a = r
        .tags
        .create(
            vault,
            folio_core::models::CreateTag {
                name: "A".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let b = r
        .tags
        .create(
            vault,
            folio_core::models::CreateTag {
                name: "B".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    r.documents.add_tag(vault, document.id, a.id).await.unwrap();
    let added = payloads(db).await;
    let event = added
        .iter()
        .find(|row| row["envelope"]["event_type"] == "document.tags_changed")
        .expect("adding a tag emits its ID delta");
    assert_eq!(
        event["envelope"]["data"]["before_ids"],
        serde_json::json!([])
    );
    assert_eq!(
        event["envelope"]["data"]["after_ids"],
        serde_json::json!([a.id])
    );
    assert_eq!(
        event["envelope"]["data"]["added_ids"],
        serde_json::json!([a.id])
    );
    r.documents.add_tag(vault, document.id, a.id).await.unwrap();
    assert_eq!(payloads(db).await, added);

    r.documents
        .set_tags(vault, document.id, &[b.id, b.id])
        .await
        .unwrap();
    let replaced = payloads(db).await;
    let event = replaced
        .iter()
        .rev()
        .find(|row| row["envelope"]["event_type"] == "document.tags_changed")
        .unwrap();
    assert_eq!(
        event["envelope"]["data"]["before_ids"],
        serde_json::json!([a.id])
    );
    assert_eq!(
        event["envelope"]["data"]["after_ids"],
        serde_json::json!([b.id])
    );
    assert_eq!(
        event["envelope"]["data"]["added_ids"],
        serde_json::json!([b.id])
    );
    assert_eq!(
        event["envelope"]["data"]["removed_ids"],
        serde_json::json!([a.id])
    );
    r.documents
        .set_tags(vault, document.id, &[b.id])
        .await
        .unwrap();
    assert_eq!(payloads(db).await, replaced);
    let rejected = r
        .documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                tag_ids: Some(vec![a.id]),
                correspondent_id: Some(Some(Id::new_v4())),
                ..Default::default()
            },
        )
        .await;
    assert!(rejected.is_err());
    assert_eq!(payloads(db).await, replaced);
    assert_eq!(
        r.documents.get_tags(vault, document.id).await.unwrap()[0].id,
        b.id
    );

    r.documents
        .remove_tag(vault, document.id, b.id)
        .await
        .unwrap();
    let removed = payloads(db).await;
    let event = removed
        .iter()
        .rev()
        .find(|row| row["envelope"]["event_type"] == "document.tags_changed")
        .unwrap();
    assert_eq!(
        event["envelope"]["data"]["before_ids"],
        serde_json::json!([b.id])
    );
    assert_eq!(
        event["envelope"]["data"]["after_ids"],
        serde_json::json!([])
    );
    assert_eq!(
        event["envelope"]["data"]["removed_ids"],
        serde_json::json!([b.id])
    );
    r.documents
        .remove_tag(vault, document.id, b.id)
        .await
        .unwrap();
    assert_eq!(payloads(db).await, removed);
}

#[tokio::test]
async fn sqlite_tag_entry_points_emit_exact_deltas_without_noops() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("tags.db").display()
    ))
    .await
    .unwrap();
    tag_transition_contract(&db).await;
}

#[tokio::test]
async fn postgres_tag_entry_points_emit_exact_deltas_without_noops() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL tag transitions", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    tag_transition_contract(&db).await;
}

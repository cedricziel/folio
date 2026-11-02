mod common;

use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, UpdateDocument, UpdateTag,
    UserId,
};
use folio_core::mutation::with_actor;
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn taxonomy_contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let actor = UserId::from("taxonomy-editor");
    let tag = with_actor(
        Some(actor.clone()),
        r.tags.create(
            vault,
            CreateTag {
                name: "History".into(),
                color: None,
            },
        ),
    )
    .await
    .unwrap();
    assert_eq!(tag.created_by, Some(actor.clone()));
    assert_eq!(tag.updated_by, Some(actor));
    assert_eq!(tag.created_at, tag.updated_at);
    let tag = r.tags.find_by_id(vault, tag.id).await.unwrap().unwrap();
    let correspondent = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Sender".into(),
            },
        )
        .await
        .unwrap();
    let kind = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Receipt".into(),
            },
        )
        .await
        .unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Kept".into(),
                correspondent_id: Some(correspondent.id),
                document_type_id: Some(kind.id),
                tag_ids: vec![tag.id],
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let unchanged = r
        .tags
        .update(vault, tag.id, UpdateTag::default())
        .await
        .unwrap();
    assert_eq!(unchanged.updated_at, tag.updated_at);
    r.tags.delete(vault, tag.id).await.unwrap();
    r.tags.delete(vault, tag.id).await.unwrap();
    assert!(r.tags.find_by_id(vault, tag.id).await.unwrap().is_none());
    let retained = r.tags.find_retained(vault, tag.id).await.unwrap().unwrap();
    assert!(retained.deleted_at.is_some());
    assert_eq!(r.tags.list_trash(vault).await.unwrap().len(), 1);
    let replacement = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "History".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        r.tags.restore(vault, tag.id).await,
        Err(folio_core::Error::Duplicate { .. }) | Err(folio_core::Error::Conflict(_))
    ));
    assert!(r
        .tags
        .find_retained(vault, tag.id)
        .await
        .unwrap()
        .unwrap()
        .deleted_at
        .is_some());
    r.tags.delete(vault, replacement.id).await.unwrap();
    assert_eq!(
        r.documents.get_tags(vault, document.id).await.unwrap()[0].id,
        tag.id
    );
    assert!(r.documents.get_tags(vault, document.id).await.unwrap()[0]
        .deleted_at
        .is_some());
    let unclassified = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Unclassified".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(r
        .documents
        .add_tag(vault, unclassified.id, tag.id)
        .await
        .is_err());
    assert!(r
        .documents
        .set_tags(vault, document.id, &[tag.id])
        .await
        .is_err());
    r.correspondents
        .delete(vault, correspondent.id)
        .await
        .unwrap();
    r.document_types.delete(vault, kind.id).await.unwrap();
    assert!(r.correspondents.list(vault).await.unwrap().is_empty());
    assert!(r.document_types.list(vault).await.unwrap().is_empty());
    assert_eq!(
        r.correspondents.list_trash(vault).await.unwrap()[0].id,
        correspondent.id
    );
    assert_eq!(
        r.document_types.list_trash(vault).await.unwrap()[0].id,
        kind.id
    );
    assert!(r
        .documents
        .update(
            vault,
            unclassified.id,
            UpdateDocument {
                correspondent_id: Some(Some(correspondent.id)),
                ..Default::default()
            }
        )
        .await
        .is_err());
    assert!(r
        .documents
        .update(
            vault,
            unclassified.id,
            UpdateDocument {
                document_type_id: Some(Some(kind.id)),
                ..Default::default()
            }
        )
        .await
        .is_err());
    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("Still kept".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let kept = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.correspondent_id, Some(correspondent.id));
    assert_eq!(kept.document_type_id, Some(kind.id));
    r.correspondents
        .restore(vault, correspondent.id)
        .await
        .unwrap();
    r.document_types.restore(vault, kind.id).await.unwrap();
    let restored = r.tags.restore(vault, tag.id).await.unwrap();
    assert!(restored.deleted_at.is_none());
    assert_eq!(restored.created_at, tag.created_at);
    assert_eq!(restored.created_by, tag.created_by);
    let restored = r.tags.find_by_id(vault, tag.id).await.unwrap().unwrap();
    let unchanged = r.tags.restore(vault, tag.id).await.unwrap();
    assert_eq!(unchanged.updated_at, restored.updated_at);
    let updated = with_actor(
        Some(UserId::from("renamer")),
        r.tags.update(
            vault,
            tag.id,
            UpdateTag {
                name: Some("Renamed history".into()),
                color: None,
            },
        ),
    )
    .await
    .unwrap();
    assert_eq!(updated.created_at, tag.created_at);
    assert_eq!(updated.created_by, tag.created_by);
    assert_eq!(updated.updated_by, Some(UserId::from("renamer")));
    let events = r
        .outbox
        .claim(100, chrono::Duration::seconds(30), 10)
        .await
        .unwrap();
    let transitions: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| event.subject().id == tag.id)
        .map(|event| event.event_type())
        .collect();
    assert_eq!(
        transitions,
        ["tag.created", "tag.deleted", "tag.restored", "tag.updated"]
    );
    let created = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .find(|event| event.subject().id == tag.id && event.event_type() == "tag.created")
        .unwrap();
    assert_eq!(created.actor_id(), tag.created_by.as_ref());
    assert_eq!(created.document_id(), None);
    assert_eq!(created.data()["name"], "History");
    let oversized_name = "x".repeat(70_000);
    assert!(r
        .tags
        .create(
            vault,
            CreateTag {
                name: oversized_name.clone(),
                color: None
            }
        )
        .await
        .is_err());
    assert!(r
        .tags
        .find_by_name(vault, &oversized_name)
        .await
        .unwrap()
        .is_none());
    assert!(r
        .outbox
        .claim(100, chrono::Duration::seconds(30), 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn sqlite_taxonomy_retains_identity_and_conflicts_on_restore() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("taxonomy.db").display()
    ))
    .await
    .unwrap();
    taxonomy_contract(db).await;
}

#[tokio::test]
async fn postgres_taxonomy_retains_identity_and_conflicts_on_restore() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL taxonomy contract",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    taxonomy_contract(
        Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}

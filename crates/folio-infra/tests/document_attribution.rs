mod common;

use folio_core::models::{
    CreateDocument, CreateTag, ProcessingStage, ProcessingStatus, UpdateDocument, UserId,
};
use folio_core::mutation::with_actor;
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn attribution_contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let creator = UserId::from("creator");
    let uploader = UserId::from("contributor-owner");
    let document = with_actor(
        Some(creator.clone()),
        r.documents.create(
            vault,
            CreateDocument {
                title: "Retained history".into(),
                content: Some("Initial text".into()),
                uploaded_by: Some(uploader.clone()),
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    assert_eq!(document.created_by, Some(creator.clone()));
    assert_eq!(document.updated_by, Some(creator.clone()));
    assert_eq!(document.uploaded_by, Some(uploader.clone()));
    assert_eq!(document.created_at, document.updated_at);
    let persisted = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.created_by, document.created_by);
    assert_eq!(persisted.updated_by, document.updated_by);

    let editor = UserId::from("editor");
    let staged = with_actor(
        Some(creator.clone()),
        r.documents.create_for_upload(
            vault,
            CreateDocument {
                title: "Staged upload".into(),
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    with_actor(
        Some(creator.clone()),
        r.documents.finalize_upload(vault, staged.id, "staged.pdf"),
    )
    .await
    .unwrap();
    let finalized = r
        .documents
        .find_by_id(vault, staged.id)
        .await
        .unwrap()
        .unwrap();
    with_actor(
        Some(editor.clone()),
        r.documents.finalize_upload(vault, staged.id, "staged.pdf"),
    )
    .await
    .unwrap();
    let retried = r
        .documents
        .find_by_id(vault, staged.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retried.created_by, Some(creator.clone()));
    assert_eq!(retried.updated_by, finalized.updated_by);
    assert_eq!(retried.updated_at, finalized.updated_at);

    let unchanged = with_actor(
        Some(editor.clone()),
        r.documents
            .update(vault, document.id, UpdateDocument::default()),
    )
    .await
    .unwrap();
    assert_eq!(unchanged.updated_at, document.updated_at);
    assert_eq!(unchanged.updated_by, document.updated_by);
    let updated = with_actor(
        Some(editor.clone()),
        r.documents.update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("Changed title".into()),
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    assert_eq!(updated.created_at, document.created_at);
    assert_eq!(updated.created_by, document.created_by);
    assert_eq!(updated.updated_by, Some(editor.clone()));
    assert_eq!(updated.uploaded_by, Some(uploader.clone()));

    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "Audit tag".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    with_actor(
        Some(creator.clone()),
        r.documents.add_tag(vault, document.id, tag.id),
    )
    .await
    .unwrap();
    let tagged = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tagged.updated_by, Some(creator.clone()));
    with_actor(
        Some(editor.clone()),
        r.documents.add_tag(vault, document.id, tag.id),
    )
    .await
    .unwrap();
    let same_tag = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(same_tag.updated_by, tagged.updated_by);
    assert_eq!(same_tag.updated_at, tagged.updated_at);
    with_actor(
        Some(editor.clone()),
        r.documents.remove_tag(vault, document.id, tag.id),
    )
    .await
    .unwrap();
    let untagged = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untagged.updated_by, Some(editor.clone()));
    with_actor(
        Some(creator.clone()),
        r.documents.remove_tag(vault, document.id, tag.id),
    )
    .await
    .unwrap();
    let same_untagged = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(same_untagged.updated_by, untagged.updated_by);
    assert_eq!(same_untagged.updated_at, untagged.updated_at);

    with_actor(
        Some(creator.clone()),
        r.documents
            .set_file_path(vault, document.id, "original.pdf"),
    )
    .await
    .unwrap();
    with_actor(
        Some(creator.clone()),
        r.documents.set_page_count(vault, document.id, 2),
    )
    .await
    .unwrap();
    let processed = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    with_actor(
        Some(editor.clone()),
        r.documents
            .set_file_path(vault, document.id, "original.pdf"),
    )
    .await
    .unwrap();
    with_actor(
        Some(editor.clone()),
        r.documents.set_page_count(vault, document.id, 2),
    )
    .await
    .unwrap();
    with_actor(
        Some(editor.clone()),
        r.documents.complete_ocr(vault, document.id, "Initial text"),
    )
    .await
    .unwrap();
    with_actor(
        Some(editor.clone()),
        r.documents.set_stage_status(
            vault,
            document.id,
            ProcessingStage::Ocr,
            ProcessingStatus::Done,
        ),
    )
    .await
    .unwrap();
    let no_op = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(no_op.updated_at, processed.updated_at);
    assert_eq!(no_op.updated_by, processed.updated_by);
    with_actor(
        None,
        r.documents.complete_ocr(vault, document.id, "Worker text"),
    )
    .await
    .unwrap();
    let system = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(system.created_by, Some(creator.clone()));
    assert_eq!(system.updated_by, None);
    assert_eq!(system.uploaded_by, Some(uploader.clone()));
    with_actor(
        Some(editor.clone()),
        r.documents.set_stage_status(
            vault,
            document.id,
            ProcessingStage::Embed,
            ProcessingStatus::Running,
        ),
    )
    .await
    .unwrap();
    with_actor(Some(editor.clone()), r.documents.reset_embedding_status())
        .await
        .unwrap();
    let requeued = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requeued.updated_by, None);
    assert_eq!(requeued.updated_at, system.updated_at);

    with_actor(Some(editor.clone()), r.documents.delete(vault, document.id))
        .await
        .unwrap();
    let trashed = r
        .documents
        .find_trashed_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trashed.updated_by, Some(editor.clone()));
    with_actor(
        Some(creator.clone()),
        r.documents.delete(vault, document.id),
    )
    .await
    .unwrap();
    let same_trash = r
        .documents
        .find_trashed_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(same_trash.updated_at, trashed.updated_at);
    assert_eq!(same_trash.updated_by, trashed.updated_by);
    let restored = with_actor(
        Some(creator.clone()),
        r.documents.restore(vault, document.id),
    )
    .await
    .unwrap();
    assert_eq!(restored.updated_by, Some(creator.clone()));
    assert_eq!(restored.created_by, Some(creator.clone()));
    assert_eq!(restored.created_at, document.created_at);
    let same_restore = with_actor(
        Some(editor.clone()),
        r.documents.restore(vault, document.id),
    )
    .await
    .unwrap();
    assert_eq!(same_restore.updated_at, restored.updated_at);
    assert_eq!(same_restore.updated_by, restored.updated_by);

    let target = r.vaults.create("Move target", None).await.unwrap().id;
    let moved = r
        .documents
        .move_to(document.id, vault, target, Some(&editor))
        .await
        .unwrap();
    assert_eq!(moved.created_at, document.created_at);
    assert_eq!(moved.created_by, Some(creator));
    assert_eq!(moved.updated_by, Some(editor));
    assert_eq!(moved.uploaded_by, Some(uploader));
}

#[tokio::test]
async fn sqlite_document_attribution_tracks_committed_changes_and_preserves_creator() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("actors.db").display()
    ))
    .await
    .unwrap();
    attribution_contract(db).await;
}

#[tokio::test]
async fn postgres_document_attribution_tracks_committed_changes_and_preserves_creator() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL document attribution contract",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    attribution_contract(
        Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}

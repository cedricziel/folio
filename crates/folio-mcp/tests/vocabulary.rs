mod common;

use common::Fixture;
use folio_core::services::{MetadataUpdate, VocabularyPolicy};

async fn assert_missing_metadata_is_rejected(mut update: MetadataUpdate) {
    let fx = Fixture::new().await;
    let document = fx.insert("Original title", "").await;
    update.title = Some("Must not change".into());

    let result = fx
        .library
        .update_metadata(
            fx.vault,
            document.id,
            update,
            VocabularyPolicy::ExistingOnly,
        )
        .await;

    assert!(matches!(result, Err(folio_core::Error::Validation(_))));
    assert!(fx.library.list_tags(fx.vault).await.unwrap().is_empty());
    assert!(fx
        .library
        .list_correspondents(fx.vault)
        .await
        .unwrap()
        .is_empty());
    assert!(fx
        .library
        .list_document_types(fx.vault)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        fx.library
            .get_document(fx.vault, document.id)
            .await
            .unwrap()
            .document
            .title,
        "Original title"
    );
}

#[tokio::test]
async fn existing_only_metadata_does_not_create_a_missing_tag() {
    assert_missing_metadata_is_rejected(MetadataUpdate {
        tags: Some(vec!["Missing tag".into()]),
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn existing_only_metadata_does_not_create_a_missing_correspondent() {
    assert_missing_metadata_is_rejected(MetadataUpdate {
        correspondent: Some(Some("Missing correspondent".into())),
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn existing_only_metadata_does_not_create_a_missing_document_type() {
    assert_missing_metadata_is_rejected(MetadataUpdate {
        document_type: Some(Some("Missing type".into())),
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn existing_only_add_tags_rejects_missing_names_without_attaching_earlier_tags() {
    let fx = Fixture::new().await;
    let seed = fx.insert("Vocabulary seed", "").await;
    let target = fx.insert("Target", "").await;
    fx.library
        .add_tags(
            fx.vault,
            seed.id,
            &["Existing".into()],
            VocabularyPolicy::CreateMissing,
        )
        .await
        .unwrap();

    let result = fx
        .library
        .add_tags(
            fx.vault,
            target.id,
            &["Existing".into(), "Missing".into()],
            VocabularyPolicy::ExistingOnly,
        )
        .await;

    assert!(matches!(result, Err(folio_core::Error::Validation(_))));
    assert_eq!(fx.library.list_tags(fx.vault).await.unwrap().len(), 1);
    assert!(fx
        .library
        .get_document(fx.vault, target.id)
        .await
        .unwrap()
        .tags
        .is_empty());
}

#[tokio::test]
async fn existing_only_metadata_reuses_trimmed_names_without_creating_vocabulary() {
    let fx = Fixture::new().await;
    let seed = fx.insert("Vocabulary seed", "").await;
    let target = fx.insert("Target", "").await;
    fx.library
        .update_metadata(
            fx.vault,
            seed.id,
            MetadataUpdate {
                tags: Some(vec!["Existing".into()]),
                correspondent: Some(Some("ACME".into())),
                document_type: Some(Some("Invoice".into())),
                ..Default::default()
            },
            VocabularyPolicy::CreateMissing,
        )
        .await
        .unwrap();

    let updated = fx
        .library
        .update_metadata(
            fx.vault,
            target.id,
            MetadataUpdate {
                tags: Some(vec![" existing ".into()]),
                correspondent: Some(Some(" acme ".into())),
                document_type: Some(Some(" invoice ".into())),
                ..Default::default()
            },
            VocabularyPolicy::ExistingOnly,
        )
        .await
        .unwrap();

    assert_eq!(updated.tags[0].name, "Existing");
    assert!(updated.document.correspondent_id.is_some());
    assert!(updated.document.document_type_id.is_some());
    assert_eq!(fx.library.list_tags(fx.vault).await.unwrap().len(), 1);
    assert_eq!(
        fx.library
            .list_correspondents(fx.vault)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fx.library
            .list_document_types(fx.vault)
            .await
            .unwrap()
            .len(),
        1
    );
}

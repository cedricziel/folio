use folio_core::{models::Id, ports::keys};

#[test]
fn replacement_keys_cannot_target_previous_original_or_another_document() {
    let document = Id::from_u128(1);
    let other_document = Id::from_u128(2);
    let first = Id::from_u128(3);
    let second = Id::from_u128(4);
    let first_key = keys::revision_original(document, first);
    assert_ne!(first_key, keys::original(document));
    assert_ne!(first_key, keys::revision_original(document, second));
    assert_ne!(first_key, keys::revision_original(other_document, first));
    assert!(first_key.starts_with(&keys::document_prefix(document)));
}

#[test]
fn retained_outputs_use_new_keys_for_each_generation() {
    let document = Id::from_u128(1);
    let revision = Id::from_u128(2);
    let first = keys::revision_artifact(document, revision, Id::from_u128(3));
    let second = keys::revision_artifact(document, revision, Id::from_u128(4));
    assert_ne!(first, second);
    assert_ne!(first, keys::revision_original(document, revision));
    assert!(first.starts_with(&keys::document_prefix(document)));
}

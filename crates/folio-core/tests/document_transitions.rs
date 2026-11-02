use folio_core::document_transitions::{document_changes, DocumentChange};
use folio_core::models::{Document, Id, VaultId};

#[test]
fn replacing_tags_reports_only_added_and_removed_ids() {
    let document = Document::new(VaultId::new(), "invoice");
    let a = Id::from_u128(1);
    let b = Id::from_u128(2);
    let c = Id::from_u128(3);
    assert_eq!(
        document_changes(&document, &[b, a], &document, &[c, b, c]),
        vec![DocumentChange::TagsChanged {
            before_ids: vec![a, b],
            after_ids: vec![b, c],
            added_ids: vec![c],
            removed_ids: vec![a],
        }]
    );
}

#[test]
fn reordered_duplicate_tags_are_a_noop() {
    let document = Document::new(VaultId::new(), "invoice");
    let a = Id::from_u128(1);
    let b = Id::from_u128(2);
    assert!(document_changes(&document, &[a, b], &document, &[b, a, b]).is_empty());
}

#[test]
fn classification_assignment_and_clearing_preserve_nulls() {
    let mut before = Document::new(VaultId::new(), "invoice");
    before.document_type_id = Some(Id::from_u128(2));
    let mut after = before.clone();
    after.correspondent_id = Some(Id::from_u128(1));
    after.document_type_id = None;
    assert_eq!(
        document_changes(&before, &[], &after, &[]),
        vec![
            DocumentChange::CorrespondentChanged {
                before_id: None,
                after_id: Some(Id::from_u128(1))
            },
            DocumentChange::DocumentTypeChanged {
                before_id: Some(Id::from_u128(2)),
                after_id: None
            },
        ]
    );
}

#[test]
fn title_delta_preserves_exact_occurrence_values() {
    let before = Document::new(VaultId::new(), "invoice");
    let mut after = before.clone();
    after.title = "Invoice ".into();
    assert_eq!(
        document_changes(&before, &[], &after, &[]),
        vec![DocumentChange::TitleChanged {
            before: "invoice".into(),
            after: "Invoice ".into()
        }]
    );
}

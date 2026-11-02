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

#[test]
fn metadata_batches_reject_cross_vault_snapshots() {
    let document = Document::new(VaultId::new(), "Before");
    let before = folio_core::EventSnapshot::from(&folio_core::DeletedDocument {
        document: document.clone(),
        tags: Vec::new(),
    });
    let mut after = before.clone();
    after.vault_id = VaultId::new();
    assert!(folio_core::document_transitions::metadata_envelopes(&before, &after).is_err());
}

#[tokio::test]
async fn successive_mutations_keep_lineage_without_sharing_mutation_identity() {
    let before = folio_core::EventSnapshot::from(&folio_core::DeletedDocument {
        document: Document::new(VaultId::new(), "Before"),
        tags: Vec::new(),
    });
    let mut after = before.clone();
    after.title = "After".into();
    let lineage = folio_core::mutation::MutationLineage {
        correlation_id: Id::from_u128(101),
        causation_id: Some(Id::from_u128(102)),
    };
    let (first, second) = folio_core::mutation::with_lineage(Some(lineage), async {
        (
            folio_core::document_transitions::metadata_envelopes(&before, &after).unwrap(),
            folio_core::document_transitions::metadata_envelopes(&after, &before).unwrap(),
        )
    })
    .await;
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert_eq!(first[0].mutation_id(), first[1].mutation_id());
    assert_eq!(second[0].mutation_id(), second[1].mutation_id());
    assert_ne!(first[0].mutation_id(), second[0].mutation_id());
    assert_eq!(first[0].correlation_id(), lineage.correlation_id);
    assert_eq!(second[0].correlation_id(), lineage.correlation_id);
    assert_eq!(first[0].causation_id(), lineage.causation_id);
    assert_eq!(second[0].causation_id(), lineage.causation_id);
}

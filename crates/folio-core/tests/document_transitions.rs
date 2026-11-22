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

#[tokio::test]
async fn file_revision_occurrences_inherit_actor_and_lineage_and_clear_review() {
    use folio_core::document_transitions::{revision_publication_envelopes, RevisionOccurrence};
    use folio_core::models::{FileIdentity, ReviewOrigin, UserId};
    use folio_core::mutation::{with_actor, with_lineage, MutationLineage};

    let mut document = Document::new(VaultId::new(), "Invoice");
    document.reviewed_at = Some(chrono::Utc::now());
    document.reviewed_by = Some(UserId::from("previous-reviewer"));
    document.review_origin = Some(ReviewOrigin::Human);
    let before = folio_core::EventSnapshot::from(&folio_core::DeletedDocument {
        document,
        tags: Vec::new(),
    });
    let mut after = before.clone();
    after.reviewed_at = None;
    after.reviewed_by = None;
    after.review_origin = None;
    after.original_filename = Some("invoice.pdf".into());
    let origin = FileIdentity::from_bytes("private/original", "application/pdf", b"invoice");
    let actor = UserId::from("editor");
    let lineage = MutationLineage {
        correlation_id: Id::from_u128(100),
        causation_id: Some(Id::from_u128(101)),
    };
    let previous_revision_id = Id::from_u128(10);

    for (restored_from_revision_id, reason, number) in [
        (None, "file_replaced", 2),
        (Some(Id::from_u128(9)), "file_restored", 3),
    ] {
        let occurrence = RevisionOccurrence {
            revision_id: Id::from_u128(11),
            revision_number: number,
            previous_revision_id: Some(previous_revision_id),
            restored_from_revision_id,
            original: &origin,
        };
        let events = with_actor(
            Some(actor.clone()),
            with_lineage(Some(lineage), async {
                revision_publication_envelopes(&before, &after, occurrence).unwrap()
            }),
        )
        .await;
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type() == "document.updated")
                .count(),
            1
        );
        let mut ids = std::collections::BTreeSet::new();
        for event in &events {
            assert!(ids.insert(event.event_id()));
            assert_eq!(event.actor_id(), Some(&actor));
            assert_eq!(event.vault_id(), after.vault_id);
            assert_eq!(event.document_id(), Some(after.document_id));
            assert_eq!(event.mutation_id(), events[0].mutation_id());
            assert_eq!(event.correlation_id(), lineage.correlation_id);
            assert_eq!(event.causation_id(), lineage.causation_id);
            assert!(!event.data().to_string().contains("private/original"));
        }
        let unreviewed = events
            .iter()
            .find(|event| event.event_type() == "document.unreviewed")
            .unwrap();
        assert_eq!(unreviewed.data()["reason"], reason);
        assert_eq!(
            unreviewed.data()["previous_reviewed_by"],
            "previous-reviewer"
        );
        let created = events
            .iter()
            .find(|event| event.event_type() == "document.revision_created")
            .unwrap();
        assert_eq!(
            created.data()["previous_revision_id"],
            previous_revision_id.to_string()
        );
        assert_eq!(created.data()["revision_number"], number);
        assert_eq!(created.data()["original_filename"], "invoice.pdf");
        assert_eq!(created.data()["sha256"], origin.sha256);
        assert_eq!(created.data()["byte_size"], origin.byte_size);
        let restored = events
            .iter()
            .find(|event| event.event_type() == "document.revision_restored");
        assert_eq!(restored.is_some(), restored_from_revision_id.is_some());
        if let Some(restored) = restored {
            assert_eq!(
                restored.data()["restored_from_revision_id"],
                Id::from_u128(9).to_string()
            );
            assert_eq!(restored.data(), created.data());
        }
    }
}

#[test]
fn publishing_over_an_unreviewed_revision_does_not_emit_an_unreview() {
    use folio_core::document_transitions::{revision_publication_envelopes, RevisionOccurrence};
    use folio_core::models::FileIdentity;

    let document = folio_core::EventSnapshot::from(&folio_core::DeletedDocument {
        document: Document::new(VaultId::new(), "Unreviewed document"),
        tags: Vec::new(),
    });
    let original = FileIdentity::from_bytes("private/replacement", "application/pdf", b"replace");
    let occurrence = RevisionOccurrence {
        revision_id: Id::from_u128(2),
        revision_number: 2,
        previous_revision_id: Some(Id::from_u128(1)),
        restored_from_revision_id: None,
        original: &original,
    };
    let events = revision_publication_envelopes(&document, &document, occurrence).unwrap();
    let types: Vec<_> = events.iter().map(|event| event.event_type()).collect();
    assert_eq!(types, ["document.revision_created", "document.updated"]);
    let restoring_current = RevisionOccurrence {
        restored_from_revision_id: occurrence.previous_revision_id,
        ..occurrence
    };
    assert!(revision_publication_envelopes(&document, &document, restoring_current).is_err());
}

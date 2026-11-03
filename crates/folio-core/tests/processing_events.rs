use chrono::Utc;
use folio_core::models::{DocumentSource, VaultId};
use folio_core::mutation::{with_lineage, MutationLineage};
use folio_core::processing_events::{
    processing_envelope, ProcessingEventOutcome as Outcome, ProcessingEventStage as Stage,
    ProcessingOccurrence, ProcessingReason as Reason,
};
use folio_core::EventSnapshot;
use uuid::Uuid;

fn snapshot() -> EventSnapshot {
    EventSnapshot {
        document_id: Uuid::new_v4(),
        vault_id: VaultId::new(),
        title: "Invoice at completion".into(),
        original_filename: Some("invoice.pdf".into()),
        mime_type: Some("application/pdf".into()),
        file_path: Some("private/storage/key".into()),
        source: DocumentSource::default(),
        created_at: Utc::now(),
        document_date: None,
        reviewed_at: None,
        reviewed_by: None,
        review_origin: None,
        correspondent_id: None,
        correspondent_name: None,
        document_type_id: None,
        document_type_name: None,
        tags: Vec::new(),
    }
}

fn occurrence(stage: Stage, outcome: Outcome) -> ProcessingOccurrence {
    ProcessingOccurrence {
        run_id: Uuid::new_v4(),
        revision_id: None,
        stage,
        attempt: 1,
        outcome,
    }
}

#[test]
fn retry_and_terminal_failure_are_distinct_occurrences() {
    let snapshot = snapshot();
    let mut transition = occurrence(
        Stage::Embedding,
        Outcome::RetryScheduled {
            reason: Reason::ProviderFailed,
            retry_at: Utc::now(),
        },
    );
    let retry = processing_envelope(&snapshot, &transition).unwrap();
    assert_eq!(
        retry.event_type(),
        "document.processing.embedding.retry_scheduled"
    );
    assert!(retry.data().get("retry_at").is_some());
    transition.outcome = Outcome::Failed {
        reason: Reason::ProviderFailed,
    };
    let failed = processing_envelope(&snapshot, &transition).unwrap();
    assert_eq!(failed.event_type(), "document.processing.embedding.failed");
    assert!(failed.data().get("retry_at").is_none());
    assert_eq!(retry.data()["run_id"], failed.data()["run_id"]);
    assert_ne!(retry.mutation_id(), failed.mutation_id());
}

#[test]
fn empty_ocr_completes_and_only_embedding_skips_for_no_text() {
    let snapshot = snapshot();
    let complete = processing_envelope(
        &snapshot,
        &occurrence(
            Stage::Ocr,
            Outcome::Completed {
                has_text: Some(false),
            },
        ),
    )
    .unwrap();
    assert_eq!(complete.event_type(), "document.processing.ocr.completed");
    assert_eq!(complete.data()["has_text"], false);
    assert!(complete.data()["revision_id"].is_null());
    let skipped = processing_envelope(
        &snapshot,
        &occurrence(
            Stage::Embedding,
            Outcome::Skipped {
                reason: Reason::NoText,
            },
        ),
    )
    .unwrap();
    assert_eq!(skipped.data()["reason_code"], "no_text");
    assert!(processing_envelope(
        &snapshot,
        &occurrence(
            Stage::Ocr,
            Outcome::Skipped {
                reason: Reason::NoText
            }
        )
    )
    .is_err());
    assert!(processing_envelope(
        &snapshot,
        &occurrence(
            Stage::Thumbnail,
            Outcome::Completed {
                has_text: Some(true)
            }
        )
    )
    .is_err());
    assert!(processing_envelope(
        &snapshot,
        &occurrence(Stage::Ocr, Outcome::Completed { has_text: None })
    )
    .is_err());
}

#[test]
fn skipped_configuration_is_never_reported_as_failure() {
    let snapshot = snapshot();
    for stage in [Stage::Ocr, Stage::Embedding, Stage::Thumbnail] {
        let skipped = processing_envelope(
            &snapshot,
            &occurrence(
                stage,
                Outcome::Skipped {
                    reason: Reason::ProviderDisabled,
                },
            ),
        )
        .unwrap();
        assert_eq!(skipped.data()["reason_code"], "provider_disabled");
        assert!(processing_envelope(
            &snapshot,
            &occurrence(
                stage,
                Outcome::Failed {
                    reason: Reason::ProviderDisabled
                }
            )
        )
        .is_err());
    }
    assert!(processing_envelope(
        &snapshot,
        &occurrence(
            Stage::Thumbnail,
            Outcome::Skipped {
                reason: Reason::UnsupportedFormat
            }
        )
    )
    .is_ok());
}

#[tokio::test]
async fn occurrence_retains_trusted_lineage_and_immutable_allowlisted_metadata() {
    let mut snapshot = snapshot();
    let lineage = MutationLineage {
        correlation_id: Uuid::new_v4(),
        causation_id: Some(Uuid::new_v4()),
    };
    let event = with_lineage(Some(lineage), async {
        processing_envelope(&snapshot, &occurrence(Stage::Ocr, Outcome::Started)).unwrap()
    })
    .await;
    snapshot.title = "Later title".into();
    assert_eq!(event.data()["title"], "Invoice at completion");
    assert_eq!(event.correlation_id(), lineage.correlation_id);
    assert_eq!(event.causation_id(), lineage.causation_id);
    assert_eq!(event.actor_id(), None);
    assert!(event.data().get("file_path").is_none());
    assert!(event.data().get("content").is_none());
    assert!(event.data().get("error").is_none());
    assert_eq!(event.vault_id(), snapshot.vault_id);
    assert_eq!(event.document_id(), Some(snapshot.document_id));
}

#[test]
fn malformed_attempt_identity_cannot_be_published() {
    let snapshot = snapshot();
    let mut event = occurrence(Stage::Thumbnail, Outcome::Started);
    event.attempt = 0;
    assert!(processing_envelope(&snapshot, &event).is_err());
    event.attempt = 1;
    event.revision_id = Some(Uuid::nil());
    assert!(processing_envelope(&snapshot, &event).is_err());
    event.revision_id = Some(Uuid::new_v4());
    assert!(processing_envelope(&snapshot, &event).is_ok());
    event.run_id = Uuid::nil();
    assert!(processing_envelope(&snapshot, &event).is_err());
}

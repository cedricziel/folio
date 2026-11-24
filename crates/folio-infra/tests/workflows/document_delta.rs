use super::*;
use serde_json::json;

#[tokio::test]
async fn tag_replacement_matches_the_committed_delta_without_matching_retained_tags() {
    let h = Harness::new().await;
    let removed = h.tag("removed").await;
    let retained = h.tag("retained").await;
    let added = h.tag("added").await;
    let doc = h.document("classification.pdf", "application/pdf").await;
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                tag_ids: Some(vec![retained, removed]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _ = h.pending_events().await;

    let wants_replacement = h
        .workflow(
            WorkflowTrigger::DocumentTagsChanged,
            serde_json::from_value(json!({
                "event": { "document_change": {
                    "type": "tags", "state": "replaced", "added_id": added, "removed_id": removed
                }}
            }))
            .unwrap(),
            unreachable_hook(),
        )
        .await;
    h.workflow(
        WorkflowTrigger::DocumentTagsChanged,
        serde_json::from_value(json!({
            "event": { "document_change": { "type": "tags", "added_id": retained } }
        }))
        .unwrap(),
        unreachable_hook(),
    )
    .await;
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                tag_ids: Some(vec![retained, added]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let replacement = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|e| e.event_type() == "document.tags_changed")
        })
        .unwrap();
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                tag_ids: Some(vec![retained]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let runs = h.engine.handle(&replacement).await.unwrap();

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, wants_replacement.id);
    assert!(h.engine.handle(&replacement).await.unwrap().is_empty());
}

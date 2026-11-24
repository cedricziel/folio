use super::*;
use serde_json::Value;

#[tokio::test]
async fn purge_keeps_legacy_deliveries_without_duplicating_multiple_trigger_workflows() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let purged = h
        .workflow(
            WorkflowTrigger::DocumentPurged,
            Default::default(),
            vec![webhook(server.url("purged"), None)],
        )
        .await;
    let deleted = h
        .workflow(
            WorkflowTrigger::DocumentDeleted,
            Default::default(),
            vec![webhook(server.url("deleted"), None)],
        )
        .await;
    let both = h
        .workflows
        .create(
            h.vault,
            WorkflowInput {
                cadence: None,
                name: "Purge with legacy alias".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentDeleted,
                triggers: vec![
                    WorkflowTrigger::DocumentDeleted,
                    WorkflowTrigger::DocumentPurged,
                ],
                conditions: Default::default(),
                actions: vec![webhook(server.url("both"), None)],
            },
        )
        .await
        .unwrap();
    let doc = h.document("retained.pdf", "application/pdf").await;
    h.documents.delete(h.vault, doc.id).await.unwrap();
    h.documents.request_purge(h.vault, doc.id).await.unwrap();
    h.documents.finalize_purge(h.vault, doc.id).await.unwrap();
    let event = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| matches!(event.event, Some(DocumentEvent::Deleted { .. })))
        .unwrap();
    let event_id = event.envelope.as_ref().unwrap().event_id();

    let (first, second) = tokio::join!(h.engine.handle(&event), h.engine.handle(&event));
    let runs: Vec<_> = first.unwrap().into_iter().chain(second.unwrap()).collect();

    assert_eq!(runs.len(), 3);
    for (workflow, expected) in [
        (purged, WorkflowTrigger::DocumentPurged),
        (deleted, WorkflowTrigger::DocumentDeleted),
        (both, WorkflowTrigger::DocumentPurged),
    ] {
        let run = runs
            .iter()
            .find(|run| run.workflow_id == workflow.id)
            .unwrap();
        let payload: Value = serde_json::from_str(&run.payload).unwrap();
        assert_eq!(payload["event"], expected.as_str());
        assert_eq!(payload["source_event"]["event_id"], event_id.to_string());
        assert_eq!(payload["document"]["title"], "retained.pdf");
        assert_eq!(
            h.runner.execute(run.id).await.unwrap(),
            RunOutcome::Succeeded
        );
    }
    assert!(h.engine.handle(&event).await.unwrap().is_empty());
    assert_eq!(server.hits("purged"), 1);
    assert_eq!(server.hits("deleted"), 1);
    assert_eq!(server.hits("both"), 1);
}

#[tokio::test]
async fn move_out_webhook_matches_and_delivers_only_the_departure_snapshot() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let source_tag = h.tag("source tag").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentMovedOut,
            WorkflowConditions {
                tag_id: Some(source_tag),
                ..Default::default()
            },
            vec![webhook(server.url("moved-out"), None)],
        )
        .await;
    let doc = h.document("moved.pdf", "application/pdf").await;
    h.documents
        .add_tag(h.vault, doc.id, source_tag)
        .await
        .unwrap();
    let destination = h.vaults.create("destination", None).await.unwrap().id;
    h.documents
        .move_to(doc.id, h.vault, destination, None)
        .await
        .unwrap();
    let remapped = h.documents.get_tags(destination, doc.id).await.unwrap();
    assert_ne!(remapped[0].id, source_tag);
    let event = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|e| e.event_type() == "document.moved_out")
        })
        .unwrap();

    let runs = h.engine.handle(&event).await.unwrap();

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
    assert_eq!(
        h.runner.execute(runs[0].id).await.unwrap(),
        RunOutcome::Succeeded
    );
    let received = server.state.received.lock().unwrap();
    assert_eq!(received.len(), 1);
    let body = &received[0].body;
    let payload: Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["document"]["tags"][0]["id"], source_tag.to_string());
    assert!(!body.contains(&remapped[0].id.to_string()));
    assert!(!body.contains(&destination.to_string()));
}

#[tokio::test]
async fn queued_move_in_workflow_cannot_modify_a_document_after_it_leaves() {
    let h = Harness::new().await;
    let source = h.vaults.create("source", None).await.unwrap().id;
    let workflow_tag = h.tag("arrival tag").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentMovedIn,
            Default::default(),
            vec![WorkflowAction::AddTags {
                tag_ids: vec![workflow_tag],
            }],
        )
        .await;
    let doc = h
        .documents
        .create(
            source,
            CreateDocument {
                title: "Moving twice".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.documents
        .move_to(doc.id, source, h.vault, None)
        .await
        .unwrap();
    let event = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|e| e.event_type() == "document.moved_in")
        })
        .unwrap();
    let runs = h.engine.handle(&event).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
    h.documents
        .move_to(doc.id, h.vault, source, None)
        .await
        .unwrap();

    assert_eq!(
        h.runner.execute(runs[0].id).await.unwrap(),
        RunOutcome::Failed
    );

    let failed = h.run(runs[0].id).await;
    assert_eq!(failed.completed_actions, 0);
    assert!(failed.last_error.unwrap().contains("workflow's vault"));
    assert!(h
        .documents
        .get_tags(source, doc.id)
        .await
        .unwrap()
        .is_empty());
}

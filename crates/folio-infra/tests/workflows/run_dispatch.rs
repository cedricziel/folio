use super::{unreachable_hook, Harness};
use chrono::Utc;
use folio_core::models::{RunStatus, WorkflowInput, WorkflowRun, WorkflowTrigger};
use folio_core::{EventOrigin, OutboxEvent};
use serde_json::json;

#[tokio::test]
async fn owner_outcomes_match_the_captured_workflow_id() {
    let h = Harness::new().await;
    let producing = h
        .webhook_workflow("https://example.test".into(), None)
        .await;
    let listen = |workflow_id| -> WorkflowInput {
        serde_json::from_value(json!({
            "name": "Listener",
            "trigger": "workflow_run.failed",
            "conditions": {"all": [{"event": {"workflow_id": workflow_id}}]},
            "actions": unreachable_hook(),
        }))
        .unwrap()
    };
    let matching = h
        .workflows
        .create(h.vault, listen(producing.id))
        .await
        .unwrap();
    h.workflows
        .create(h.vault, listen(matching.id))
        .await
        .unwrap();
    let foreign_vault = h.vaults.create("Unrelated vault", None).await.unwrap().id;
    h.workflows
        .create(foreign_vault, listen(producing.id))
        .await
        .unwrap();
    let mut run = WorkflowRun::new(
        h.vault,
        producing.id,
        WorkflowTrigger::DocumentUploaded,
        None,
        "{}".into(),
    )
    .with_definition(&producing);
    h.workflows.create_run(&run).await.unwrap();
    run.status = RunStatus::Failed;
    run.finished_at = Some(Utc::now());
    h.workflows.update_run(&run).await.unwrap();
    let outcome = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|source| {
                source.event_type() == "workflow_run.failed" && source.subject().id == run.id
            })
        })
        .expect("the repository published the terminal occurrence");
    assert!(h
        .engine
        .handle(&OutboxEvent {
            origin: EventOrigin::User,
            ..outcome.clone()
        })
        .await
        .unwrap()
        .is_empty());
    let runs = h.engine.handle(&outcome).await.unwrap();
    assert_eq!(
        runs.iter().map(|run| run.workflow_id).collect::<Vec<_>>(),
        [matching.id]
    );
}

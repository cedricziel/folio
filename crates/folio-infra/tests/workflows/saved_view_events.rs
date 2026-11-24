use super::*;
use folio_auth::User;
use folio_core::models::{
    MetadataExactFilter, MetadataFieldInput, MetadataKind, MetadataValue, SavedViewInput,
    SavedViewQuery, UserId, VaultId, VaultRole, ViewPresentation, ViewVisibility,
    WorkflowActorCondition, WorkflowEventConditions,
};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::ports::SavedViewAccess;
use folio_core::EventAudience;
use folio_infra::Repositories;
use serde_json::json;

#[tokio::test]
async fn sqlite_saved_view_workflows_respect_occurrence_and_current_visibility() {
    let (_directory, database) = common::sqlite_database().await;
    shared_saved_view_contract(database.repositories().await).await;
}

#[tokio::test]
async fn postgres_saved_view_workflows_respect_occurrence_and_current_visibility() {
    let Some((_container, database)) =
        common::postgres_database("saved-view workflow events").await
    else {
        return;
    };
    shared_saved_view_contract(database.repositories().await).await;
}

async fn occurrence(repos: &Repositories, view_id: Id, event_type: &str) -> OutboxEvent {
    let event = repos
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.subject().id == view_id && envelope.event_type() == event_type
            })
        })
        .unwrap();
    assert!(event.event.is_none());
    event
}

async fn shared_saved_view_contract(repos: Repositories) {
    let user = UserId::from("saved-view-owner");
    repos
        .auth
        .create_user(User {
            id: user.to_string(),
            username: user.to_string(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    let default_vault = repos.vaults.instance_default().await.unwrap();
    let vault = repos.vaults.create("Views", Some(&user)).await.unwrap().id;
    assert_ne!(vault, default_vault);
    let access = SavedViewAccess {
        user: user.clone(),
        role: VaultRole::Owner,
    };
    let lineage = MutationLineage {
        correlation_id: Uuid::new_v4(),
        causation_id: Some(Uuid::new_v4()),
    };

    with_actor(
        Some(user),
        with_lineage(Some(lineage), exercise(&repos, vault, &access, lineage)),
    )
    .await;
}

async fn exercise(
    repos: &Repositories,
    vault: VaultId,
    access: &SavedViewAccess,
    lineage: MutationLineage,
) {
    let user = &access.user;
    let store = Arc::new(ObjectStoreDocumentStore::in_memory());
    let engine = WorkflowEngine::new(
        repos.workflows.clone(),
        repos.documents.clone(),
        repos.correspondents.clone(),
        repos.document_types.clone(),
        Arc::new(StorageFileMetadata::new(store)),
        "http://folio.test",
    )
    .with_saved_views(repos.saved_views.clone());
    let runner = WorkflowRunner::new(
        repos.workflows.clone(),
        repos.workflow_documents.clone(),
        WebhookClient::new(),
    )
    .with_saved_views(repos.saved_views.clone());

    let server = TestServer::start().await;
    for (name, trigger) in [
        ("created", WorkflowTrigger::SavedViewCreated),
        ("updated", WorkflowTrigger::SavedViewUpdated),
        ("deleted", WorkflowTrigger::SavedViewDeleted),
    ] {
        repos
            .workflows
            .create(
                vault,
                WorkflowInput {
                    name: format!("View {name}"),
                    enabled: true,
                    trigger,
                    cadence: None,
                    conditions: WorkflowConditions {
                        event: Some(WorkflowEventConditions {
                            actor: Some(WorkflowActorCondition::User {
                                user_id: user.clone(),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    actions: vec![webhook(server.url(name), None)],
                },
            )
            .await
            .unwrap();
    }

    let mut input = SavedViewInput {
        name: "Initial private name".into(),
        query: SavedViewQuery::default(),
        visibility: ViewVisibility::Private,
        presentation: ViewPresentation::List,
    };
    input.query.filters.search_query = Some("never deliver this query".into());
    let field = repos
        .metadata_fields
        .create(
            vault,
            MetadataFieldInput {
                name: "Reference".into(),
                kind: MetadataKind::Text,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    input.query.version = 3;
    input.query.filters.metadata_exact = Some(MetadataExactFilter {
        field_id: field.id,
        value: MetadataValue::Text("never deliver this custom field value".into()),
    });
    let private = repos
        .saved_views
        .create(vault, access, input.clone())
        .await
        .unwrap();
    let private_created = occurrence(repos, private.id, "saved_view.created").await;

    input.name = "Promoted view".into();
    input.visibility = ViewVisibility::Shared;
    repos
        .saved_views
        .update(vault, access, private.id, input.clone())
        .await
        .unwrap();
    let promotion = occurrence(repos, private.id, "saved_view.updated").await;
    for event in [private_created, promotion] {
        assert_eq!(
            event.envelope.as_ref().unwrap().audience(),
            &EventAudience::User {
                user_id: user.clone()
            }
        );
        assert!(engine.handle(&event).await.unwrap().is_empty());
    }

    input.name = "Public view at creation".into();
    let shared = repos
        .saved_views
        .create(vault, access, input.clone())
        .await
        .unwrap();
    let created = occurrence(repos, shared.id, "saved_view.created").await;
    let envelope = created.envelope.as_ref().unwrap();
    let (first, replay) = tokio::join!(engine.handle(&created), engine.handle(&created));
    let mut runs = first.unwrap();
    runs.extend(replay.unwrap());
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.document_id, None);
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    {
        let received = server.state.received.lock().unwrap();
        let body = &received[0].body;
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(!body.contains("never deliver this query"));
        assert!(payload.get("document").is_none());
        assert_eq!(payload["occurred_at"], json!(envelope.occurred_at()));
        assert_eq!(
            payload["source_event"],
            json!({
                "event_id": envelope.event_id(),
                "mutation_id": envelope.mutation_id(),
                "correlation_id": lineage.correlation_id,
            })
        );
        assert_eq!(
            payload["saved_view"],
            json!({
                "vault_id": vault, "saved_view_id": shared.id, "name": "Public view at creation",
                "visibility": "shared", "presentation": "list",
                "actor_id": user, "causation_id": lineage.causation_id,
            })
        );
    }

    input.name = "Public view after rename".into();
    input.presentation = ViewPresentation::Grid;
    repos
        .saved_views
        .update(vault, access, shared.id, input.clone())
        .await
        .unwrap();
    let updated = occurrence(repos, shared.id, "saved_view.updated").await;
    let mut workflow_origin = updated.clone();
    workflow_origin.origin = EventOrigin::Workflow;
    assert!(engine.handle(&workflow_origin).await.unwrap().is_empty());
    let rename = engine.handle(&updated).await.unwrap().pop().unwrap();

    input.name = "Public view before restriction".into();
    repos
        .saved_views
        .update(vault, access, shared.id, input.clone())
        .await
        .unwrap();
    let latest = occurrence(repos, shared.id, "saved_view.updated").await;
    let queued = engine.handle(&latest).await.unwrap().pop().unwrap();
    assert_eq!(
        runner.execute(rename.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    {
        let received = server.state.received.lock().unwrap();
        assert_eq!(received[1].path, "updated");
        let payload: serde_json::Value = serde_json::from_str(&received[1].body).unwrap();
        assert_eq!(payload["saved_view"]["name"], "Public view after rename");
        assert_eq!(payload["saved_view"]["presentation"], "grid");
    }

    input.name = "Private again".into();
    input.visibility = ViewVisibility::Private;
    repos
        .saved_views
        .update(vault, access, shared.id, input.clone())
        .await
        .unwrap();
    let demotion = occurrence(repos, shared.id, "saved_view.updated").await;
    assert_eq!(
        demotion.envelope.as_ref().unwrap().audience(),
        &EventAudience::User {
            user_id: user.clone()
        }
    );
    assert!(engine.handle(&updated).await.unwrap().is_empty());
    assert!(engine.handle(&demotion).await.unwrap().is_empty());
    assert_eq!(runner.execute(queued.id).await.unwrap(), RunOutcome::Failed);
    assert_eq!(server.hits("updated"), 1);

    repos
        .saved_views
        .delete(vault, access, shared.id)
        .await
        .unwrap();
    let private_deleted = occurrence(repos, shared.id, "saved_view.deleted").await;
    assert!(engine.handle(&private_deleted).await.unwrap().is_empty());

    repos
        .saved_views
        .delete(vault, access, private.id)
        .await
        .unwrap();
    let shared_deleted = occurrence(repos, private.id, "saved_view.deleted").await;
    assert_eq!(
        shared_deleted.envelope.as_ref().unwrap().audience(),
        &EventAudience::Vault
    );
    let run = engine.handle(&shared_deleted).await.unwrap().pop().unwrap();
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    assert_eq!(server.hits("deleted"), 1);
    for request in server.state.received.lock().unwrap().iter() {
        assert!(!request
            .body
            .contains("never deliver this custom field value"));
        assert!(!request.body.contains("metadata_exact"));
    }

    let other = repos
        .vaults
        .create("Other vault", Some(user))
        .await
        .unwrap()
        .id;
    assert!(!repos
        .saved_views
        .is_shared_for_workflow(other, private.id, true)
        .await
        .unwrap());
    repos.vaults.delete(vault).await.unwrap();
    assert!(!repos
        .saved_views
        .is_shared_for_workflow(vault, private.id, true)
        .await
        .unwrap());
    assert!(engine.handle(&shared_deleted).await.unwrap().is_empty());
}

use folio_auth::User;
use folio_core::models::{
    SavedViewInput, SavedViewQuery, UserId, VaultRole, ViewPresentation, ViewVisibility,
};
use folio_core::ports::SavedViewAccess;
use folio_infra::{Database, Repositories};

async fn member(repos: &Repositories, name: &str, role: VaultRole) -> SavedViewAccess {
    repos
        .auth
        .create_user(User {
            id: name.into(),
            username: name.into(),
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
    SavedViewAccess {
        user: UserId::from(name),
        role,
    }
}

fn input(visibility: ViewVisibility) -> SavedViewInput {
    SavedViewInput {
        name: "Invoices".into(),
        visibility,
        query: SavedViewQuery::default(),
        presentation: ViewPresentation::List,
    }
}

async fn contract(repos: Repositories) {
    let vault = repos.vaults.instance_default().await.unwrap();
    let alice = member(&repos, "alice", VaultRole::Editor).await;
    let bob = member(&repos, "bob", VaultRole::Owner).await;
    let view = folio_core::mutation::with_actor(
        Some(alice.user.clone()),
        repos
            .saved_views
            .create(vault, &alice, input(ViewVisibility::Private)),
    )
    .await
    .unwrap();
    assert_eq!(view.owner_id, alice.user);
    assert_eq!(view.created_by.as_ref(), Some(&alice.user));
    assert_eq!(view.updated_by.as_ref(), Some(&alice.user));
    assert_eq!(view.created_at, view.updated_at);
    assert!(repos
        .saved_views
        .find(vault, &bob, view.id)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .saved_views
        .list(vault, &bob, None, 100)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .saved_views
        .delete(vault, &bob, view.id)
        .await
        .is_err());
    let other_vault = repos.vaults.create("Other", Some(&bob.user)).await.unwrap();
    assert!(repos
        .saved_views
        .find(other_vault.id, &alice, view.id)
        .await
        .unwrap()
        .is_none());
    let unchanged = repos
        .saved_views
        .update(vault, &alice, view.id, input(ViewVisibility::Private))
        .await
        .unwrap();
    assert_eq!(unchanged.updated_at, view.updated_at);
    let shared = repos
        .saved_views
        .update(vault, &alice, view.id, input(ViewVisibility::Shared))
        .await
        .unwrap();
    assert_eq!(shared.created_at, view.created_at);
    assert!(repos
        .saved_views
        .find(vault, &bob, view.id)
        .await
        .unwrap()
        .is_some());
    let deleted = repos.saved_views.delete(vault, &bob, view.id).await;
    assert!(deleted.is_ok());
    assert!(repos
        .saved_views
        .find(vault, &alice, view.id)
        .await
        .unwrap()
        .is_none());
    repos
        .saved_views
        .create(vault, &alice, input(ViewVisibility::Private))
        .await
        .unwrap();
    let viewer = member(&repos, "viewer", VaultRole::Viewer).await;
    assert!(repos
        .saved_views
        .create(vault, &viewer, input(ViewVisibility::Shared))
        .await
        .is_err());
    assert!(repos
        .saved_views
        .list(vault, &alice, None, 101)
        .await
        .is_err());
}

#[tokio::test]
async fn sqlite_enforces_saved_view_ownership_and_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let database = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        directory.path().join("views.db").display()
    ))
    .await
    .unwrap();
    database.migrate().await.unwrap();
    contract(database.repositories().await).await;
    event_contract(&database).await;
    rollback_contract(&database).await;
}

#[tokio::test]
async fn postgres_enforces_saved_view_ownership_and_lifecycle() {
    use testcontainers_modules::{
        postgres::Postgres,
        testcontainers::{runners::AsyncRunner, ImageExt},
    };
    let container = Postgres::default()
        .with_tag("17-alpine")
        .start()
        .await
        .expect("saved-view PostgreSQL regression requires Docker");
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let database = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    database.migrate().await.unwrap();
    contract(database.repositories().await).await;
    event_contract(&database).await;
    rollback_contract(&database).await;
}

async fn view_events(
    repos: &Repositories,
    id: uuid::Uuid,
) -> Vec<folio_core::domain_event::DomainEventEnvelope> {
    repos
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|event| {
            let envelope = event.envelope?;
            if envelope.subject().subject_type != "saved_view" || envelope.subject().id != id {
                return None;
            }
            assert!(event.event.is_none());
            Some(envelope)
        })
        .collect()
}

async fn event_contract(database: &Database) {
    use folio_core::domain_event::EventAudience;
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let owner = member(&repos, "event-owner", VaultRole::Editor).await;
    folio_core::mutation::with_actor(Some(owner.user.clone()), async {
        let mut draft = input(ViewVisibility::Private);
        draft.query.filters.search_query = Some("confidential query".into());
        let lineage = folio_core::mutation::MutationLineage {
            correlation_id: uuid::Uuid::new_v4(),
            causation_id: Some(uuid::Uuid::new_v4()),
        };
        let view = folio_core::mutation::with_lineage(
            Some(lineage),
            repos.saved_views.create(vault, &owner, draft.clone()),
        )
        .await
        .unwrap();
        assert_eq!(view.created_by.as_ref(), Some(&owner.user));
        let private_audience = EventAudience::User {
            user_id: owner.user.clone(),
        };
        let created = view_events(&repos, view.id).await;
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].event_type(), "saved_view.created");
        assert_eq!(created[0].vault_id(), vault);
        assert_eq!(created[0].actor_id(), Some(&owner.user));
        assert_eq!(created[0].audience(), &private_audience);
        assert_eq!(created[0].document_id(), None);
        assert_eq!(created[0].correlation_id(), lineage.correlation_id);
        assert_eq!(created[0].causation_id(), lineage.causation_id);
        assert_eq!(
            created[0].data(),
            &serde_json::json!({"name":"Invoices","visibility":"private","presentation":"list"})
        );
        assert!(repos
            .saved_views
            .create(vault, &owner, draft.clone())
            .await
            .is_err());
        let unchanged = repos
            .saved_views
            .update(vault, &owner, view.id, draft.clone())
            .await
            .unwrap();
        assert_eq!(unchanged.updated_at, view.updated_at);
        assert_eq!(view_events(&repos, view.id).await.len(), 1);

        draft.visibility = ViewVisibility::Shared;
        repos
            .saved_views
            .update(vault, &owner, view.id, draft.clone())
            .await
            .unwrap();
        assert_eq!(
            view_events(&repos, view.id).await[1].audience(),
            &private_audience
        );
        draft.name = "Public invoices".into();
        repos
            .saved_views
            .update(vault, &owner, view.id, draft.clone())
            .await
            .unwrap();
        assert_eq!(
            view_events(&repos, view.id).await[2].audience(),
            &EventAudience::Vault
        );
        repos
            .saved_views
            .update(vault, &owner, view.id, draft.clone())
            .await
            .unwrap();
        assert_eq!(view_events(&repos, view.id).await.len(), 3);
        draft.visibility = ViewVisibility::Private;
        repos
            .saved_views
            .update(vault, &owner, view.id, draft)
            .await
            .unwrap();
        assert_eq!(
            view_events(&repos, view.id).await[3].audience(),
            &private_audience
        );
        repos
            .saved_views
            .delete(vault, &owner, view.id)
            .await
            .unwrap();
        assert!(repos
            .saved_views
            .delete(vault, &owner, view.id)
            .await
            .is_err());
        let events = view_events(&repos, view.id).await;
        assert_eq!(events.len(), 5);
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_type())
                .collect::<Vec<_>>(),
            vec![
                "saved_view.created",
                "saved_view.updated",
                "saved_view.updated",
                "saved_view.updated",
                "saved_view.deleted"
            ]
        );
        assert!(!serde_json::to_string(&events)
            .unwrap()
            .contains("confidential query"));
        assert_eq!(events[4].event_type(), "saved_view.deleted");
        assert_eq!(events[4].audience(), &private_audience);
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            5
        );
    })
    .await;
    let mut concurrent_input = input(ViewVisibility::Private);
    concurrent_input.name = "Concurrent view".into();
    let concurrent = repos
        .saved_views
        .create(vault, &owner, concurrent_input.clone())
        .await
        .unwrap();
    concurrent_input.visibility = ViewVisibility::Shared;
    let (first, repeated) = tokio::join!(
        repos
            .saved_views
            .update(vault, &owner, concurrent.id, concurrent_input.clone()),
        repos
            .saved_views
            .update(vault, &owner, concurrent.id, concurrent_input),
    );
    assert_eq!(first.unwrap().updated_at, repeated.unwrap().updated_at);
    assert_eq!(view_events(&repos, concurrent.id).await.len(), 2);
    repos
        .saved_views
        .delete(vault, &owner, concurrent.id)
        .await
        .unwrap();
    let mut system_input = input(ViewVisibility::Private);
    system_input.name = "System-created view".into();
    let system_view = repos
        .saved_views
        .create(vault, &owner, system_input)
        .await
        .unwrap();
    assert_eq!(system_view.owner_id, owner.user);
    assert_eq!(system_view.created_by, None);
    assert_eq!(system_view.updated_by, None);
    assert_eq!(
        view_events(&repos, system_view.id).await[0].actor_id(),
        None
    );
}

async fn fail_outbox(database: &Database, enabled: bool) {
    match database {
        Database::Sqlite(pool) => {
            let statement = if enabled {
                "CREATE TRIGGER reject_saved_view_event BEFORE INSERT ON outbox_events WHEN NEW.type LIKE 'saved_view.%' BEGIN SELECT RAISE(ABORT, 'fixture outbox failure'); END"
            } else {
                "DROP TRIGGER reject_saved_view_event"
            };
            sqlx::raw_sql(statement).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            let statement = if enabled {
                "CREATE FUNCTION reject_saved_view_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.type LIKE 'saved_view.%' THEN RAISE EXCEPTION 'fixture outbox failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_saved_view_event BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION reject_saved_view_event()"
            } else {
                "DROP TRIGGER reject_saved_view_event ON outbox_events; DROP FUNCTION reject_saved_view_event()"
            };
            sqlx::raw_sql(statement).execute(pool).await.unwrap();
        }
    }
}

async fn rollback_contract(database: &Database) {
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let owner = member(&repos, "rollback-owner", VaultRole::Editor).await;
    let view = repos
        .saved_views
        .create(vault, &owner, input(ViewVisibility::Private))
        .await
        .unwrap();
    let original_events = view_events(&repos, view.id).await;
    fail_outbox(database, true).await;
    let mut draft = input(ViewVisibility::Private);
    draft.name = "Rolled back".into();
    assert!(repos
        .saved_views
        .create(vault, &owner, draft.clone())
        .await
        .is_err());
    assert_eq!(
        repos
            .saved_views
            .list(vault, &owner, None, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(repos
        .saved_views
        .update(vault, &owner, view.id, draft)
        .await
        .is_err());
    assert!(repos
        .saved_views
        .delete(vault, &owner, view.id)
        .await
        .is_err());
    let retained = repos
        .saved_views
        .find(vault, &owner, view.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.name, view.name);
    assert_eq!(retained.updated_at, view.updated_at);
    assert_eq!(view_events(&repos, view.id).await, original_events);
    fail_outbox(database, false).await;
    repos
        .saved_views
        .delete(vault, &owner, view.id)
        .await
        .unwrap();
    assert_eq!(
        view_events(&repos, view.id).await.len(),
        original_events.len() + 1
    );
}

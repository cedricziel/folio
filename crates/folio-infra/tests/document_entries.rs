mod common;

use chrono::{Duration, Utc};
use folio_core::models::{CreateDocument, EntryBody, EntryKind, EntryPageQuery, Id, UserId};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::ports::EntryCreation;
use folio_infra::{Database, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

fn body(text: &str) -> EntryBody {
    EntryBody::parse(text.into()).unwrap()
}

async fn contract(r: &Repositories, kind: EntryKind, other_kind: EntryKind, prefix: &str) {
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Notes fixture".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = Id::new_v4();
    let author = UserId::from("entry-author");
    let entry = with_actor(Some(author.clone()), async {
        match r
            .entries
            .create(vault, document.id, kind, body("Private text"), key)
            .await
            .unwrap()
        {
            EntryCreation::Created(entry) => entry,
            outcome => panic!("expected creation, got {outcome:?}"),
        }
    })
    .await;
    assert_eq!(entry.created_by, Some(author.clone()));
    assert_eq!(entry.updated_by, entry.created_by);
    assert_eq!(entry.created_at, entry.updated_at);
    let replay = with_actor(
        Some(author.clone()),
        r.entries
            .create(vault, document.id, kind, body("Private text"), key),
    )
    .await
    .unwrap();
    assert!(matches!(replay, EntryCreation::Replayed(replayed) if replayed.id == entry.id));
    assert!(with_actor(
        Some(author.clone()),
        r.entries
            .create(vault, document.id, kind, body("Different"), key)
    )
    .await
    .is_err());
    assert!(with_actor(
        Some(UserId::from("other")),
        r.entries
            .update(vault, document.id, entry.id, body("Unauthorized"))
    )
    .await
    .is_err());
    with_actor(
        Some(author.clone()),
        r.entries
            .update(vault, document.id, entry.id, body("Private text")),
    )
    .await
    .unwrap();
    let changed = with_actor(
        Some(author.clone()),
        r.entries
            .update(vault, document.id, entry.id, body("Revised")),
    )
    .await
    .unwrap();
    assert_eq!(changed.body, "Revised");
    assert_eq!(changed.created_at, entry.created_at);
    assert_eq!(changed.created_by, entry.created_by);
    let page = r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind,
                limit: 1,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(page.next_cursor.is_none());
    assert!(r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind: other_kind,
                limit: 1,
                cursor: None
            }
        )
        .await
        .unwrap()
        .entries
        .is_empty());
    with_actor(
        Some(author.clone()),
        r.entries.delete(vault, document.id, entry.id, false),
    )
    .await
    .unwrap();
    with_actor(
        Some(author.clone()),
        r.entries.delete(vault, document.id, entry.id, false),
    )
    .await
    .unwrap();
    assert!(r
        .entries
        .find(vault, document.id, entry.id)
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        with_actor(
            Some(author),
            r.entries
                .create(vault, document.id, kind, body("Private text"), key)
        )
        .await
        .unwrap(),
        EntryCreation::Deleted
    ));
    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::hours(1), 100)
        .await
        .unwrap();
    let transitions: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| event.event_type().starts_with(&format!("{prefix}.")))
        .collect();
    assert_eq!(transitions.len(), 3, "create, changed update, delete only");
    assert_eq!(transitions[0].event_type(), format!("{prefix}.created"));
    assert_eq!(transitions[1].event_type(), format!("{prefix}.updated"));
    assert_eq!(transitions[2].event_type(), format!("{prefix}.deleted"));
    assert!(!serde_json::to_string(&transitions)
        .unwrap()
        .contains("Private text"));
    assert!(!serde_json::to_string(&transitions)
        .unwrap()
        .contains("Revised"));
}

async fn rollback_and_concurrency_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Atomic entry".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = Id::new_v4();
    let failed = with_lineage(
        Some(MutationLineage {
            correlation_id: Id::nil(),
            causation_id: None,
        }),
        r.entries.create(
            vault,
            document.id,
            EntryKind::Comment,
            body("Retry me"),
            key,
        ),
    )
    .await;
    assert!(failed.is_err());
    assert!(r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind: EntryKind::Comment,
                limit: 10,
                cursor: None
            }
        )
        .await
        .unwrap()
        .entries
        .is_empty());
    let correlation = Id::new_v4();
    let cause = Id::new_v4();
    let lineage = Some(MutationLineage {
        correlation_id: correlation,
        causation_id: Some(cause),
    });
    let (first, second) = tokio::join!(
        with_lineage(
            lineage,
            r.entries.create(
                vault,
                document.id,
                EntryKind::Comment,
                body("Retry me"),
                key
            )
        ),
        with_lineage(
            lineage,
            r.entries.create(
                vault,
                document.id,
                EntryKind::Comment,
                body("Retry me"),
                key
            )
        )
    );
    let pair = (first.unwrap(), second.unwrap());
    let (created, replayed) = match pair {
        (EntryCreation::Created(created), EntryCreation::Replayed(replayed))
        | (EntryCreation::Replayed(replayed), EntryCreation::Created(created)) => {
            (created, replayed)
        }
        other => panic!("expected one creation and one replay, got {other:?}"),
    };
    assert_eq!(created.id, replayed.id);
    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::hours(1), 100)
        .await
        .unwrap();
    let events: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| event.subject().id == created.id)
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].correlation_id(), correlation);
    assert_eq!(events[0].causation_id(), Some(cause));
}

async fn document_move_contract(r: &Repositories) {
    let from = r.vaults.instance_default().await.unwrap();
    let to = r.vaults.create("Move target", None).await.unwrap().id;
    let document = r
        .documents
        .create(
            from,
            CreateDocument {
                title: "Moving discussion".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let author = UserId::from("move-author");
    let key = Id::new_v4();
    let entry = match with_actor(
        Some(author.clone()),
        r.entries.create(
            from,
            document.id,
            EntryKind::Note,
            body("Stays attached"),
            key,
        ),
    )
    .await
    .unwrap()
    {
        EntryCreation::Created(entry) => entry,
        other => panic!("expected new entry, got {other:?}"),
    };
    r.documents
        .move_to(document.id, from, to, None)
        .await
        .unwrap();
    assert!(r.entries.find(from, document.id, entry.id).await.is_err());
    let moved = r
        .entries
        .find(to, document.id, entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.vault_id, to);
    assert_eq!(moved.created_by, Some(author.clone()));
    assert_eq!(
        moved.updated_by, None,
        "system move must update attribution"
    );
    assert_eq!(moved.created_at, entry.created_at);
    let replay = with_actor(
        Some(author),
        r.entries.create(
            to,
            document.id,
            EntryKind::Note,
            body("Stays attached"),
            key,
        ),
    )
    .await
    .unwrap();
    assert!(
        matches!(replay, EntryCreation::Replayed(original) if original.id == entry.id && original.vault_id == to)
    );
    r.documents.delete(to, document.id).await.unwrap();
    assert!(r.entries.find(to, document.id, entry.id).await.is_err());
    r.documents.restore(to, document.id).await.unwrap();
    assert_eq!(
        r.entries
            .find(to, document.id, entry.id)
            .await
            .unwrap()
            .unwrap()
            .body,
        "Stays attached"
    );
}

async fn paging_and_moderation_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Paged entries".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let author = Some(UserId::from("paging-author"));
    with_actor(
        author.clone(),
        r.entries.create(
            vault,
            document.id,
            EntryKind::Note,
            body("First"),
            Id::new_v4(),
        ),
    )
    .await
    .unwrap();
    with_actor(
        author.clone(),
        r.entries.create(
            vault,
            document.id,
            EntryKind::Note,
            body("Second"),
            Id::new_v4(),
        ),
    )
    .await
    .unwrap();
    let first = r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind: EntryKind::Note,
                limit: 1,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(first.entries.len(), 1);
    assert!(first.next_cursor.is_some());
    let first_id = first.entries[0].id;
    with_actor(
        author.clone(),
        r.entries.update(
            vault,
            document.id,
            first_id,
            body("Edited without reordering"),
        ),
    )
    .await
    .unwrap();
    let second = r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind: EntryKind::Note,
                limit: 1,
                cursor: first.next_cursor.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_ne!(second.entries[0].id, first_id);
    assert!(second.next_cursor.is_none());
    assert!(r
        .entries
        .page(
            vault,
            document.id,
            EntryPageQuery {
                kind: EntryKind::Comment,
                limit: 1,
                cursor: first.next_cursor
            }
        )
        .await
        .is_err());
    let other = Some(UserId::from("moderator"));
    assert!(with_actor(
        other.clone(),
        r.entries.delete(vault, document.id, first_id, false)
    )
    .await
    .is_err());
    with_actor(other, r.entries.delete(vault, document.id, first_id, true))
        .await
        .unwrap();
    assert!(r
        .entries
        .find(vault, document.id, first_id)
        .await
        .unwrap()
        .is_none());
    let unrelated = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Unrelated".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(r
        .entries
        .find(vault, unrelated.id, second.entries[0].id)
        .await
        .unwrap()
        .is_none());
    assert!(with_actor(
        author,
        r.entries.update(
            vault,
            unrelated.id,
            second.entries[0].id,
            body("Wrong parent")
        )
    )
    .await
    .is_err());
}

#[tokio::test]
async fn sqlite_entries_commit_attribution_receipts_and_events_together() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}",
        directory.path().join("entries.db").display()
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    contract(
        &repositories,
        EntryKind::Note,
        EntryKind::Comment,
        "document_note",
    )
    .await;
    contract(
        &repositories,
        EntryKind::Comment,
        EntryKind::Note,
        "document_comment",
    )
    .await;
    rollback_and_concurrency_contract(&repositories).await;
    document_move_contract(&repositories).await;
    paging_and_moderation_contract(&repositories).await;
}

#[tokio::test]
async fn postgres_entries_commit_attribution_receipts_and_events_together() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    let repositories = db.repositories().await;
    contract(
        &repositories,
        EntryKind::Note,
        EntryKind::Comment,
        "document_note",
    )
    .await;
    contract(
        &repositories,
        EntryKind::Comment,
        EntryKind::Note,
        "document_comment",
    )
    .await;
    rollback_and_concurrency_contract(&repositories).await;
    document_move_contract(&repositories).await;
    paging_and_moderation_contract(&repositories).await;
}

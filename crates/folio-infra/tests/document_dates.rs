mod common;

use chrono::{Duration, Utc};
use folio_core::models::{CreateDocument, UpdateDocument, UserId};
use folio_core::mutation::with_actor;
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn reject_generic_updates(db: &Database, reject: bool) {
    match db {
        Database::Sqlite(pool) => {
            let sql = if reject {
                "CREATE TRIGGER reject_date_update BEFORE INSERT ON outbox_events WHEN NEW.type = 'updated' BEGIN SELECT RAISE(ABORT, 'injected outbox failure'); END"
            } else {
                "DROP TRIGGER reject_date_update"
            };
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            let sql = if reject {
                "CREATE FUNCTION reject_date_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.type = 'updated' THEN RAISE EXCEPTION 'injected outbox failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_date_update BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION reject_date_update()"
            } else {
                "DROP TRIGGER reject_date_update ON outbox_events; DROP FUNCTION reject_date_update()"
            };
            sqlx::raw_sql(sql).execute(pool).await.unwrap();
        }
    }
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let doc = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Dated letter".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let doc = r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(doc.document_date, None);
    let date = "1582-10-10".parse().unwrap();
    let update = || UpdateDocument {
        document_date: Some(Some(date)),
        ..Default::default()
    };
    reject_generic_updates(&db, true).await;
    assert!(r.documents.update(vault, doc.id, update()).await.is_err());
    let rolled_back = r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rolled_back.document_date, None);
    assert_eq!(rolled_back.updated_at, doc.updated_at);
    reject_generic_updates(&db, false).await;
    let changed = with_actor(
        Some(UserId("editor".into())),
        r.documents.update(vault, doc.id, update()),
    )
    .await
    .unwrap();
    assert_eq!(changed.document_date, Some(date));
    assert_eq!(changed.created_at, doc.created_at);
    let saved = r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    let repeated = r.documents.update(vault, doc.id, update()).await.unwrap();
    assert_eq!(repeated.updated_at, saved.updated_at);
    let omitted = r
        .documents
        .update(vault, doc.id, UpdateDocument::default())
        .await
        .unwrap();
    assert_eq!(omitted.document_date, Some(date));
    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::minutes(5), 100)
        .await
        .unwrap();
    let dates: Vec<_> = events
        .iter()
        .filter_map(|e| e.envelope.as_ref())
        .filter(|e| e.event_type() == "document.date_changed")
        .collect();
    assert_eq!(dates.len(), 1);
    assert_eq!(dates[0].actor_id(), Some(&UserId("editor".into())));
    assert_eq!(dates[0].data()["document_date"], "1582-10-10");
    assert!(dates[0].data()["previous_document_date"].is_null());
    let updated: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| event.event_type() == "document.updated")
        .collect();
    assert_eq!(updated.len(), 1);
    assert_eq!(dates[0].mutation_id(), updated[0].mutation_id());
    assert_eq!(dates[0].correlation_id(), updated[0].correlation_id());
    let cleared = r
        .documents
        .update(
            vault,
            doc.id,
            UpdateDocument {
                document_date: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.document_date, None);
    assert_eq!(
        r.documents
            .find_by_id(vault, doc.id)
            .await
            .unwrap()
            .unwrap()
            .document_date,
        None
    );
    r.documents.delete(vault, doc.id).await.unwrap();
    assert!(r.documents.update(vault, doc.id, update()).await.is_err());
    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::minutes(5), 100)
        .await
        .unwrap();
    let dates: Vec<_> = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .filter(|event| event.event_type() == "document.date_changed")
        .collect();
    assert_eq!(dates.len(), 2);
    assert_eq!(dates[1].data()["previous_document_date"], "1582-10-10");
    assert!(dates[1].data()["document_date"].is_null());
}

#[tokio::test]
async fn sqlite_dates_preserve_import_time_and_emit_only_real_changes() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("dates.db").display()
    ))
    .await
    .unwrap();
    contract(db).await;
}

#[tokio::test]
async fn postgres_dates_preserve_import_time_and_emit_only_real_changes() {
    let Some(container) = common::container_or_skip(
        "document dates",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    contract(db).await;
}

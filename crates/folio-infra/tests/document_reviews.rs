mod common;

use chrono::{Duration, Utc};
use folio_core::models::{CreateDocument, ReviewOrigin, UpdateDocument, UserId};
use folio_core::mutation::with_actor;
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

async fn reject_generic_updates(db: &Database, reject: bool) {
    match db {
        Database::Sqlite(pool) => {
            let sql = if reject {
                "CREATE TRIGGER reject_review_update BEFORE INSERT ON outbox_events WHEN NEW.type = 'updated' BEGIN SELECT RAISE(ABORT, 'injected outbox failure'); END"
            } else {
                "DROP TRIGGER reject_review_update"
            };
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            let sql = if reject {
                "CREATE FUNCTION reject_review_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.type = 'updated' THEN RAISE EXCEPTION 'injected outbox failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_review_update BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION reject_review_update()"
            } else {
                "DROP TRIGGER reject_review_update ON outbox_events; DROP FUNCTION reject_review_update()"
            };
            sqlx::raw_sql(sql).execute(pool).await.unwrap();
        }
    }
}

async fn review_contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let imported = r
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Review decision".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let imported = r
        .documents
        .find_by_id(vault, imported.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(imported.reviewed_at, None);
    reject_generic_updates(&db, true).await;
    assert!(r
        .documents
        .set_reviewed(vault, imported.id, true)
        .await
        .is_err());
    let rolled_back = r
        .documents
        .find_by_id(vault, imported.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rolled_back.reviewed_at, None);
    assert_eq!(rolled_back.updated_at, imported.updated_at);
    reject_generic_updates(&db, false).await;
    assert!(r
        .documents
        .set_reviewed(folio_core::models::VaultId::new(), imported.id, true)
        .await
        .is_err());
    let reviewer = UserId("reviewer".into());
    let reviewed = with_actor(Some(reviewer.clone()), async {
        let (first, second) = tokio::join!(
            r.documents.set_reviewed(vault, imported.id, true),
            r.documents.set_reviewed(vault, imported.id, true)
        );
        let first = first?;
        assert_eq!(first.reviewed_at, second?.reviewed_at);
        Ok::<_, folio_core::Error>(first)
    })
    .await
    .unwrap();
    assert!(reviewed.reviewed_at.is_some());
    assert_eq!(reviewed.reviewed_by, Some(reviewer.clone()));
    assert_eq!(reviewed.review_origin, Some(ReviewOrigin::Human));
    assert_eq!(reviewed.ocr_status, imported.ocr_status);
    assert_eq!(reviewed.embed_status, imported.embed_status);
    assert_eq!(reviewed.created_at, imported.created_at);

    let repeated = r
        .documents
        .set_reviewed(vault, imported.id, true)
        .await
        .unwrap();
    assert_eq!(repeated.reviewed_at, reviewed.reviewed_at);
    assert_eq!(repeated.reviewed_by, reviewed.reviewed_by);
    assert_eq!(repeated.updated_at, reviewed.updated_at);

    r.documents
        .complete_ocr(vault, imported.id, "Extracted later")
        .await
        .unwrap();
    let edited = r
        .documents
        .update(
            vault,
            imported.id,
            UpdateDocument {
                title: Some("Renamed later".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.reviewed_at, reviewed.reviewed_at);
    assert_eq!(edited.reviewed_by, reviewed.reviewed_by);
    assert_eq!(edited.review_origin, reviewed.review_origin);

    r.documents.delete(vault, imported.id).await.unwrap();
    assert!(r
        .documents
        .set_reviewed(vault, imported.id, false)
        .await
        .is_err());
    let restored = r.documents.restore(vault, imported.id).await.unwrap();
    assert_eq!(restored.reviewed_at, reviewed.reviewed_at);
    let unreviewed = r
        .documents
        .set_reviewed(vault, imported.id, false)
        .await
        .unwrap();
    assert_eq!(unreviewed.reviewed_at, None);
    assert_eq!(unreviewed.reviewed_by, None);
    assert_eq!(unreviewed.review_origin, None);
    let repeated = r
        .documents
        .set_reviewed(vault, imported.id, false)
        .await
        .unwrap();
    assert_eq!(repeated.updated_at, unreviewed.updated_at);

    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::minutes(5), 100)
        .await
        .unwrap();
    let reviews: Vec<_> = events
        .iter()
        .filter_map(|row| row.envelope.as_ref())
        .filter(|event| {
            matches!(
                event.event_type(),
                "document.reviewed" | "document.unreviewed"
            )
        })
        .collect();
    assert_eq!(reviews.len(), 2);
    assert_eq!(reviews[0].event_type(), "document.reviewed");
    assert_eq!(reviews[0].actor_id(), Some(&reviewer));
    assert_eq!(reviews[0].data()["review_origin"], "human");
    assert_eq!(reviews[1].event_type(), "document.unreviewed");
    assert_eq!(reviews[1].data()["previous_reviewed_by"], "reviewer");
    assert_eq!(reviews[1].data()["reason"], "user");
    for review in reviews {
        let generic: Vec<_> = events
            .iter()
            .filter_map(|row| row.envelope.as_ref())
            .filter(|event| {
                event.event_type() == "document.updated"
                    && event.mutation_id() == review.mutation_id()
            })
            .collect();
        assert_eq!(generic.len(), 1);
        assert_eq!(generic[0].correlation_id(), review.correlation_id());
    }
    let system_review = r
        .documents
        .set_reviewed(vault, imported.id, true)
        .await
        .unwrap();
    assert_eq!(system_review.reviewed_by, None);
    assert_eq!(system_review.review_origin, Some(ReviewOrigin::System));
    let retained = with_actor(
        Some(reviewer),
        r.documents.set_reviewed(vault, imported.id, true),
    )
    .await
    .unwrap();
    assert_eq!(retained.review_origin, Some(ReviewOrigin::System));
    assert_eq!(retained.reviewed_at, system_review.reviewed_at);

    r.documents
        .set_reviewed(vault, imported.id, false)
        .await
        .unwrap();
    let events = r
        .outbox
        .replay(0, Utc::now() - Duration::minutes(5), 100)
        .await
        .unwrap();
    let system_unreview = events
        .iter()
        .filter_map(|row| row.envelope.as_ref())
        .filter(|event| event.event_type() == "document.unreviewed")
        .next_back()
        .unwrap();
    assert_eq!(system_unreview.actor_id(), None);
    assert_eq!(system_unreview.data()["reason"], "system");
}

#[tokio::test]
async fn sqlite_review_is_an_idempotent_decision_independent_of_processing() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("reviews.db").display()
    ))
    .await
    .unwrap();
    review_contract(db).await;
}

#[tokio::test]
async fn postgres_review_is_an_idempotent_decision_independent_of_processing() {
    let Some(container) = common::container_or_skip(
        "document review",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        container.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    review_contract(db).await;
}

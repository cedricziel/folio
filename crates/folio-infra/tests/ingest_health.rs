use folio_core::models::{SourceKind, SourceOrigin, SourceStatus};
use folio_core::ports::{IngestOutcomeRepository, OutboxStore};
use folio_infra::database::{
    postgres::PostgresIngestOutcomeRepository, sqlite::SqliteIngestOutcomeRepository,
};
use folio_infra::{Database, PostgresOutboxStore, SqliteOutboxStore};

async fn health_contract(database: Database) {
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let (writer, outbox): (Box<dyn IngestOutcomeRepository>, Box<dyn OutboxStore>) = match &database
    {
        Database::Sqlite(pool) => (
            Box::new(SqliteIngestOutcomeRepository::new(pool.clone())),
            Box::new(SqliteOutboxStore::new(pool.clone())),
        ),
        Database::Postgres(pool) => (
            Box::new(PostgresIngestOutcomeRepository::new(pool.clone())),
            Box::new(PostgresOutboxStore::new(pool.clone())),
        ),
    };
    let deleted_source = uuid::Uuid::new_v4();
    let failure = SourceStatus {
        last_error: Some("private failure".into()),
        ..Default::default()
    };
    assert!(writer
        .record_source_status(
            vault,
            deleted_source,
            SourceOrigin::Managed,
            SourceKind::Folder,
            &failure
        )
        .await
        .is_err());
    assert!(repos
        .sources
        .status(deleted_source)
        .await
        .unwrap()
        .is_none());
    for kind in [SourceKind::Folder, SourceKind::Email] {
        let source = uuid::Uuid::new_v4();
        let healthy = SourceStatus {
            last_run_at: Some(chrono::Utc::now()),
            last_failed: 1,
            ..Default::default()
        };
        if kind == SourceKind::Folder {
            writer
                .record_source_status(vault, source, SourceOrigin::Config, kind, &healthy)
                .await
                .unwrap();
        }
        assert!(outbox
            .claim(10, chrono::Duration::seconds(30), 10)
            .await
            .unwrap()
            .is_empty());
        match &database {
            Database::Sqlite(pool) => {
                sqlx::query("CREATE TRIGGER reject_health_event BEFORE INSERT ON outbox_events WHEN NEW.type LIKE 'source.%' BEGIN SELECT RAISE(ABORT, 'injected health failure'); END").execute(pool).await.unwrap();
            }
            Database::Postgres(pool) => {
                sqlx::query("ALTER TABLE outbox_events ADD CONSTRAINT reject_health_event CHECK (type NOT LIKE 'source.%') NOT VALID").execute(pool).await.unwrap();
            }
        }
        let failed = SourceStatus {
            last_error: Some("private failure".into()),
            ..healthy.clone()
        };
        assert!(writer
            .record_source_status(vault, source, SourceOrigin::Config, kind, &failed)
            .await
            .is_err());
        assert!(repos
            .sources
            .status(source)
            .await
            .unwrap()
            .is_none_or(|status| status.last_error.is_none()));
        match &database {
            Database::Sqlite(pool) => {
                sqlx::query("DROP TRIGGER reject_health_event")
                    .execute(pool)
                    .await
                    .unwrap();
            }
            Database::Postgres(pool) => {
                sqlx::query("ALTER TABLE outbox_events DROP CONSTRAINT reject_health_event")
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }
        let (first, repeated) = tokio::join!(
            writer.record_source_status(vault, source, SourceOrigin::Config, kind, &failed),
            writer.record_source_status(vault, source, SourceOrigin::Config, kind, &failed),
        );
        first.unwrap();
        repeated.unwrap();
        for error in ["password=secret", "private/mailbox", "password=secret"] {
            let failed = SourceStatus {
                last_error: Some(error.into()),
                ..healthy.clone()
            };
            writer
                .record_source_status(vault, source, SourceOrigin::Config, kind, &failed)
                .await
                .unwrap();
        }
        let events = outbox
            .claim(10, chrono::Duration::seconds(30), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        let event = events[0].envelope.as_ref().unwrap();
        assert_eq!(event.event_type(), "source.failed");
        assert_eq!(event.subject().id, source);
        assert_eq!(event.vault_id(), vault);
        assert_eq!(
            event.audience(),
            &folio_core::domain_event::EventAudience::Owners
        );
        assert_eq!(event.document_id(), None);
        assert_eq!(event.correlation_id(), source);
        let failed_mutation = event.mutation_id();
        assert!(!event.data().to_string().contains("secret"));
        assert!(!event.data().to_string().contains("mailbox"));
        outbox.mark_dispatched(events[0].id).await.unwrap();
        writer
            .record_source_status(vault, source, SourceOrigin::Config, kind, &healthy)
            .await
            .unwrap();
        writer
            .record_source_status(vault, source, SourceOrigin::Config, kind, &healthy)
            .await
            .unwrap();
        let events = outbox
            .claim(10, chrono::Duration::seconds(30), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].envelope.as_ref().unwrap().event_type(),
            "source.recovered"
        );
        assert_ne!(
            events[0].envelope.as_ref().unwrap().mutation_id(),
            failed_mutation
        );
        assert_eq!(
            repos
                .sources
                .status(source)
                .await
                .unwrap()
                .unwrap()
                .last_error,
            None
        );
        outbox.mark_dispatched(events[0].id).await.unwrap();
    }
}

#[tokio::test]
async fn sqlite_source_failure_recovery_emit_only_on_transition() {
    health_contract(Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_source_failure_recovery_emit_only_on_transition() {
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let container = match started {
        Ok(container) => container,
        Err(error) => {
            assert!(
                !std::env::var("FOLIO_REQUIRE_DOCKER").is_ok_and(|v| !v.is_empty() && v != "0"),
                "PostgreSQL required: {error}"
            );
            eprintln!("SKIPPED: PostgreSQL unavailable: {error}");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    health_contract(
        Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}

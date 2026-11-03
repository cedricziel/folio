use folio_infra::Database;
use sqlx::SqlitePool;

async fn database() -> SqlitePool {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    database.migrate().await.unwrap();
    let Database::Sqlite(pool) = database else {
        unreachable!()
    };
    sqlx::query("INSERT INTO vaults (id, name, created_at, updated_at) VALUES ('vault-a', 'A', '2026-11-02T00:00:00Z', '2026-11-02T00:00:00Z'), ('vault-b', 'B', '2026-11-02T00:00:00Z', '2026-11-02T00:00:00Z')")
        .execute(&pool).await.unwrap();
    pool
}

async fn item(pool: &SqlitePool, id: &str, vault: &str, source: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO ingest_items (id, vault_id, source_id, source_kind, source_managed, locator_key, attempt_id, started_at, lease_expires_at, decision_fingerprint, state) VALUES (?, ?, ?, 'folder', 0, ?, ?, '2026-11-02T00:00:00Z', '2026-11-02T00:10:00Z', ?, 'processing')")
        .bind(id).bind(vault).bind(source).bind("a".repeat(64)).bind(format!("attempt-{id}")).bind("b".repeat(64))
        .execute(pool).await?;
    Ok(())
}

async fn outcome(
    pool: &SqlitePool,
    attempt: &str,
    event: &str,
    vault: &str,
    result: &str,
    document: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO ingest_outcomes (vault_id, source_id, item_id, attempt_id, event_id, occurred_at, outcome, document_id) VALUES (?, 'source-a', 'item-a', ?, ?, '2026-11-02T00:00:00Z', ?, ?)")
        .bind(vault).bind(attempt).bind(event).bind(result).bind(document).execute(pool).await?;
    Ok(())
}

#[tokio::test]
async fn observation_identity_is_unique_within_source_and_vault() {
    let pool = database().await;
    item(&pool, "item-a", "vault-a", "source-a").await.unwrap();
    assert!(item(&pool, "item-b", "vault-a", "source-a").await.is_err());
    item(&pool, "item-c", "vault-a", "source-b").await.unwrap();
    item(&pool, "item-d", "vault-b", "source-a").await.unwrap();
}

#[tokio::test]
async fn terminal_outcomes_cannot_duplicate_attempt_or_event_identity() {
    let pool = database().await;
    item(&pool, "item-a", "vault-a", "source-a").await.unwrap();
    outcome(&pool, "attempt-a", "event-a", "vault-a", "failed", None)
        .await
        .unwrap();
    assert!(outcome(
        &pool,
        "attempt-a",
        "event-b",
        "vault-a",
        "imported",
        Some("document-a")
    )
    .await
    .is_err());
    assert!(
        outcome(&pool, "attempt-b", "event-a", "vault-a", "failed", None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn non_document_outcome_cannot_expose_reserved_document_identity() {
    let pool = database().await;
    item(&pool, "item-a", "vault-a", "source-a").await.unwrap();
    assert!(outcome(
        &pool,
        "attempt-a",
        "event-a",
        "vault-a",
        "failed",
        Some("reserved-document")
    )
    .await
    .is_err());
    assert!(
        outcome(&pool, "attempt-a", "event-a", "vault-a", "imported", None)
            .await
            .is_err()
    );
    outcome(&pool, "attempt-a", "event-a", "vault-a", "skipped", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn outcome_cannot_cross_the_items_vault_boundary() {
    let pool = database().await;
    item(&pool, "item-a", "vault-a", "source-a").await.unwrap();
    assert!(
        outcome(&pool, "attempt-a", "event-a", "vault-b", "failed", None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn expired_observation_is_reclaimed_and_stale_release_is_fenced() {
    use folio_core::models::{IngestClaim, IngestItemKey, SourceKind};
    use folio_infra::database::sqlite::SqliteIngestOutcomeRepository;
    let database = Database::connect("sqlite::memory:").await.unwrap();
    database.migrate().await.unwrap();
    let vault = database
        .repositories()
        .await
        .vaults
        .instance_default()
        .await
        .unwrap();
    let Database::Sqlite(pool) = database else {
        unreachable!()
    };
    let repository = SqliteIngestOutcomeRepository::new(pool.clone());
    let source = uuid::Uuid::new_v4();
    let key = IngestItemKey::folder(b"scan.pdf", b"generation", b"pdf");
    let fingerprint = "a".repeat(64);
    let IngestClaim::Claimed(first) = repository
        .claim(
            vault,
            source,
            SourceKind::Folder,
            folio_core::models::SourceOrigin::Config,
            &key,
            &fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("first observation must claim")
    };
    assert!(matches!(
        repository
            .claim(
                vault,
                source,
                SourceKind::Folder,
                folio_core::models::SourceOrigin::Config,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Busy
    ));
    sqlx::query("UPDATE ingest_items SET lease_expires_at = '2000-01-01T00:00:00Z'")
        .execute(&pool)
        .await
        .unwrap();

    let IngestClaim::Claimed(retry) = repository
        .claim(
            vault,
            source,
            SourceKind::Folder,
            folio_core::models::SourceOrigin::Config,
            &key,
            &fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("expired observation must be reclaimable")
    };

    assert_eq!(first.item_id, retry.item_id);
    assert_ne!(first.attempt_id, retry.attempt_id);
    assert!(!repository.release(&first).await.unwrap());
    assert!(repository.release(&retry).await.unwrap());
}

#[tokio::test]
async fn postgres_claims_reconsider_changed_skip_rules_and_page_with_vault_isolation() {
    use folio_core::models::{IngestClaim, IngestItemKey, IngestResult, SourceKind, VaultId};
    use folio_infra::database::postgres::PostgresIngestOutcomeRepository;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

    let started = Postgres::default().with_tag("17-alpine").start().await;
    let container = match started {
        Ok(container) => container,
        Err(error) => {
            assert!(
                !std::env::var("FOLIO_REQUIRE_DOCKER")
                    .is_ok_and(|value| !value.is_empty() && value != "0"),
                "PostgreSQL container required: {error}"
            );
            eprintln!("SKIPPED: PostgreSQL container unavailable: {error}");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let database = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    database.migrate().await.unwrap();
    let vault = database
        .repositories()
        .await
        .vaults
        .instance_default()
        .await
        .unwrap();
    let Database::Postgres(pool) = database else {
        unreachable!()
    };
    let repository = PostgresIngestOutcomeRepository::new(pool.clone());
    let source = uuid::Uuid::new_v4();
    let key = IngestItemKey::email("INBOX", 1, 7, 0).unwrap();
    let fingerprint = "a".repeat(64);
    let IngestClaim::Claimed(first) = repository
        .claim(
            vault,
            source,
            SourceKind::Email,
            folio_core::models::SourceOrigin::Config,
            &key,
            &fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("first observation must claim")
    };
    assert!(matches!(
        repository
            .claim(
                vault,
                source,
                SourceKind::Email,
                folio_core::models::SourceOrigin::Config,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Busy
    ));
    sqlx::query("UPDATE ingest_items SET state = 'skipped' WHERE id = $1")
        .bind(first.item_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        repository
            .claim(
                vault,
                source,
                SourceKind::Email,
                folio_core::models::SourceOrigin::Config,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Completed(IngestResult::Skipped)
    ));

    let IngestClaim::Claimed(retry) = repository
        .claim(
            vault,
            source,
            SourceKind::Email,
            folio_core::models::SourceOrigin::Config,
            &key,
            &"b".repeat(64),
        )
        .await
        .unwrap()
    else {
        panic!("changed rule must reconsider skipped item")
    };

    assert_eq!(first.item_id, retry.item_id);
    assert_ne!(first.attempt_id, retry.attempt_id);
    assert!(!repository.release(&first).await.unwrap());
    assert!(repository.release(&retry).await.unwrap());
    sqlx::query("INSERT INTO ingest_outcomes(vault_id, source_id, item_id, attempt_id, event_id, occurred_at, outcome) VALUES ($1,$2,$3,$4,$5,now(),'failed')")
        .bind(vault.0).bind(source).bind(first.item_id).bind(first.attempt_id).bind(uuid::Uuid::new_v4()).execute(&pool).await.unwrap();
    let page = repository.page(vault, source, None, 1).await.unwrap();
    assert_eq!(page.outcomes.len(), 1);
    assert_eq!(page.outcomes[0].attempt_id, first.attempt_id);
    assert!(repository
        .page(VaultId::new(), source, None, 1)
        .await
        .unwrap()
        .outcomes
        .is_empty());
    assert!(repository
        .page(vault, uuid::Uuid::new_v4(), None, 1)
        .await
        .unwrap()
        .outcomes
        .is_empty());
}

async fn durable_outcome_contract(database: Database) {
    use folio_core::domain_event::EventAudience;
    use folio_core::models::{IngestClaim, IngestItemKey, IngestResult, SourceKind};
    use folio_core::ports::{IngestOutcomeRepository, OutboxStore};
    use folio_infra::database::{
        postgres::PostgresIngestOutcomeRepository, sqlite::SqliteIngestOutcomeRepository,
    };
    use folio_infra::{PostgresOutboxStore, SqliteOutboxStore};

    match &database {
        Database::Sqlite(pool) => {
            let migrations = sqlx::migrate!("../../migrations/sqlite");
            let version = migrations
                .iter()
                .filter(|m| m.version < 20261102171000)
                .map(|m| m.version)
                .max()
                .unwrap();
            migrations.run_to(version, pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            let migrations = sqlx::migrate!("../../migrations/postgres");
            let version = migrations
                .iter()
                .filter(|m| m.version < 20261102171000)
                .map(|m| m.version)
                .max()
                .unwrap();
            migrations.run_to(version, pool).await.unwrap();
        }
    }
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let legacy = folio_core::models::LedgerEntry {
        vault_id: vault,
        key: "legacy-content".into(),
        source_id: uuid::Uuid::new_v4(),
        document_id: None,
        outcome: "imported".into(),
        created_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    };
    repos.ledger.insert(vault, &legacy).await.unwrap();
    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    let retained = repos
        .ledger
        .find(vault, &legacy.key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.outcome, legacy.outcome);
    assert_eq!(retained.created_at, legacy.created_at);
    assert!(repos
        .ingest_outcomes
        .page(vault, legacy.source_id, None, 10)
        .await
        .unwrap()
        .outcomes
        .is_empty());
    assert!(repos
        .outbox
        .claim(10, chrono::Duration::seconds(30), 10)
        .await
        .unwrap()
        .is_empty());
    let (repository, outbox): (Box<dyn IngestOutcomeRepository>, Box<dyn OutboxStore>) =
        match &database {
            Database::Sqlite(pool) => (
                Box::new(SqliteIngestOutcomeRepository::new(pool.clone())),
                Box::new(SqliteOutboxStore::new(pool.clone())),
            ),
            Database::Postgres(pool) => (
                Box::new(PostgresIngestOutcomeRepository::new(pool.clone())),
                Box::new(PostgresOutboxStore::new(pool.clone())),
            ),
        };
    let lineage = folio_core::mutation::MutationLineage {
        correlation_id: uuid::Uuid::new_v4(),
        causation_id: Some(uuid::Uuid::new_v4()),
    };
    for kind in [SourceKind::Folder, SourceKind::Email] {
        let source = uuid::Uuid::new_v4();
        let key = match kind {
            SourceKind::Folder => IngestItemKey::folder(b"private.pdf", b"stage", b"bytes"),
            _ => IngestItemKey::email("private mailbox", 1, 7, 0).unwrap(),
        };
        let fingerprint = "a".repeat(64);
        let IngestClaim::Claimed(attempt) = repository
            .claim(
                vault,
                source,
                kind,
                folio_core::models::SourceOrigin::Config,
                &key,
                &fingerprint,
            )
            .await
            .unwrap()
        else {
            panic!("claim required")
        };
        assert!(repository
            .complete(
                &attempt,
                IngestResult::Failed,
                Some(uuid::Uuid::new_v4()),
                None
            )
            .await
            .is_err());
        if kind == SourceKind::Folder {
            match &database {
                Database::Sqlite(pool) => {
                    sqlx::query("CREATE TRIGGER reject_ingest_event BEFORE INSERT ON outbox_events WHEN NEW.type LIKE 'ingest.%' BEGIN SELECT RAISE(ABORT, 'injected outbox failure'); END").execute(pool).await.unwrap();
                }
                Database::Postgres(pool) => {
                    sqlx::query("ALTER TABLE outbox_events ADD CONSTRAINT reject_ingest_event CHECK (type NOT LIKE 'ingest.%')").execute(pool).await.unwrap();
                }
            }
            assert!(repository
                .complete(&attempt, IngestResult::Skipped, None, None)
                .await
                .is_err());
            assert!(repository
                .page(vault, source, None, 10)
                .await
                .unwrap()
                .outcomes
                .is_empty());
            assert!(matches!(
                repository
                    .claim(
                        vault,
                        source,
                        kind,
                        folio_core::models::SourceOrigin::Config,
                        &key,
                        &fingerprint
                    )
                    .await
                    .unwrap(),
                IngestClaim::Busy
            ));
            match &database {
                Database::Sqlite(pool) => {
                    sqlx::query("DROP TRIGGER reject_ingest_event")
                        .execute(pool)
                        .await
                        .unwrap();
                }
                Database::Postgres(pool) => {
                    sqlx::query("ALTER TABLE outbox_events DROP CONSTRAINT reject_ingest_event")
                        .execute(pool)
                        .await
                        .unwrap();
                }
            }
        }
        let (first, replay) = tokio::join!(
            folio_core::mutation::with_lineage(
                Some(lineage),
                repository.complete(&attempt, IngestResult::Skipped, None, None)
            ),
            folio_core::mutation::with_lineage(
                Some(lineage),
                repository.complete(&attempt, IngestResult::Skipped, None, None)
            ),
        );
        let first = first.unwrap().unwrap();
        let replay = replay.unwrap().unwrap();
        assert_eq!(first.id, replay.id);
        assert_eq!(first.event_id, replay.event_id);
        assert!(matches!(
            repository
                .claim(
                    vault,
                    source,
                    kind,
                    folio_core::models::SourceOrigin::Config,
                    &key,
                    &fingerprint
                )
                .await
                .unwrap(),
            IngestClaim::Completed(IngestResult::Skipped)
        ));
        let IngestClaim::Claimed(retry) = repository
            .claim(
                vault,
                source,
                kind,
                folio_core::models::SourceOrigin::Config,
                &key,
                &"b".repeat(64),
            )
            .await
            .unwrap()
        else {
            panic!("changed decision must reopen")
        };
        assert_eq!(attempt.item_id, retry.item_id);
        assert_ne!(attempt.attempt_id, retry.attempt_id);
        let mut stale = attempt.clone();
        stale.attempt_id = uuid::Uuid::new_v4();
        assert!(repository
            .complete(&stale, IngestResult::Failed, None, None)
            .await
            .unwrap()
            .is_none());
        repository
            .complete(&retry, IngestResult::Failed, None, None)
            .await
            .unwrap()
            .unwrap();
        let page = repository.page(vault, source, None, 1).await.unwrap();
        assert_eq!(page.outcomes[0].attempt_id, retry.attempt_id);
        assert!(page.next_before.is_some());
    }
    let events = outbox
        .claim(10, chrono::Duration::seconds(30), 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 4);
    for event in events {
        assert!(event.event.is_none());
        let envelope = event.envelope.unwrap();
        assert_eq!(envelope.audience(), &EventAudience::Owners);
        assert_eq!(envelope.document_id(), None);
        assert_eq!(envelope.vault_id(), vault);
        assert_eq!(envelope.subject().subject_type, "ingest");
        assert_eq!(envelope.data()["payload_version"], 1);
        assert_eq!(
            envelope.mutation_id().to_string(),
            envelope.data()["attempt_id"].as_str().unwrap()
        );
        if envelope.event_type() == "ingest.skipped" {
            assert_eq!(envelope.correlation_id(), lineage.correlation_id);
            assert_eq!(envelope.causation_id(), lineage.causation_id);
        }
        assert!(!envelope.data().to_string().contains("private"));
    }
    let repos = database.repositories().await;
    for result in [IngestResult::Imported, IngestResult::Failed] {
        let source = uuid::Uuid::new_v4();
        let key = IngestItemKey::folder(b"ledger.pdf", source.as_bytes(), b"ledger");
        let IngestClaim::Claimed(attempt) = repository
            .claim(
                vault,
                source,
                SourceKind::Folder,
                folio_core::models::SourceOrigin::Config,
                &key,
                &"c".repeat(64),
            )
            .await
            .unwrap()
        else {
            panic!("claim required")
        };
        let document = repos
            .documents
            .create(
                vault,
                folio_core::models::CreateDocument {
                    title: "ledger".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        repos
            .documents
            .set_file_path(vault, document.id, "test-original")
            .await
            .unwrap();
        let claim = folio_core::models::LedgerEntry {
            vault_id: vault,
            key: format!("contract-{source}"),
            source_id: source,
            document_id: Some(document.id),
            outcome: "claimed".into(),
            created_at: chrono::DateTime::from_timestamp_micros(
                chrono::Utc::now().timestamp_micros(),
            )
            .unwrap(),
        };
        repos.ledger.insert(vault, &claim).await.unwrap();
        let document_id = (result == IngestResult::Imported).then_some(document.id);
        let mut stale = claim.clone();
        stale.created_at -= chrono::Duration::seconds(1);
        assert!(repository
            .complete(&attempt, result, document_id, Some(&stale))
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            repos
                .ledger
                .find(vault, &claim.key)
                .await
                .unwrap()
                .unwrap()
                .outcome,
            "claimed"
        );
        let record = repository
            .complete(&attempt, result, document_id, Some(&claim))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.document_id, document_id);
        assert_eq!(
            repos
                .ledger
                .find(vault, &claim.key)
                .await
                .unwrap()
                .unwrap()
                .outcome,
            if result == IngestResult::Imported {
                "imported"
            } else {
                "retryable"
            }
        );
        assert_eq!(
            repository
                .complete(&attempt, result, document_id, Some(&claim))
                .await
                .unwrap()
                .unwrap()
                .event_id,
            record.event_id
        );
    }
}

#[tokio::test]
async fn sqlite_outcomes_publish_once_and_reopen_only_for_new_attempts() {
    durable_outcome_contract(Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_outcomes_publish_once_and_reopen_only_for_new_attempts() {
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
    durable_outcome_contract(
        Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}

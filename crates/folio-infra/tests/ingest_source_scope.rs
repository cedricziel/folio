use folio_core::models::{
    FolderAfterImport, FolderConfig, IngestClaim, IngestItemKey, IngestResult,
    IngestionSourceInput, SourceConfig, SourceKind, SourceOrigin,
};
use folio_infra::Database;

async fn scope_contract(database: Database) {
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let other = repos.vaults.create("Other", None).await.unwrap().id;
    let input = IngestionSourceInput {
        name: "scope contract".into(),
        enabled: true,
        config: SourceConfig::Folder(FolderConfig {
            path: "/unused-test-source".into(),
            recursive: false,
            subfolders_as_tags: false,
            after_import: FolderAfterImport::Delete,
            settle_secs: 0,
            rescan_secs: 60,
        }),
    };
    let source = repos.sources.create(vault, input.clone()).await.unwrap();
    let key = IngestItemKey::folder(b"private.pdf", b"stage", b"source scope");
    let fingerprint = "d".repeat(64);
    assert!(matches!(
        repos
            .ingest_outcomes
            .claim(
                other,
                source.id,
                SourceKind::Folder,
                SourceOrigin::Managed,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Busy
    ));
    let IngestClaim::Claimed(attempt) = repos
        .ingest_outcomes
        .claim(
            vault,
            source.id,
            SourceKind::Folder,
            SourceOrigin::Managed,
            &key,
            &fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("active managed source must claim")
    };
    repos
        .sources
        .update(
            vault,
            source.id,
            IngestionSourceInput {
                enabled: false,
                ..input
            },
        )
        .await
        .unwrap();
    assert!(repos
        .ingest_outcomes
        .complete(&attempt, IngestResult::Skipped, None, None)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .ingest_outcomes
        .page(vault, source.id, None, 10)
        .await
        .unwrap()
        .outcomes
        .is_empty());
    repos.sources.delete(vault, source.id).await.unwrap();
    assert!(matches!(
        repos
            .ingest_outcomes
            .claim(
                vault,
                source.id,
                SourceKind::Folder,
                SourceOrigin::Managed,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Busy
    ));
    let configured = uuid::Uuid::new_v4();
    assert!(matches!(
        repos
            .ingest_outcomes
            .claim(
                vault,
                configured,
                SourceKind::Folder,
                SourceOrigin::Config,
                &key,
                &fingerprint
            )
            .await
            .unwrap(),
        IngestClaim::Claimed(_)
    ));
}

#[tokio::test]
async fn sqlite_managed_source_must_still_be_active_in_its_vault() {
    scope_contract(Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_managed_source_must_still_be_active_in_its_vault() {
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
    scope_contract(
        Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}

//! One live Paperless source per vault, enforced inside the write transaction

mod common;

use folio_core::error::Error;
use folio_core::models::IngestionSourceInput;
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

fn paperless(name: &str) -> IngestionSourceInput {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "config": {"type": "paperless", "url": "https://paperless.example", "token": "t"}
    }))
    .unwrap()
}

fn folder(name: &str) -> IngestionSourceInput {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "config": {"type": "folder", "path": "/unused/scans"}
    }))
    .unwrap()
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();

    let attempts = (0..8).map(|n| {
        let sources = r.sources.clone();
        tokio::spawn(async move { sources.create(vault, paperless(&format!("P{n}"))).await })
    });
    let mut created = Vec::new();
    for attempt in attempts {
        match attempt.await.unwrap() {
            Ok(def) => created.push(def),
            Err(Error::Conflict(_)) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    let [live] = created.as_slice() else {
        panic!("exactly one concurrent create wins, got {}", created.len())
    };

    let scans = r.sources.create(vault, folder("Scans")).await.unwrap();
    assert!(matches!(
        r.sources.update(vault, scans.id, paperless("Scans")).await,
        Err(Error::Conflict(_))
    ));
    r.sources
        .update(vault, live.id, paperless("Renamed"))
        .await
        .expect("a source keeps its own slot");

    r.sources.delete(vault, live.id).await.unwrap();
    let second = r.sources.create(vault, paperless("Second")).await.unwrap();
    assert!(matches!(
        r.sources.restore(vault, live.id).await,
        Err(Error::Conflict(_))
    ));
    r.sources.delete(vault, second.id).await.unwrap();
    r.sources.restore(vault, live.id).await.unwrap();
}

#[tokio::test]
async fn sqlite_allows_one_live_paperless_source_per_vault() {
    let dir = tempfile::tempdir().unwrap();
    contract(
        Database::connect(&format!("sqlite:{}", dir.path().join("slot.db").display()))
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn postgres_allows_one_live_paperless_source_per_vault() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL Paperless source slot",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    contract(Database::connect(&url).await.unwrap()).await;
}

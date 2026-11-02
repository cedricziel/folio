mod common;

use folio_core::models::{CreateDocument, Id, UpdateDocument, UserId};
use folio_core::mutation::with_actor;
use folio_infra::Database;
use serde_json::Value;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

async fn payloads(db: &Database) -> Vec<Value> {
    let rows: Vec<(String,)> = match db {
        Database::Sqlite(pool) => sqlx::query_as("SELECT payload FROM outbox_events ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => {
            sqlx::query_as("SELECT payload::text FROM outbox_events ORDER BY id")
                .fetch_all(pool)
                .await
                .unwrap()
        }
    };
    rows.into_iter()
        .map(|(payload,)| serde_json::from_str(&payload).unwrap())
        .collect()
}

#[path = "document_transitions/classifications.rs"]
mod classifications;
#[path = "document_transitions/tags.rs"]
mod tags;
#[path = "document_transitions/title.rs"]
mod title;

mod common;

use chrono::{DateTime, Duration};
use folio_core::domain_event::{
    DomainEventEnvelope, DomainEventSubject, EventAudience, DOCUMENT_METADATA_CHANGED,
};
use folio_core::events::encode_stored_outbox_payload;
use folio_core::models::VaultId;
use folio_core::{EventOrigin, OutboxEvent};
use folio_infra::Database;
use serde_json::json;
use uuid::Uuid;

const MIB: usize = 1_048_576;

async fn payload_size(db: &Database, payload: &str) -> usize {
    let bytes: i64 = match db {
        Database::Sqlite(pool) => sqlx::query_scalar("SELECT length(CAST(? AS BLOB))")
            .bind(payload)
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => {
            sqlx::query_scalar("SELECT octet_length($1::jsonb::text)::bigint")
                .bind(payload)
                .fetch_one(pool)
                .await
                .unwrap()
        }
    };
    usize::try_from(bytes).unwrap()
}

fn encode(event: &DomainEventEnvelope) -> String {
    encode_stored_outbox_payload(None, EventOrigin::User, None, Some(event)).unwrap()
}

async fn sized_event(db: &Database, bytes: usize) -> DomainEventEnvelope {
    let document = Uuid::new_v4();
    let event = DomainEventEnvelope::new(
        DOCUMENT_METADATA_CHANGED,
        VaultId::new(),
        DomainEventSubject {
            subject_type: "document".into(),
            id: document,
        },
        Some(document),
        EventAudience::Vault,
        json!({"text": "🦀".repeat(200_000)}),
    )
    .unwrap();
    let padding = bytes - payload_size(db, &encode(&event)).await;
    let mut stored = serde_json::to_value(event).unwrap();
    stored["data"]["text"] = json!(format!("{}{}", "🦀".repeat(200_000), "x".repeat(padding)));
    let event = serde_json::from_value(stored).unwrap();
    assert_eq!(payload_size(db, &encode(&event)).await, bytes);
    event
}

async fn insert(db: &Database, event: &DomainEventEnvelope) -> i64 {
    let created = DateTime::UNIX_EPOCH + Duration::hours(1);
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar(
            "INSERT INTO outbox_events (type, payload, created_at, available_at) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(event.event_type())
        .bind(encode(event))
        .bind(created.to_rfc3339())
        .bind(created.to_rfc3339())
        .fetch_one(pool)
        .await
        .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(
            "INSERT INTO outbox_events (type, payload, created_at, available_at) VALUES ($1, $2::jsonb, $3, $3) RETURNING id",
        )
        .bind(event.event_type())
        .bind(encode(event))
        .bind(created)
        .fetch_one(pool)
        .await
        .unwrap(),
    }
}

async fn replay(db: &Database, limit: u32) -> Vec<OutboxEvent> {
    db.repositories()
        .await
        .outbox
        .replay(0, DateTime::UNIX_EPOCH, limit)
        .await
        .unwrap()
}

async fn clear(db: &Database) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DELETE FROM outbox_events")
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DELETE FROM outbox_events")
                .execute(pool)
                .await
                .unwrap();
        }
    };
}

async fn assert_budget(db: &Database, bytes: usize, expected: usize) {
    let mut events = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..9 {
        let event = sized_event(db, bytes).await;
        ids.push(insert(db, &event).await);
        events.push(event);
    }

    let replayed = replay(db, 500).await;
    assert_eq!(
        replayed.iter().map(|event| event.id).collect::<Vec<_>>(),
        ids[9 - expected..],
        "replay must return the newest complete events within 8 MiB, in ID order"
    );
    assert_eq!(
        replayed
            .iter()
            .map(|event| event.envelope.as_ref().unwrap())
            .collect::<Vec<_>>(),
        events[9 - expected..].iter().collect::<Vec<_>>(),
        "the byte bound must preserve every selected payload"
    );
    assert_eq!(
        replay(db, 2)
            .await
            .iter()
            .map(|event| event.id)
            .collect::<Vec<_>>(),
        ids[7..],
        "the requested event count remains a separate bound"
    );
    assert!(replay(db, 0).await.is_empty());
    clear(db).await;
}

async fn replay_contract(db: &Database) {
    assert_budget(db, MIB, 8).await;
    assert_budget(db, MIB + 1, 7).await;
}

#[tokio::test]
async fn sqlite_replay_bounds_complete_utf8_payloads() {
    let (_directory, db) = common::sqlite_database().await;
    replay_contract(&db).await;
}

#[tokio::test]
async fn postgres_replay_bounds_complete_utf8_payloads() {
    let Some((_container, db)) = common::postgres_database("outbox replay").await else {
        return;
    };
    replay_contract(&db).await;
}

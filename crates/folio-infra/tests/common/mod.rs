//! Helpers shared by the container-backed integration tests.

#![allow(dead_code)]

use std::fmt::Display;

pub mod ingest;

/// Unwrap a container start result, or decide what an unreachable Docker means.
///
/// Without `FOLIO_REQUIRE_DOCKER` the test is skipped with a message; with it
/// (CI sets it) the test fails, so a broken runner cannot hide missing coverage.
pub fn container_or_skip<T, E: Display>(what: &str, started: Result<T, E>) -> Option<T> {
    match started {
        Ok(container) => Some(container),
        Err(e) if require_docker() => {
            panic!("FOLIO_REQUIRE_DOCKER is set but {what} could not start: {e}")
        }
        Err(e) => {
            eprintln!("SKIPPED: {what} needs a Docker daemon ({e})");
            None
        }
    }
}

fn require_docker() -> bool {
    std::env::var("FOLIO_REQUIRE_DOCKER").is_ok_and(|v| !v.is_empty() && v != "0")
}

pub const MINIO_USER: &str = "folio-test-user";
pub const MINIO_PASSWORD: &str = "folio-test-password";

/// Create a bucket with a hand-signed SigV4 request; `object_store` cannot
/// create buckets.
pub async fn create_bucket(endpoint: &str, bucket: &str) {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::{Digest, Sha256};

    fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
        mac.update(data.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    let host = endpoint.trim_start_matches("http://");
    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let payload_hash = hex(&Sha256::digest(b""));
    let canonical = format!(
        "PUT\n/{bucket}\n\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n\nhost;x-amz-content-sha256;x-amz-date\n{payload_hash}"
    );
    let scope = format!("{date}/us-east-1/s3/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let mut key = hmac_sha256(format!("AWS4{MINIO_PASSWORD}").as_bytes(), &date);
    for part in ["us-east-1", "s3", "aws4_request"] {
        key = hmac_sha256(&key, part);
    }
    let signature = hex(&hmac_sha256(&key, &to_sign));

    let response = reqwest::Client::new()
        .put(format!("{endpoint}/{bucket}"))
        .header("x-amz-date", &amz_date)
        .header("x-amz-content-sha256", &payload_hash)
        .header(
            "Authorization",
            format!(
                "AWS4-HMAC-SHA256 Credential={MINIO_USER}/{scope}, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={signature}"
            ),
        )
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "creating bucket failed: {} {}",
        response.status(),
        response.text().await.unwrap_or_default()
    );
}

/// Seed a vault with SQL that predates vault lifecycle columns, for upgrade tests
/// that stop migrations before the current schema.
pub async fn legacy_vault(db: &folio_infra::Database) -> folio_core::models::VaultId {
    let id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    match db {
        folio_infra::Database::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO vaults (id, name, created_at, updated_at) VALUES (?, 'Legacy', ?, ?)",
            )
            .bind(id.to_string())
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await
            .unwrap();
        }
        folio_infra::Database::Postgres(pool) => {
            sqlx::query("INSERT INTO vaults (id, name, created_at, updated_at) VALUES ($1, 'Legacy', $2, $2)")
                .bind(id)
                .bind(now)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    folio_core::models::VaultId(id)
}

/// A migrated SQLite database in a temporary directory; keep the directory alive.
pub async fn sqlite_database() -> (tempfile::TempDir, folio_infra::Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = folio_infra::Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("test.db").display()
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    (dir, db)
}

/// A migrated PostgreSQL database in a fresh container, or `None` when Docker
/// is unavailable and not required; keep the container alive.
pub async fn postgres_database(
    what: &str,
) -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    folio_infra::Database,
)> {
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
    let container = container_or_skip(
        what,
        testcontainers_modules::postgres::Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await,
    )?;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = folio_infra::Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    Some((container, db))
}

/// A filed, text-bearing Paperless document with no taxonomy, keyed by `external_id`
pub fn imported_document(
    vault: folio_core::models::VaultId,
    external_id: &str,
    added: chrono::DateTime<chrono::Utc>,
) -> folio_core::models::ImportedDocument {
    use folio_core::models::{ExternalRef, Id, ImportSource, ImportedDocument};
    ImportedDocument {
        id: Id::new_v5(
            &Id::NAMESPACE_URL,
            format!("{vault}/{external_id}").as_bytes(),
        ),
        external_ref: ExternalRef {
            source: ImportSource::Paperless,
            id: external_id.into(),
        },
        title: "Electricity bill".into(),
        content: Some("Total due 42.00".into()),
        original_filename: Some("bill.pdf".into()),
        mime_type: Some("application/pdf".into()),
        file_path: format!("documents/{external_id}/original"),
        content_sha256: "ab".repeat(32),
        document_date: Some("2019-03-04".parse().unwrap()),
        created_at: added,
        updated_at: added + chrono::Duration::days(2),
        correspondent_id: None,
        document_type_id: None,
        tag_ids: Vec::new(),
        metadata: Default::default(),
        notes: Vec::new(),
        in_inbox: false,
    }
}

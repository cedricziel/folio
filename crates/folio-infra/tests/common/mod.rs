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

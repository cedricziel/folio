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

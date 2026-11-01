//! Liveness and readiness probes
//!
//! These routes are unauthenticated by design: orchestrators and reverse
//! proxies call them without credentials. Keep them out of any auth layer.

use std::{sync::Arc, time::Duration};

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use folio_core::ports::HealthCheck;
use futures::future::join_all;
use serde_json::{json, Map, Value};

/// Maximum time a single check may take before it is reported as failed.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Registered readiness checks.
pub type HealthChecks = Arc<[Arc<dyn HealthCheck>]>;

/// Router for `/healthz` and `/readyz`, independent of [`crate::AppState`].
pub fn router(checks: HealthChecks) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(checks)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn readyz(State(checks): State<HealthChecks>) -> (StatusCode, Json<Value>) {
    let results = join_all(checks.iter().map(|check| async move {
        let outcome = match tokio::time::timeout(CHECK_TIMEOUT, check.check()).await {
            Ok(result) => result,
            Err(_) => Err(format!("timed out after {}s", CHECK_TIMEOUT.as_secs())),
        };
        (check.name().to_string(), outcome)
    }))
    .await;

    let ready = results.iter().all(|(_, outcome)| outcome.is_ok());
    let checks: Map<String, Value> = results
        .into_iter()
        .map(|(name, outcome)| {
            let value = match outcome {
                Ok(()) => "ok".to_string(),
                Err(error) => {
                    tracing::warn!(check = %name, %error, "readiness check failed");
                    error
                }
            };
            (name, Value::String(value))
        })
        .collect();

    let (status, label) = if ready {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
    };
    (status, Json(json!({ "status": label, "checks": checks })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use folio_infra::{create_pool, DatabaseHealthCheck};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn get_json(app: Router, path: &str) -> (StatusCode, Value) {
        let response = app
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn db_router(close: bool) -> Router {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        if close {
            pool.close().await;
        }
        let check: Arc<dyn HealthCheck> = Arc::new(DatabaseHealthCheck::new(pool));
        router(Arc::from(vec![check]))
    }

    struct Hanging;

    #[async_trait::async_trait]
    impl HealthCheck for Hanging {
        fn name(&self) -> &str {
            "slow"
        }
        async fn check(&self) -> Result<(), String> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn healthz_is_always_ok() {
        let (status, body) = get_json(router(Arc::from(vec![])), "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "status": "ok" }));
    }

    #[tokio::test]
    async fn readyz_ok_with_healthy_database() {
        let (status, body) = get_json(db_router(false).await, "/readyz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({ "status": "ok", "checks": { "database": "ok" } })
        );
    }

    #[tokio::test]
    async fn readyz_unavailable_when_pool_is_closed() {
        let (status, body) = get_json(db_router(true).await, "/readyz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "unavailable");
        assert_ne!(body["checks"]["database"], "ok");
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_fails_a_check_that_times_out() {
        let check: Arc<dyn HealthCheck> = Arc::new(Hanging);
        let (status, body) = get_json(router(Arc::from(vec![check])), "/readyz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["checks"]["slow"], "timed out after 2s");
    }
}

//! Health check port

use async_trait::async_trait;

/// A named dependency probe used by the readiness endpoint.
#[async_trait]
pub trait HealthCheck: Send + Sync {
    /// Stable identifier reported in the readiness response (for example `database`).
    fn name(&self) -> &str;

    /// Probe the dependency; the error is a short, human-readable reason.
    async fn check(&self) -> Result<(), String>;
}

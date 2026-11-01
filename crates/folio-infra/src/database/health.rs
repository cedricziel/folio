//! Database health check

use super::Database;
use async_trait::async_trait;
use folio_core::ports::HealthCheck;

/// Verifies the database answers a trivial query.
pub struct DatabaseHealthCheck {
    database: Database,
}

impl DatabaseHealthCheck {
    pub fn new(database: impl Into<Database>) -> Self {
        Self {
            database: database.into(),
        }
    }
}

#[async_trait]
impl HealthCheck for DatabaseHealthCheck {
    fn name(&self) -> &str {
        "database"
    }

    async fn check(&self) -> Result<(), String> {
        let result = match &self.database {
            Database::Sqlite(pool) => sqlx::query("SELECT 1").execute(pool).await.map(|_| ()),
            Database::Postgres(pool) => sqlx::query("SELECT 1").execute(pool).await.map(|_| ()),
        };
        result.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_pool;

    #[tokio::test]
    async fn healthy_pool_passes() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        assert!(DatabaseHealthCheck::new(pool).check().await.is_ok());
    }

    #[tokio::test]
    async fn closed_pool_fails() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        pool.close().await;
        assert!(DatabaseHealthCheck::new(pool).check().await.is_err());
    }
}

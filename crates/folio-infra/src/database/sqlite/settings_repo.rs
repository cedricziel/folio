//! SQLite settings repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::instrument;

use folio_core::error::{Error, Result};
use folio_core::models::Setting;
use folio_core::ports::SettingsRepository;

/// SQLite-backed settings repository
#[derive(Clone)]
pub struct SqliteSettingsRepository {
    pool: SqlitePool,
}

impl SqliteSettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SettingsRepository for SqliteSettingsRepository {
    #[instrument(skip(self))]
    async fn get(&self, key: &str) -> Result<Option<Setting>> {
        let row: Option<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT key, value, description, updated_at FROM settings WHERE key = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Repository(e.to_string()))?;

        Ok(row.map(|(key, value, description, updated_at)| Setting {
            key,
            value,
            description,
            updated_at: DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }))
    }

    #[instrument(skip(self))]
    async fn list(&self) -> Result<Vec<Setting>> {
        let rows: Vec<(String, String, Option<String>, String)> =
            sqlx::query_as("SELECT key, value, description, updated_at FROM settings ORDER BY key")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| Error::Repository(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(key, value, description, updated_at)| Setting {
                key,
                value,
                description,
                updated_at: DateTime::parse_from_rfc3339(&updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }

    #[instrument(skip(self))]
    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Setting>> {
        let pattern = format!("{}%", prefix);
        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT key, value, description, updated_at FROM settings WHERE key LIKE ? ORDER BY key",
        )
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Repository(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(key, value, description, updated_at)| Setting {
                key,
                value,
                description,
                updated_at: DateTime::parse_from_rfc3339(&updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }

    #[instrument(skip(self, value))]
    async fn set(&self, key: &str, value: &str) -> Result<Setting> {
        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO settings (key, value, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(key)
        .bind(value)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(e.to_string()))?;

        // Fetch the updated setting
        self.get(key)
            .await?
            .ok_or_else(|| Error::Repository("Failed to retrieve setting after update".to_string()))
    }

    #[instrument(skip(self, settings))]
    async fn set_many(&self, settings: &[(String, String)]) -> Result<()> {
        let now = Utc::now();

        for (key, value) in settings {
            sqlx::query(
                r#"
                INSERT INTO settings (key, value, updated_at)
                VALUES (?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(key)
            .bind(value)
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Repository(e.to_string()))?;
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Repository(e.to_string()))?;

        Ok(())
    }
}

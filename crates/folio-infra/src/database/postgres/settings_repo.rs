//! PostgreSQL settings repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::instrument;

use folio_core::error::{Error, Result};
use folio_core::models::Setting;
use folio_core::ports::SettingsRepository;

/// PostgreSQL-backed settings repository
#[derive(Clone)]
pub struct PostgresSettingsRepository {
    pool: PgPool,
}

impl PostgresSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct SettingRow {
    key: String,
    value: String,
    description: Option<String>,
    updated_at: DateTime<Utc>,
}

impl From<SettingRow> for Setting {
    fn from(row: SettingRow) -> Self {
        Setting {
            key: row.key,
            value: row.value,
            description: row.description,
            updated_at: row.updated_at,
        }
    }
}

fn repo_err(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

const UPSERT: &str = "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
     ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = EXCLUDED.updated_at";

#[async_trait]
impl SettingsRepository for PostgresSettingsRepository {
    #[instrument(skip(self))]
    async fn get(&self, key: &str) -> Result<Option<Setting>> {
        let row: Option<SettingRow> = sqlx::query_as(
            "SELECT key, value, description, updated_at FROM settings WHERE key = $1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(row.map(Into::into))
    }

    #[instrument(skip(self))]
    async fn list(&self) -> Result<Vec<Setting>> {
        let rows: Vec<SettingRow> =
            sqlx::query_as("SELECT key, value, description, updated_at FROM settings ORDER BY key")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    #[instrument(skip(self))]
    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Setting>> {
        let rows: Vec<SettingRow> = sqlx::query_as(
            "SELECT key, value, description, updated_at FROM settings
             WHERE key LIKE $1 ORDER BY key",
        )
        .bind(format!("{}%", prefix))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    #[instrument(skip(self, value))]
    async fn set(&self, key: &str, value: &str) -> Result<Setting> {
        sqlx::query(UPSERT)
            .bind(key)
            .bind(value)
            .bind(Utc::now())
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        self.get(key)
            .await?
            .ok_or_else(|| Error::Repository("Failed to retrieve setting after update".to_string()))
    }

    #[instrument(skip(self, settings))]
    async fn set_many(&self, settings: &[(String, String)]) -> Result<()> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await.map_err(repo_err)?;

        for (key, value) in settings {
            sqlx::query(UPSERT)
                .bind(key)
                .bind(value)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
        }

        tx.commit().await.map_err(repo_err)
    }

    #[instrument(skip(self))]
    async fn delete(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM settings WHERE key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;

        Ok(())
    }
}

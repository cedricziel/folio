//! SQLite storage for push devices and pending notifications

use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::models::{
    DeviceRegistration, DocumentOutcome, Id, NotificationKind, PendingNotification, PushDevice,
    PushDeviceState, PushToken, PushTokenKind, UserId, VaultId,
};
use folio_core::ports::{GrantLiveness, PushDeviceRepository};

/// Fixed-width UTC timestamps, so text comparison orders them correctly
fn ts(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_ts(value: Option<String>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|v| v.parse::<DateTime<Utc>>().map_err(err))
        .transpose()
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

const DEVICE_COLS: &str = "id, user_id, installation_id, client_id, grant_id, platform, \
    environment, locale, preferences, alert_token, alert_token_registered_at, widget_token, \
    widget_token_registered_at, created_at, updated_at, last_delivered_at";

fn token(value: Option<String>, at: Option<String>) -> Result<Option<PushToken>> {
    Ok(match (value, parse_ts(at)?) {
        (Some(value), Some(registered_at)) => Some(PushToken {
            value,
            registered_at,
        }),
        _ => None,
    })
}

fn device(row: &SqliteRow) -> Result<PushDevice> {
    Ok(PushDevice {
        id: row.get::<String, _>("id").parse().map_err(err)?,
        user_id: UserId(row.get("user_id")),
        installation_id: row.get("installation_id"),
        client_id: row.get("client_id"),
        grant_id: row.get("grant_id"),
        platform: row.get::<String, _>("platform").parse().map_err(err)?,
        environment: row.get::<String, _>("environment").parse().map_err(err)?,
        locale: row.get("locale"),
        preferences: serde_json::from_str(&row.get::<String, _>("preferences")).map_err(err)?,
        alert_token: token(row.get("alert_token"), row.get("alert_token_registered_at"))?,
        widget_token: token(
            row.get("widget_token"),
            row.get("widget_token_registered_at"),
        )?,
        created_at: row.get::<String, _>("created_at").parse().map_err(err)?,
        updated_at: row.get::<String, _>("updated_at").parse().map_err(err)?,
        last_delivered_at: parse_ts(row.get("last_delivered_at"))?,
    })
}

fn pending(row: &SqliteRow) -> Result<PendingNotification> {
    Ok(PendingNotification {
        device_id: row.get::<String, _>("device_id").parse().map_err(err)?,
        document_id: row.get::<String, _>("document_id").parse().map_err(err)?,
        kind: row.get::<String, _>("kind").parse().map_err(err)?,
        vault_id: VaultId(row.get::<String, _>("vault_id").parse().map_err(err)?),
        created_at: row.get::<String, _>("created_at").parse().map_err(err)?,
    })
}

pub struct SqlitePushRepository {
    pool: SqlitePool,
}

impl SqlitePushRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn one(&self, sql: &str, binds: &[&str]) -> Result<Option<PushDevice>> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLS} FROM push_devices WHERE {sql}"
        )));
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?
            .as_ref()
            .map(device)
            .transpose()
    }

    async fn many(&self, sql: &str, binds: &[&str]) -> Result<Vec<PushDevice>> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLS} FROM push_devices {sql} ORDER BY created_at, id"
        )));
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_all(&self.pool)
            .await
            .map_err(err)?
            .iter()
            .map(device)
            .collect()
    }
}

#[async_trait]
impl PushDeviceRepository for SqlitePushRepository {
    async fn upsert(&self, r: DeviceRegistration) -> Result<(PushDevice, bool)> {
        let now = ts(Utc::now());
        let preferences = serde_json::to_string(&r.preferences).map_err(err)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM push_devices WHERE user_id = ? AND installation_id = ?",
        )
        .bind(r.user_id.as_str())
        .bind(&r.installation_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(err)?;
        let id = existing
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // A token belongs to one installation of the server; take it from any other row,
        // and delete only those rows that are left without a token.
        let mut emptied: Vec<String> = Vec::new();
        for (column, value) in [
            ("alert_token", &r.alert_token),
            ("widget_token", &r.widget_token),
        ] {
            if let Some(value) = value {
                let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "UPDATE push_devices SET {column} = NULL, {column}_registered_at = NULL \
                     WHERE {column} = ? AND id <> ? RETURNING id"
                )))
                .bind(value)
                .bind(&id)
                .fetch_all(&mut *tx)
                .await
                .map_err(err)?;
                emptied.extend(ids);
            }
        }
        for other in &emptied {
            sqlx::query(
                "DELETE FROM push_devices WHERE id = ? AND alert_token IS NULL AND widget_token IS NULL",
            )
            .bind(other)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }

        if existing.is_some() {
            sqlx::query(
                "UPDATE push_devices SET client_id = ?, grant_id = ?, platform = ?, environment = ?,
                    locale = ?, preferences = ?, updated_at = ?,
                    alert_token = COALESCE(?, alert_token),
                    alert_token_registered_at = CASE WHEN ? IS NULL THEN alert_token_registered_at ELSE ? END,
                    widget_token = COALESCE(?, widget_token),
                    widget_token_registered_at = CASE WHEN ? IS NULL THEN widget_token_registered_at ELSE ? END
                 WHERE id = ?",
            )
            .bind(&r.client_id)
            .bind(&r.grant_id)
            .bind(r.platform.as_str())
            .bind(r.environment.as_str())
            .bind(&r.locale)
            .bind(&preferences)
            .bind(&now)
            .bind(&r.alert_token)
            .bind(&r.alert_token)
            .bind(&now)
            .bind(&r.widget_token)
            .bind(&r.widget_token)
            .bind(&now)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        } else {
            sqlx::query(
                "INSERT INTO push_devices (id, user_id, installation_id, client_id, grant_id,
                    platform, environment, locale, preferences, alert_token, alert_token_registered_at,
                    widget_token, widget_token_registered_at, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(r.user_id.as_str())
            .bind(&r.installation_id)
            .bind(&r.client_id)
            .bind(&r.grant_id)
            .bind(r.platform.as_str())
            .bind(r.environment.as_str())
            .bind(&r.locale)
            .bind(&preferences)
            .bind(&r.alert_token)
            .bind(r.alert_token.as_ref().map(|_| &now))
            .bind(&r.widget_token)
            .bind(r.widget_token.as_ref().map(|_| &now))
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }
        tx.commit().await.map_err(err)?;
        let device = self
            .one("id = ?", &[&id])
            .await?
            .ok_or_else(|| err("registered device vanished"))?;
        Ok((device, existing.is_none()))
    }

    async fn get(&self, id: Uuid) -> Result<Option<PushDevice>> {
        self.one("id = ?", &[&id.to_string()]).await
    }

    async fn find(&self, user: &UserId, installation_id: &str) -> Result<Option<PushDevice>> {
        self.one(
            "user_id = ? AND installation_id = ?",
            &[user.as_str(), installation_id],
        )
        .await
    }

    async fn list_for_user(&self, user: &UserId) -> Result<Vec<PushDevice>> {
        self.many("WHERE user_id = ?", &[user.as_str()]).await
    }

    async fn list_all(&self) -> Result<Vec<PushDevice>> {
        self.many("", &[]).await
    }

    async fn delete(&self, user: &UserId, installation_id: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM push_devices WHERE user_id = ? AND installation_id = ?")
                .bind(user.as_str())
                .bind(installation_id)
                .execute(&self.pool)
                .await
                .map_err(err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_by_id(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM push_devices WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn clear_token(
        &self,
        device: Uuid,
        kind: PushTokenKind,
        inactive_since: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let column = match kind {
            PushTokenKind::Alert => "alert_token",
            PushTokenKind::Widget => "widget_token",
        };
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
        let cleared = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE push_devices SET {column} = NULL, {column}_registered_at = NULL
             WHERE id = ? AND {column} IS NOT NULL
               AND (? IS NULL OR {column}_registered_at < ?)"
        )))
        .bind(device.to_string())
        .bind(inactive_since.map(ts))
        .bind(inactive_since.map(ts))
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected()
            > 0;
        sqlx::query(
            "DELETE FROM push_devices WHERE id = ? AND alert_token IS NULL AND widget_token IS NULL",
        )
        .bind(device.to_string())
        .execute(&mut *tx)
        .await
        .map_err(err)?;
        tx.commit().await.map_err(err)?;
        Ok(cleared)
    }

    async fn record_first_outcome(
        &self,
        document: Id,
        outcome: DocumentOutcome,
        items: &[PendingNotification],
    ) -> Result<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
        let first = sqlx::query(
            "INSERT INTO push_document_outcomes (document_id, outcome, created_at)
             SELECT ?, ?, ? WHERE EXISTS (SELECT 1 FROM documents WHERE id = ?)
             ON CONFLICT DO NOTHING",
        )
        .bind(document.to_string())
        .bind(outcome.as_str())
        .bind(ts(Utc::now()))
        .bind(document.to_string())
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected()
            > 0;
        if first {
            for item in items {
                sqlx::query(
                    "INSERT INTO push_pending (device_id, document_id, kind, vault_id, created_at)
                     SELECT ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM push_devices WHERE id = ?)
                     ON CONFLICT DO NOTHING",
                )
                .bind(item.device_id.to_string())
                .bind(item.document_id.to_string())
                .bind(item.kind.as_str())
                .bind(item.vault_id.to_string())
                .bind(ts(item.created_at))
                .bind(item.device_id.to_string())
                .execute(&mut *tx)
                .await
                .map_err(err)?;
            }
        }
        tx.commit().await.map_err(err)?;
        Ok(first)
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        quiet: Duration,
        ceiling: Duration,
        lease: Duration,
        limit: u32,
    ) -> Result<Vec<Uuid>> {
        // SQLite has one writer, so this UPDATE claims atomically.
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE push_devices SET claimed_until = ?1
             WHERE id IN (
                SELECT d.id FROM push_devices d JOIN push_pending p ON p.device_id = d.id
                WHERE (d.claimed_until IS NULL OR d.claimed_until <= ?2)
                  AND (d.backoff_until IS NULL OR d.backoff_until <= ?2)
                GROUP BY d.id
                HAVING MAX(p.created_at) <= ?3 OR MIN(p.created_at) <= ?4
                ORDER BY MIN(p.created_at)
                LIMIT ?5)
             RETURNING id",
        )
        .bind(ts(now + lease))
        .bind(ts(now))
        .bind(ts(now - quiet))
        .bind(ts(now - ceiling))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        ids.iter().map(|id| id.parse().map_err(err)).collect()
    }

    async fn claim_widget_due(
        &self,
        now: DateTime<Utc>,
        min_interval: Duration,
        lease: Duration,
        limit: u32,
    ) -> Result<Vec<Uuid>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE push_devices SET claimed_until = ?1
             WHERE id IN (
                SELECT id FROM push_devices
                WHERE widget_token IS NOT NULL AND widgets_dirty_at IS NOT NULL
                  AND (widgets_sent_at IS NULL OR widgets_sent_at <= ?3)
                  AND (claimed_until IS NULL OR claimed_until <= ?2)
                ORDER BY widgets_dirty_at
                LIMIT ?4)
             RETURNING id",
        )
        .bind(ts(now + lease))
        .bind(ts(now))
        .bind(ts(now - min_interval))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        ids.iter().map(|id| id.parse().map_err(err)).collect()
    }

    async fn pending(&self, device: Uuid) -> Result<Vec<PendingNotification>> {
        sqlx::query(
            "SELECT device_id, document_id, kind, vault_id, created_at FROM push_pending
             WHERE device_id = ? ORDER BY created_at, document_id",
        )
        .bind(device.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(err)?
        .iter()
        .map(pending)
        .collect()
    }

    async fn remove_pending(&self, device: Uuid, items: &[(Id, NotificationKind)]) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
        for (document, kind) in items {
            sqlx::query(
                "DELETE FROM push_pending WHERE device_id = ? AND document_id = ? AND kind = ?",
            )
            .bind(device.to_string())
            .bind(document.to_string())
            .bind(kind.as_str())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }
        tx.commit().await.map_err(err)
    }

    async fn state(&self, device: Uuid) -> Result<PushDeviceState> {
        let row = sqlx::query(
            "SELECT window_started_at, alerts_in_window, backoff_until, failures,
                widgets_dirty_at, widgets_sent_at, test_sent_at
             FROM push_devices WHERE id = ?",
        )
        .bind(device.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(err)?;
        let Some(row) = row else {
            return Ok(PushDeviceState::default());
        };
        Ok(PushDeviceState {
            window_started_at: parse_ts(row.get("window_started_at"))?,
            alerts_in_window: row.get::<i64, _>("alerts_in_window") as u32,
            backoff_until: parse_ts(row.get("backoff_until"))?,
            failures: row.get::<i64, _>("failures") as u32,
            widgets_dirty_at: parse_ts(row.get("widgets_dirty_at"))?,
            widgets_sent_at: parse_ts(row.get("widgets_sent_at"))?,
            test_sent_at: parse_ts(row.get("test_sent_at"))?,
        })
    }

    async fn release(&self, device: Uuid, state: &PushDeviceState) -> Result<()> {
        // Widgets dirtied after the state was read stay dirty.
        sqlx::query(
            "UPDATE push_devices SET claimed_until = NULL, window_started_at = ?,
                alerts_in_window = ?, backoff_until = ?, failures = ?, widgets_sent_at = ?,
                widgets_dirty_at = CASE
                    WHEN ? IS NULL AND widgets_dirty_at > COALESCE(?, '') THEN widgets_dirty_at
                    ELSE ? END
             WHERE id = ?",
        )
        .bind(state.window_started_at.map(ts))
        .bind(i64::from(state.alerts_in_window))
        .bind(state.backoff_until.map(ts))
        .bind(i64::from(state.failures))
        .bind(state.widgets_sent_at.map(ts))
        .bind(state.widgets_dirty_at.map(ts))
        .bind(state.widgets_sent_at.map(ts))
        .bind(state.widgets_dirty_at.map(ts))
        .bind(device.to_string())
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn mark_delivered(&self, device: Uuid, at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE push_devices SET last_delivered_at = ? WHERE id = ?")
            .bind(ts(at))
            .bind(device.to_string())
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn mark_widgets_dirty(&self, devices: &[Uuid], at: DateTime<Utc>) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(err)?;
        for device in devices {
            sqlx::query("UPDATE push_devices SET widgets_dirty_at = ? WHERE id = ?")
                .bind(ts(at))
                .bind(device.to_string())
                .execute(&mut *tx)
                .await
                .map_err(err)?;
        }
        tx.commit().await.map_err(err)
    }

    async fn try_record_test(
        &self,
        device: Uuid,
        at: DateTime<Utc>,
        not_since: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE push_devices SET test_sent_at = ?
             WHERE id = ? AND (test_sent_at IS NULL OR test_sent_at <= ?)",
        )
        .bind(ts(at))
        .bind(device.to_string())
        .bind(ts(not_since))
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn prune_pending(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM push_pending WHERE created_at < ?")
            .bind(ts(before))
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(result.rows_affected())
    }

    async fn count_devices(&self) -> Result<u64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM push_devices")
            .fetch_one(&self.pool)
            .await
            .map_err(err)?;
        Ok(count as u64)
    }
}

#[async_trait]
impl GrantLiveness for SqlitePushRepository {
    async fn is_live(&self, grant_id: &str) -> Result<bool> {
        if grant_id.is_empty() {
            return Ok(false);
        }
        let now = folio_auth::server::now();
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM oauth_access_tokens WHERE grant_id = ?1 AND expires_at > ?2)
                 OR EXISTS (SELECT 1 FROM oauth_refresh_tokens
                            WHERE grant_id = ?1 AND used = 0 AND expires_at > ?2)",
        )
        .bind(grant_id)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(err)?;
        Ok(live)
    }
}

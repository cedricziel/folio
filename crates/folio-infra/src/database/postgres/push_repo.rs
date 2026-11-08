//! PostgreSQL storage for push devices and pending notifications
//!
//! Devices are claimed with `FOR UPDATE SKIP LOCKED` and a lease, so several
//! Folio instances never send the same batch twice.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::models::{
    DeviceRegistration, DocumentOutcome, Id, NotificationKind, PendingNotification, PushDevice,
    PushDeviceState, PushToken, PushTokenKind, UserId, VaultId,
};
use folio_core::ports::{GrantLiveness, PushDeviceRepository};

fn err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

const DEVICE_COLS: &str = "id, user_id, installation_id, client_id, grant_id, platform, \
    environment, locale, preferences, alert_token, alert_token_registered_at, widget_token, \
    widget_token_registered_at, created_at, updated_at, last_delivered_at";

fn token(value: Option<String>, at: Option<DateTime<Utc>>) -> Result<Option<PushToken>> {
    Ok(match (value, at) {
        (Some(value), Some(registered_at)) => Some(PushToken {
            value,
            registered_at,
        }),
        _ => None,
    })
}

fn device(row: &PgRow) -> Result<PushDevice> {
    Ok(PushDevice {
        id: row.get("id"),
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
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        last_delivered_at: row.get("last_delivered_at"),
    })
}

fn pending(row: &PgRow) -> Result<PendingNotification> {
    Ok(PendingNotification {
        device_id: row.get("device_id"),
        document_id: row.get("document_id"),
        kind: row.get::<String, _>("kind").parse().map_err(err)?,
        vault_id: VaultId(row.get("vault_id")),
        created_at: row.get("created_at"),
    })
}

pub struct PostgresPushRepository {
    pool: PgPool,
}

impl PostgresPushRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn many(&self, sql: &str, user: Option<&str>) -> Result<Vec<PushDevice>> {
        let sql = format!("SELECT {DEVICE_COLS} FROM push_devices {sql} ORDER BY created_at, id");
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        if let Some(user) = user {
            query = query.bind(user);
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
impl PushDeviceRepository for PostgresPushRepository {
    async fn upsert(&self, r: DeviceRegistration) -> Result<(PushDevice, bool)> {
        let now = Utc::now();
        let preferences = serde_json::to_string(&r.preferences).map_err(err)?;
        let mut tx = self.pool.begin().await.map_err(err)?;
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM push_devices WHERE user_id = $1 AND installation_id = $2 FOR UPDATE",
        )
        .bind(r.user_id.as_str())
        .bind(&r.installation_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(err)?;
        let id = existing.unwrap_or_else(Uuid::new_v4);

        // A token belongs to one installation of the server; take it from any other row,
        // and delete only those rows that are left without a token.
        let mut emptied: Vec<Uuid> = Vec::new();
        for (column, value) in [
            ("alert_token", &r.alert_token),
            ("widget_token", &r.widget_token),
        ] {
            if let Some(value) = value {
                let ids: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "UPDATE push_devices SET {column} = NULL, {column}_registered_at = NULL \
                     WHERE {column} = $1 AND id <> $2 RETURNING id"
                )))
                .bind(value)
                .bind(id)
                .fetch_all(&mut *tx)
                .await
                .map_err(err)?;
                emptied.extend(ids);
            }
        }
        sqlx::query(
            "DELETE FROM push_devices WHERE id = ANY($1) AND alert_token IS NULL AND widget_token IS NULL",
        )
        .bind(&emptied)
        .execute(&mut *tx)
        .await
        .map_err(err)?;

        let stored = sqlx::query(
            "INSERT INTO push_devices (id, user_id, installation_id, client_id, grant_id,
                platform, environment, locale, preferences, alert_token, alert_token_registered_at,
                widget_token, widget_token_registered_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                CASE WHEN $10::text IS NULL THEN NULL ELSE $12 END, $11,
                CASE WHEN $11::text IS NULL THEN NULL ELSE $12 END, $12, $12)
             ON CONFLICT (user_id, installation_id) DO UPDATE SET
                client_id = EXCLUDED.client_id, grant_id = EXCLUDED.grant_id,
                platform = EXCLUDED.platform, environment = EXCLUDED.environment,
                locale = EXCLUDED.locale, preferences = EXCLUDED.preferences,
                updated_at = EXCLUDED.updated_at,
                alert_token = COALESCE(EXCLUDED.alert_token, push_devices.alert_token),
                alert_token_registered_at = COALESCE(EXCLUDED.alert_token_registered_at,
                    push_devices.alert_token_registered_at),
                widget_token = COALESCE(EXCLUDED.widget_token, push_devices.widget_token),
                widget_token_registered_at = COALESCE(EXCLUDED.widget_token_registered_at,
                    push_devices.widget_token_registered_at)
             RETURNING id, (xmax = 0) AS inserted",
        )
        .bind(id)
        .bind(r.user_id.as_str())
        .bind(&r.installation_id)
        .bind(&r.client_id)
        .bind(&r.grant_id)
        .bind(r.platform.as_str())
        .bind(r.environment.as_str())
        .bind(&r.locale)
        .bind(&preferences)
        .bind(&r.alert_token)
        .bind(&r.widget_token)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
        tx.commit().await.map_err(err)?;
        // A concurrent first registration may have won the insert; report its row.
        let (id, inserted): (Uuid, bool) = (stored.get("id"), stored.get("inserted"));
        let device = self
            .get(id)
            .await?
            .ok_or_else(|| err("registered device vanished"))?;
        Ok((device, inserted))
    }

    async fn get(&self, id: Uuid) -> Result<Option<PushDevice>> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLS} FROM push_devices WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(err)?
        .as_ref()
        .map(device)
        .transpose()
    }

    async fn find(&self, user: &UserId, installation_id: &str) -> Result<Option<PushDevice>> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLS} FROM push_devices WHERE user_id = $1 AND installation_id = $2"
        )))
        .bind(user.as_str())
        .bind(installation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(err)?
        .as_ref()
        .map(device)
        .transpose()
    }

    async fn list_for_user(&self, user: &UserId) -> Result<Vec<PushDevice>> {
        self.many("WHERE user_id = $1", Some(user.as_str())).await
    }

    async fn list_all(&self) -> Result<Vec<PushDevice>> {
        self.many("", None).await
    }

    async fn delete(&self, user: &UserId, installation_id: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM push_devices WHERE user_id = $1 AND installation_id = $2")
                .bind(user.as_str())
                .bind(installation_id)
                .execute(&self.pool)
                .await
                .map_err(err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_by_id(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM push_devices WHERE id = $1")
            .bind(id)
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
        let mut tx = self.pool.begin().await.map_err(err)?;
        let cleared = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE push_devices SET {column} = NULL, {column}_registered_at = NULL
             WHERE id = $1 AND {column} IS NOT NULL
               AND ($2::timestamptz IS NULL OR {column}_registered_at < $2)"
        )))
        .bind(device)
        .bind(inactive_since)
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected()
            > 0;
        sqlx::query(
            "DELETE FROM push_devices WHERE id = $1 AND alert_token IS NULL AND widget_token IS NULL",
        )
        .bind(device)
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
        let mut tx = self.pool.begin().await.map_err(err)?;
        let first = sqlx::query(
            "INSERT INTO push_document_outcomes (document_id, outcome, created_at)
             SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM documents WHERE id = $1)
             ON CONFLICT DO NOTHING",
        )
        .bind(document)
        .bind(outcome.as_str())
        .bind(Utc::now())
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected()
            > 0;
        if first {
            for item in items {
                sqlx::query(
                    "INSERT INTO push_pending (device_id, document_id, kind, vault_id, created_at)
                     SELECT $1, $2, $3, $4, $5 WHERE EXISTS (SELECT 1 FROM push_devices WHERE id = $1)
                     ON CONFLICT DO NOTHING",
                )
                .bind(item.device_id)
                .bind(item.document_id)
                .bind(item.kind.as_str())
                .bind(item.vault_id.0)
                .bind(item.created_at)
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
        holder: Uuid,
    ) -> Result<Vec<Uuid>> {
        sqlx::query_scalar(
            "UPDATE push_devices SET claimed_until = $1, claim_holder = $6
             WHERE id IN (
                SELECT d.id FROM push_devices d
                WHERE d.id IN (
                    SELECT device_id FROM push_pending GROUP BY device_id
                    HAVING MAX(created_at) <= $3 OR MIN(created_at) <= $4)
                  AND (d.claimed_until IS NULL OR d.claimed_until <= $2)
                  AND (d.backoff_until IS NULL OR d.backoff_until <= $2)
                ORDER BY d.id
                LIMIT $5
                FOR UPDATE SKIP LOCKED)
             RETURNING id",
        )
        .bind(now + lease)
        .bind(now)
        .bind(now - quiet)
        .bind(now - ceiling)
        .bind(i64::from(limit))
        .bind(holder)
        .fetch_all(&self.pool)
        .await
        .map_err(err)
    }

    async fn claim_widget_due(
        &self,
        now: DateTime<Utc>,
        min_interval: Duration,
        lease: Duration,
        limit: u32,
        holder: Uuid,
    ) -> Result<Vec<Uuid>> {
        sqlx::query_scalar(
            "UPDATE push_devices SET claimed_until = $1, claim_holder = $5
             WHERE id IN (
                SELECT id FROM push_devices
                WHERE widget_token IS NOT NULL AND widgets_dirty_at IS NOT NULL
                  AND (widgets_sent_at IS NULL OR widgets_sent_at <= $3)
                  AND (claimed_until IS NULL OR claimed_until <= $2)
                ORDER BY widgets_dirty_at
                LIMIT $4
                FOR UPDATE SKIP LOCKED)
             RETURNING id",
        )
        .bind(now + lease)
        .bind(now)
        .bind(now - min_interval)
        .bind(i64::from(limit))
        .bind(holder)
        .fetch_all(&self.pool)
        .await
        .map_err(err)
    }

    async fn pending(&self, device: Uuid) -> Result<Vec<PendingNotification>> {
        sqlx::query(
            "SELECT device_id, document_id, kind, vault_id, created_at FROM push_pending
             WHERE device_id = $1 ORDER BY created_at, document_id",
        )
        .bind(device)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?
        .iter()
        .map(pending)
        .collect()
    }

    async fn remove_pending(&self, device: Uuid, items: &[(Id, NotificationKind)]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        for (document, kind) in items {
            sqlx::query(
                "DELETE FROM push_pending WHERE device_id = $1 AND document_id = $2 AND kind = $3",
            )
            .bind(device)
            .bind(document)
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
                widgets_dirty_at, widgets_sent_at, test_sent_at, batch_id
             FROM push_devices WHERE id = $1",
        )
        .bind(device)
        .fetch_optional(&self.pool)
        .await
        .map_err(err)?;
        let Some(row) = row else {
            return Ok(PushDeviceState::default());
        };
        Ok(PushDeviceState {
            window_started_at: row.get("window_started_at"),
            alerts_in_window: row.get::<i32, _>("alerts_in_window") as u32,
            backoff_until: row.get("backoff_until"),
            failures: row.get::<i32, _>("failures") as u32,
            widgets_dirty_at: row.get("widgets_dirty_at"),
            widgets_sent_at: row.get("widgets_sent_at"),
            test_sent_at: row.get("test_sent_at"),
            batch_id: row.get("batch_id"),
        })
    }

    async fn renew(&self, device: Uuid, holder: Uuid, until: DateTime<Utc>) -> Result<bool> {
        let renewed = sqlx::query(
            "UPDATE push_devices SET claimed_until = $1 WHERE id = $2 AND claim_holder = $3",
        )
        .bind(until)
        .bind(device)
        .bind(holder)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(renewed.rows_affected() == 1)
    }

    async fn release(&self, device: Uuid, holder: Uuid, state: &PushDeviceState) -> Result<bool> {
        // Widgets dirtied after the state was read stay dirty.
        let released = sqlx::query(
            "UPDATE push_devices SET claimed_until = NULL, claim_holder = NULL, batch_id = $8,
                window_started_at = $1,
                alerts_in_window = $2, backoff_until = $3, failures = $4, widgets_sent_at = $5,
                widgets_dirty_at = CASE
                    WHEN $6::timestamptz IS NULL
                     AND widgets_dirty_at > COALESCE($5::timestamptz, '-infinity') THEN widgets_dirty_at
                    ELSE $6 END
             WHERE id = $7 AND claim_holder = $9",
        )
        .bind(state.window_started_at)
        .bind(state.alerts_in_window as i32)
        .bind(state.backoff_until)
        .bind(state.failures as i32)
        .bind(state.widgets_sent_at)
        .bind(state.widgets_dirty_at)
        .bind(device)
        .bind(state.batch_id)
        .bind(holder)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(released.rows_affected() == 1)
    }

    async fn mark_delivered(&self, device: Uuid, at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE push_devices SET last_delivered_at = $1 WHERE id = $2")
            .bind(at)
            .bind(device)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn mark_widgets_dirty(&self, devices: &[Uuid], at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE push_devices SET widgets_dirty_at = $1 WHERE id = ANY($2)")
            .bind(at)
            .bind(devices)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn try_record_test(
        &self,
        device: Uuid,
        at: DateTime<Utc>,
        not_since: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE push_devices SET test_sent_at = $1
             WHERE id = $2 AND (test_sent_at IS NULL OR test_sent_at <= $3)",
        )
        .bind(at)
        .bind(device)
        .bind(not_since)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn prune_pending(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM push_pending WHERE created_at < $1")
            .bind(before)
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
impl GrantLiveness for PostgresPushRepository {
    async fn is_live(&self, grant_id: &str) -> Result<bool> {
        if grant_id.is_empty() {
            return Ok(false);
        }
        let now = folio_auth::server::now();
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM oauth_access_tokens WHERE grant_id = $1 AND expires_at > $2)
                 OR EXISTS (SELECT 1 FROM oauth_refresh_tokens
                            WHERE grant_id = $1 AND used = 0 AND expires_at > $2)",
        )
        .bind(grant_id)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(err)
    }
}

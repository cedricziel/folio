use chrono::{DateTime, Utc};
use folio_core::{models::ProcessingStageSummary, Error, Result};
use serde::de::DeserializeOwned;

#[derive(sqlx::FromRow)]
pub(super) struct ProcessingSummaryRow {
    pub run_id: String,
    pub revision_id: Option<String>,
    pub stage: String,
    pub attempt: i64,
    pub state: String,
    pub reason_code: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
    pub has_text: Option<bool>,
}

fn invalid_summary() -> Error {
    Error::Repository("Invalid persisted processing stage summary".into())
}

fn decode<T: DeserializeOwned>(value: String) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(value)).map_err(|_| invalid_summary())
}

impl TryFrom<ProcessingSummaryRow> for ProcessingStageSummary {
    type Error = Error;

    fn try_from(row: ProcessingSummaryRow) -> Result<Self> {
        Ok(Self {
            run_id: row.run_id.parse().map_err(|_| invalid_summary())?,
            revision_id: row
                .revision_id
                .map(|id| id.parse().map_err(|_| invalid_summary()))
                .transpose()?,
            stage: decode(row.stage)?,
            attempt: row.attempt.try_into().map_err(|_| invalid_summary())?,
            state: decode(row.state)?,
            reason_code: row.reason_code.map(decode).transpose()?,
            retry_at: row.retry_at,
            has_text: row.has_text,
        })
    }
}

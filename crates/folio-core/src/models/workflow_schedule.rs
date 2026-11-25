use chrono::{DateTime, Datelike, Days, Duration, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

use super::{
    DocumentDate, WebhookDocument, WebhookPayload, Workflow, WorkflowRun, WorkflowTrigger,
};
use crate::EventSnapshot;
use crate::{Error, Result};

const CATCH_UP_LIMIT: i64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowCadence {
    Once {
        /// RFC 3339 UTC instant, with up to nine fractional digits.
        #[cfg_attr(feature = "openapi", schema(value_type = String, example = "2026-12-01T09:00:00Z"))]
        at: DateTime<Utc>,
    },
    Recurring {
        /// RFC 3339 UTC instant with up to nine fractional digits. Leap seconds are unsupported for recurring clocks.
        #[cfg_attr(feature = "openapi", schema(value_type = String, example = "2026-12-01T09:00:00Z"))]
        first_at: DateTime<Utc>,
        #[cfg_attr(feature = "openapi", schema(value_type = i64, minimum = 1, maximum = 4294967295_i64))]
        interval_seconds: u32,
    },
    DocumentDateOffset {
        days: i32,
        #[cfg_attr(feature = "openapi", schema(value_type = String, example = "09:00:00"))]
        time_utc: NaiveTime,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct DueWorkflowOccurrences {
    pub due: Vec<DateTime<Utc>>,
    pub next_at: Option<DateTime<Utc>>,
}

impl WorkflowCadence {
    pub fn first_cursor(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Once { at } => Some(at),
            Self::Recurring { first_at, .. } => Some(first_at),
            Self::DocumentDateOffset { time_utc, .. } => {
                let today = now.date_naive().and_time(time_utc).and_utc();
                if today >= now {
                    Some(today)
                } else {
                    today
                        .checked_add_days(Days::new(1))
                        .filter(|next| next.year() <= 9999)
                }
            }
        }
    }

    pub fn document_date_at(self, occurrence: DateTime<Utc>) -> Option<DocumentDate> {
        let Self::DocumentDateOffset { days, time_utc } = self else {
            return None;
        };
        if occurrence.time() != time_utc {
            return None;
        }
        DocumentDate::from_naive_date(
            occurrence
                .date_naive()
                .checked_sub_signed(Duration::days(i64::from(days)))?,
        )
    }

    pub fn validate(self) -> Result<()> {
        let at = match self {
            Self::Once { at } => at,
            Self::Recurring {
                first_at,
                interval_seconds,
            } => {
                if interval_seconds == 0 {
                    return Err(Error::Validation(
                        "A recurring interval must be positive".into(),
                    ));
                }
                first_at
            }
            Self::DocumentDateOffset { days, time_utc } => {
                if days.unsigned_abs() >= 3_652_059 || time_utc.nanosecond() >= 1_000_000_000 {
                    return Err(Error::Validation(
                        "The document date offset is out of range".into(),
                    ));
                }
                return Ok(());
            }
        };
        if !(1..=9999).contains(&at.year()) {
            return Err(Error::Validation(
                "The schedule start is out of range".into(),
            ));
        }
        if matches!(self, Self::Recurring { first_at, .. } if first_at.timestamp_subsec_nanos() >= 1_000_000_000)
        {
            return Err(Error::Validation(
                "Recurring clocks cannot start during a leap second".into(),
            ));
        }
        Ok(())
    }

    pub fn claim_window(
        self,
        cursor: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<DueWorkflowOccurrences> {
        self.validate()?;
        let Some(cursor) = cursor else {
            return Ok(DueWorkflowOccurrences {
                due: Vec::new(),
                next_at: None,
            });
        };
        if let Self::DocumentDateOffset { time_utc, .. } = self {
            if cursor.time() != time_utc {
                return Err(Error::Validation(
                    "The date offset cursor is not on its UTC time".into(),
                ));
            }
            return Self::Recurring {
                first_at: cursor,
                interval_seconds: 86_400,
            }
            .claim_window(Some(cursor), now);
        }
        match self {
            Self::Once { at } if cursor != at => {
                return Err(Error::Validation(
                    "The schedule cursor is not its occurrence".into(),
                ));
            }
            Self::Recurring {
                first_at,
                interval_seconds,
            } => {
                let elapsed = cursor.signed_duration_since(first_at);
                let step = i64::from(interval_seconds);
                if elapsed < Duration::zero()
                    || elapsed.subsec_nanos() != 0
                    || elapsed.num_seconds() % step != 0
                {
                    return Err(Error::Validation(
                        "The schedule cursor is not on its interval".into(),
                    ));
                }
            }
            _ => {}
        }
        if cursor > now {
            return Ok(DueWorkflowOccurrences {
                due: Vec::new(),
                next_at: Some(cursor),
            });
        }
        let Self::Recurring {
            interval_seconds, ..
        } = self
        else {
            return Ok(DueWorkflowOccurrences {
                due: vec![cursor],
                next_at: None,
            });
        };
        let step = i64::from(interval_seconds);
        let periods = now.signed_duration_since(cursor).num_seconds() / step;
        let addition = |offset| cursor.checked_add_signed(Duration::seconds(offset * step));
        let start = (periods + 1 - CATCH_UP_LIMIT).max(0);
        let due = (start..=periods)
            .map(|offset| addition(offset).expect("due occurrence is no later than now"))
            .collect();
        Ok(DueWorkflowOccurrences {
            due,
            next_at: addition(periods + 1).filter(|next| next.year() <= 9999),
        })
    }
}

impl WorkflowRun {
    pub fn for_document_date(
        workflow: &Workflow,
        occurrence_at: DateTime<Utc>,
        date: DocumentDate,
        snapshot: &EventSnapshot,
        public_url: &str,
    ) -> Result<Self> {
        let mut document = WebhookDocument::from_snapshot(snapshot, None, public_url);
        document.document_date = Some(date);
        let payload = serde_json::to_string(&WebhookPayload::new(
            WorkflowTrigger::DocumentDateOffset,
            occurrence_at,
            document,
        ))
        .map_err(|error| Error::Repository(format!("Invalid workflow payload: {error}")))?;
        Ok(Self::new(
            workflow.vault_id,
            workflow.id,
            WorkflowTrigger::DocumentDateOffset,
            Some(snapshot.document_id),
            payload,
        )
        .with_definition(workflow))
    }

    pub fn for_clock(workflow: &Workflow, occurrence_at: DateTime<Utc>) -> Result<Self> {
        let payload = serde_json::to_string(&WebhookPayload::for_clock(occurrence_at))
            .map_err(|error| Error::Repository(format!("Invalid workflow payload: {error}")))?;
        Ok(Self::new(
            workflow.vault_id,
            workflow.id,
            WorkflowTrigger::Scheduled,
            None,
            payload,
        )
        .with_definition(workflow))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_one_off_schedule_cannot_fire_again_after_its_cursor_is_consumed() {
        let at = utc("2026-11-22T14:00:00Z");
        let cadence = WorkflowCadence::Once { at };
        let before = cadence
            .claim_window(cadence.first_cursor(at), at - Duration::seconds(1))
            .unwrap();
        assert!(before.due.is_empty());
        let due = cadence.claim_window(before.next_at, at).unwrap();
        assert_eq!(due.due, vec![at]);
        assert!(cadence
            .claim_window(due.next_at, at)
            .unwrap()
            .due
            .is_empty());
    }

    #[test]
    fn leap_seconds_are_allowed_for_one_time_clocks_only() {
        let at = utc("2016-12-31T23:59:60Z");
        let once = WorkflowCadence::Once { at };
        assert_eq!(once.claim_window(Some(at), at).unwrap().due, [at]);
        let recurring = WorkflowCadence::Recurring {
            first_at: at,
            interval_seconds: 3600,
        };
        assert!(recurring.validate().is_err());
    }

    #[test]
    fn document_date_offsets_use_the_utc_calendar_and_keep_the_recent_window() {
        let cadence = WorkflowCadence::DocumentDateOffset {
            days: -1,
            time_utc: "09:30:00".parse().unwrap(),
        };
        let now = utc("2028-02-27T09:30:01Z");
        let cursor = cadence.first_cursor(now);
        assert_eq!(cursor, Some(utc("2028-02-28T09:30:00Z")));
        let day = cadence.document_date_at(cursor.unwrap()).unwrap();
        assert_eq!(day.to_string(), "2028-02-29");
        assert!(cadence.document_date_at(now).is_none());
        let catch_up = cadence
            .claim_window(cursor, utc("2028-04-01T11:00:00Z"))
            .unwrap();
        assert_eq!(catch_up.due.len(), 16);
        assert_eq!(catch_up.due.last(), Some(&utc("2028-04-01T09:30:00Z")));
        assert_eq!(catch_up.next_at, Some(utc("2028-04-02T09:30:00Z")));
    }

    #[test]
    fn recurring_catch_up_keeps_only_the_latest_occurrences_and_does_not_drift() {
        let first_at = utc("2026-11-01T00:00:00Z");
        let now = utc("2026-11-22T14:23:00Z");
        let cadence = WorkflowCadence::Recurring {
            first_at,
            interval_seconds: 3600,
        };
        let due = cadence.claim_window(Some(first_at), now).unwrap();
        assert_eq!(due.due.len(), 16);
        assert_eq!(due.due[0], utc("2026-11-21T23:00:00Z"));
        assert_eq!(due.due[15], utc("2026-11-22T14:00:00Z"));
        assert_eq!(due.next_at, Some(utc("2026-11-22T15:00:00Z")));
        assert!(cadence
            .claim_window(due.next_at, now)
            .unwrap()
            .due
            .is_empty());
    }
}

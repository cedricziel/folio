use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::{WebhookPayload, Workflow, WorkflowRun, WorkflowTrigger};
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
        /// RFC 3339 UTC instant, with up to nine fractional digits.
        #[cfg_attr(feature = "openapi", schema(value_type = String, example = "2026-12-01T09:00:00Z"))]
        first_at: DateTime<Utc>,
        #[cfg_attr(feature = "openapi", schema(value_type = i64, minimum = 1, maximum = 4294967295_i64))]
        interval_seconds: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct DueWorkflowOccurrences {
    pub due: Vec<DateTime<Utc>>,
    pub next_at: Option<DateTime<Utc>>,
}

impl WorkflowCadence {
    pub fn first_at(self) -> DateTime<Utc> {
        match self {
            Self::Once { at } => at,
            Self::Recurring { first_at, .. } => first_at,
        }
    }

    pub fn validate(self) -> Result<()> {
        if !(1..=9999).contains(&self.first_at().year()) {
            return Err(Error::Validation(
                "The schedule start is out of range".into(),
            ));
        }
        if matches!(
            self,
            Self::Recurring {
                interval_seconds: 0,
                ..
            }
        ) {
            return Err(Error::Validation(
                "A recurring interval must be positive".into(),
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
            .claim_window(Some(cadence.first_at()), at - Duration::seconds(1))
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

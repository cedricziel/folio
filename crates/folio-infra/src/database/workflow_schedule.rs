use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::{
    models::{Workflow, WorkflowCadence, WorkflowInput},
    Error, Result,
};

pub(crate) fn clock_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

pub(crate) fn clock_cursor(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(str::parse)
        .transpose()
        .map_err(|error| Error::Repository(format!("Invalid workflow clock cursor: {error}")))
}

pub(crate) fn encode_cadence(cadence: Option<WorkflowCadence>) -> Result<Option<String>> {
    cadence
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|error| Error::Repository(format!("Invalid workflow cadence: {error}")))
}

pub(crate) fn decode_cadence(value: Option<&str>) -> Result<Option<WorkflowCadence>> {
    value
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| Error::Repository(format!("Invalid workflow cadence: {error}")))
}

pub(crate) fn edited_clock_cursor(
    workflow: &Workflow,
    input: &WorkflowInput,
    current: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    if workflow.cadence != input.cadence {
        return Ok(input.cadence.map(|cadence| clock_time(cadence.first_at())));
    }
    if !workflow.enabled && input.enabled {
        return resumed_clock_cursor(workflow, current, now);
    }
    Ok(current.map(str::to_owned))
}

pub(crate) fn resumed_clock_cursor(
    workflow: &Workflow,
    current: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let Some(cadence) = workflow.cadence else {
        return Ok(None);
    };
    let cursor = clock_cursor(current)?;
    if cursor.is_some_and(|value| value <= now) {
        let window = cadence.claim_window(cursor, now)?;
        Ok(window.next_at.map(clock_time))
    } else {
        Ok(current.map(str::to_owned))
    }
}

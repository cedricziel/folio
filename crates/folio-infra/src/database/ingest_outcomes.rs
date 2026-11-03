use folio_core::error::{Error, Result};
use folio_core::models::{IngestResult, SourceKind};

pub(super) fn validate_observation(kind: SourceKind, fingerprint: &str) -> Result<()> {
    if !matches!(kind, SourceKind::Folder | SourceKind::Email)
        || fingerprint.len() != 64
        || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::Validation("Invalid ingestion observation".into()));
    }
    Ok(())
}

pub(super) fn completed_observation(
    state: &str,
    previous: &str,
    current: &str,
) -> Result<Option<IngestResult>> {
    match state {
        "processing" | "failed" => Ok(None),
        "skipped" if previous != current => Ok(None),
        _ => parse_result(state).map(Some),
    }
}

pub(super) fn parse_result(value: &str) -> Result<IngestResult> {
    match value {
        "imported" => Ok(IngestResult::Imported),
        "skipped" => Ok(IngestResult::Skipped),
        "duplicate" => Ok(IngestResult::Duplicate),
        "failed" => Ok(IngestResult::Failed),
        _ => Err(Error::Repository("Invalid stored ingestion outcome".into())),
    }
}

pub(super) fn validate_page(before: Option<i64>, limit: u32) -> Result<()> {
    if before.is_some_and(|id| id <= 0) || !(1..=100).contains(&limit) {
        return Err(Error::Validation("Invalid outcome pagination".into()));
    }
    Ok(())
}

pub(super) fn validate_completion(
    attempt: &folio_core::models::IngestAttempt,
    outcome: IngestResult,
    document_id: Option<folio_core::models::Id>,
    claim: Option<&folio_core::models::LedgerEntry>,
) -> Result<()> {
    if (outcome == IngestResult::Imported) != document_id.is_some()
        || claim
            .is_some_and(|claim| claim.vault_id != attempt.vault_id || claim.outcome != "claimed")
        || (outcome == IngestResult::Imported
            && !claim.is_some_and(|claim| claim.document_id == document_id))
        || (claim.is_some() && !matches!(outcome, IngestResult::Imported | IngestResult::Failed))
    {
        return Err(Error::Validation("Invalid ingestion completion".into()));
    }
    Ok(())
}

pub(super) fn outcome_event(
    attempt: &folio_core::models::IngestAttempt,
    outcome: IngestResult,
) -> Result<folio_core::domain_event::DomainEventEnvelope> {
    use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
    let event = DomainEventEnvelope::new(outcome.event_type(), attempt.vault_id,
        DomainEventSubject { subject_type: "ingest".into(), id: attempt.item_id }, None, EventAudience::Owners,
        serde_json::json!({"payload_version": 1, "source_id": attempt.source_id, "source_kind": attempt.source_kind.as_str(),
            "item_id": attempt.item_id, "attempt_id": attempt.attempt_id, "outcome": outcome.as_str(),
            "reason_code": outcome.reason_code(), "retryable": outcome.retryable()}))?
        .with_mutation(attempt.attempt_id)?;
    if folio_core::mutation::current_lineage().is_some() {
        Ok(event)
    } else {
        event.with_correlation(attempt.item_id, None)
    }
}

pub(super) fn health_event(
    vault: folio_core::models::VaultId,
    source: folio_core::models::Id,
    kind: SourceKind,
    change: folio_core::models::SourceHealthChange,
) -> Result<folio_core::domain_event::DomainEventEnvelope> {
    use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
    let mut data = serde_json::json!({"payload_version": 1, "source_id": source, "source_kind": kind.as_str()});
    if change == folio_core::models::SourceHealthChange::Failed {
        data["reason_code"] = "source_unavailable".into();
    }
    let event = DomainEventEnvelope::new(
        change.event_type(),
        vault,
        DomainEventSubject {
            subject_type: "source".into(),
            id: source,
        },
        None,
        EventAudience::Owners,
        data,
    )?;
    if folio_core::mutation::current_lineage().is_some() {
        Ok(event)
    } else {
        event.with_correlation(source, None)
    }
}

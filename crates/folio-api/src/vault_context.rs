use chrono::{DateTime, Utc};
use folio_core::models::{Document, UserId, VaultAction, VaultId, VaultRole};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VaultSummary {
    #[schema(value_type = uuid::Uuid)]
    pub id: VaultId,
    pub name: String,
    #[schema(value_type = String, example = "owner")]
    pub role: VaultRole,
    pub created_at: DateTime<Utc>,
    /// Whether anyone besides the caller can reach the vault; absent where the server doesn't check.
    /// A group grant counts as shared, even when the caller is the group's only member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
}

#[derive(Clone)]
pub struct VaultContext {
    pub id: VaultId,
    pub role: VaultRole,
    pub vaults: Vec<VaultSummary>,
    pub user: Option<UserId>,
    /// Scopes of the bearer token, or `None` for a browser session or no sign-in
    pub token_scopes: Option<Vec<String>>,
    /// Path and query of the page being rendered, so links out of a list can lead back to it
    pub page: String,
}

tokio::task_local! {
    pub static CURRENT_VAULT: VaultContext;
}

pub fn is_scoped_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    path == "/"
        || [
            "/documents",
            "/trash",
            "/saved-views",
            "/api/documents",
            "/api/tags",
            "/api/correspondents",
            "/api/document_types",
            "/api/workflows",
            "/api/sources",
            "/manage",
            "/workflows",
            "/events",
            "/settings/sources",
            "/search",
        ]
        .iter()
        .any(|prefix| {
            path == *prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
}

pub fn url(path: &str) -> String {
    if is_scoped_path(path) {
        if let Ok(id) = CURRENT_VAULT.try_with(|ctx| ctx.id) {
            return format!("/vaults/{id}{path}");
        }
    }
    path.to_owned()
}

pub fn can_edit_document(document: &Document) -> bool {
    CURRENT_VAULT
        .try_with(|ctx| {
            ctx.user
                .as_ref()
                .map_or(ctx.role == VaultRole::Owner, |user| {
                    ctx.role.permits(
                        &VaultAction::EditDocument {
                            uploaded_by: document.uploaded_by.clone(),
                        },
                        user,
                    )
                })
        })
        .unwrap_or(false)
}

pub struct ProcessingSummary {
    pub title: &'static str,
    pub status: &'static str,
    pub detail: Option<&'static str>,
    pub attempt: u32,
    pub retry_at: Option<String>,
}

pub fn processing_summaries(
    document: &Document,
    stages: &[folio_core::models::ProcessingStageSummary],
) -> Vec<ProcessingSummary> {
    use folio_core::models::{ProcessingStatus, StageOutcome};
    use folio_core::processing_events::{ProcessingEventStage, ProcessingReason};

    let mut summaries = Vec::new();
    for (stage, title, running, legacy) in [
        (
            ProcessingEventStage::Ocr,
            "Text extraction",
            "Reading",
            Some(document.ocr_status),
        ),
        (
            ProcessingEventStage::Embedding,
            "Search indexing",
            "Indexing",
            Some(document.embed_status),
        ),
        (
            ProcessingEventStage::Thumbnail,
            "Page previews",
            "Generating",
            None,
        ),
    ] {
        if let Some(current) = stages.iter().find(|summary| summary.stage == stage) {
            let status = match current.state {
                StageOutcome::Pending => "Waiting",
                StageOutcome::Running => running,
                StageOutcome::WaitingToRetry => "Waiting to retry",
                StageOutcome::Completed
                    if stage == ProcessingEventStage::Ocr && current.has_text == Some(false) =>
                {
                    "Completed with no text"
                }
                StageOutcome::Completed => "Completed",
                StageOutcome::Failed => "Failed",
                StageOutcome::Skipped => "Skipped",
            };
            let detail = match current.reason_code {
                Some(ProcessingReason::ProviderDisabled) => Some("Disabled in settings."),
                Some(ProcessingReason::NoText) => Some("There is no extracted text to index."),
                Some(ProcessingReason::UnsupportedFormat) => {
                    Some("Page previews are unavailable for this file type.")
                }
                Some(ProcessingReason::ProviderFailed) => {
                    Some("The processing service could not complete this attempt.")
                }
                Some(ProcessingReason::InvalidResponse) => {
                    Some("The processing service returned an unusable result.")
                }
                Some(ProcessingReason::StorageFailed) => Some("A file could not be read or saved."),
                Some(ProcessingReason::LeaseExpired) => {
                    Some("The processing attempt expired before finishing.")
                }
                None if current.state == StageOutcome::Failed => {
                    Some("Processing did not finish. The original file is unchanged.")
                }
                None => None,
            };
            summaries.push(ProcessingSummary {
                title,
                status,
                detail,
                attempt: current.attempt,
                retry_at: current
                    .retry_at
                    .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string()),
            });
        } else if let Some(legacy) = legacy {
            let status = match legacy {
                ProcessingStatus::Pending => "Waiting",
                ProcessingStatus::Running => running,
                ProcessingStatus::Done
                    if stage == ProcessingEventStage::Ocr
                        && document.content.as_deref() == Some("") =>
                {
                    "Completed with no text"
                }
                ProcessingStatus::Done => "Completed",
                ProcessingStatus::Failed => "Last attempt failed",
                ProcessingStatus::Skipped => "Skipped",
            };
            let detail = if legacy == ProcessingStatus::Failed {
                Some(if stage == ProcessingEventStage::Ocr {
                    "Text extraction did not finish. The original file is unchanged."
                } else {
                    "Search indexing did not finish. The original file is unchanged."
                })
            } else {
                None
            };
            summaries.push(ProcessingSummary {
                title,
                status,
                detail,
                attempt: 0,
                retry_at: None,
            });
        }
    }
    summaries
}

pub mod filters {
    use super::*;

    #[askama::filter_fn]
    pub fn vault_url(
        value: impl std::fmt::Display,
        _: &dyn askama::Values,
    ) -> askama::Result<String> {
        Ok(url(&value.to_string()))
    }

    /// Link to a document that remembers the list it is opened from
    #[askama::filter_fn]
    pub fn document_href(
        id: impl std::fmt::Display,
        _: &dyn askama::Values,
    ) -> askama::Result<String> {
        let href = url(&format!("/documents/{id}"));
        let page = CURRENT_VAULT
            .try_with(|ctx| ctx.page.clone())
            .unwrap_or_default();
        Ok(match crate::back_link::resolve(Some(&page)) {
            Some(back) => back.carried_by(href),
            None => href,
        })
    }

    #[askama::filter_fn]
    pub fn can_upload(_: bool, _: &dyn askama::Values) -> askama::Result<bool> {
        Ok(CURRENT_VAULT
            .try_with(|ctx| ctx.role >= VaultRole::Contributor)
            .unwrap_or(false))
    }

    #[askama::filter_fn]
    pub fn can_manage(_: bool, _: &dyn askama::Values) -> askama::Result<bool> {
        Ok(CURRENT_VAULT
            .try_with(|ctx| ctx.role >= VaultRole::Editor)
            .unwrap_or(false))
    }

    #[askama::filter_fn]
    pub fn can_workflows(_: bool, _: &dyn askama::Values) -> askama::Result<bool> {
        Ok(CURRENT_VAULT
            .try_with(|ctx| ctx.role == VaultRole::Owner)
            .unwrap_or(false))
    }

    #[askama::filter_fn]
    pub fn can_edit_document(document: &Document, _: &dyn askama::Values) -> askama::Result<bool> {
        Ok(super::can_edit_document(document))
    }
}

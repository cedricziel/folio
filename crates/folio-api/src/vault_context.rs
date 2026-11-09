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

pub mod filters {
    use super::*;

    #[askama::filter_fn]
    pub fn processing_status(
        status: &folio_core::models::ProcessingStatus,
        _: &dyn askama::Values,
    ) -> askama::Result<&'static str> {
        Ok(processing_label(status))
    }

    fn processing_label(status: &folio_core::models::ProcessingStatus) -> &'static str {
        use folio_core::models::ProcessingStatus;
        match status {
            ProcessingStatus::Pending => "Waiting",
            ProcessingStatus::Running => "Running",
            ProcessingStatus::Done => "Completed",
            ProcessingStatus::Failed => "Last attempt failed",
            ProcessingStatus::Skipped => "Skipped",
        }
    }

    #[askama::filter_fn]
    pub fn ocr_processing_status(
        document: &Document,
        _: &dyn askama::Values,
    ) -> askama::Result<&'static str> {
        if document.ocr_status == folio_core::models::ProcessingStatus::Done
            && document.content.as_deref() == Some("")
        {
            return Ok("Completed with no text");
        }
        Ok(processing_label(&document.ocr_status))
    }

    #[askama::filter_fn]
    pub fn vault_url(
        value: impl std::fmt::Display,
        _: &dyn askama::Values,
    ) -> askama::Result<String> {
        Ok(url(&value.to_string()))
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

//! Vault-scoped archival copies of documents.

use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use folio_core::models::{ArchiveAttempt, ArchiveCompletion, ArchiveState, ArchiveTool};
use folio_infra::queue::ArchiveService;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, SemaphorePermit};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    extract::{ApiJson, ApiPath},
    openapi::{BadRequest, InternalError, NotFound},
    state::AppState,
    vault::{DefaultVault, Uploader},
};

use super::documents::{content_disposition, FileContent};

static DOWNLOAD_SLOTS: Semaphore = Semaphore::const_new(4);

const NO_STORE: [(header::HeaderName, &str); 1] = [(header::CACHE_CONTROL, "private, no-store")];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchiveStatus {
    NotCreated,
    Queued,
    Ready,
    Skipped,
    Failed,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ToolDetails {
    pub name: String,
    pub version: String,
}

impl From<ArchiveTool> for ToolDetails {
    fn from(tool: ArchiveTool) -> Self {
        Self {
            name: tool.name,
            version: tool.version,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ArchiveDetails {
    pub profile: String,
    pub converter: ToolDetails,
    pub validator: ToolDetails,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ArchiveStatusResponse {
    pub status: ArchiveStatus,
    pub attempt_id: Option<Uuid>,
    pub reason: Option<&'static str>,
    pub details: Option<ArchiveDetails>,
    #[serde(skip)]
    interrupted: bool,
}

impl ArchiveStatusResponse {
    pub fn label(&self) -> &'static str {
        match self.status {
            ArchiveStatus::NotCreated => "Not created",
            ArchiveStatus::Queued => "Queued",
            ArchiveStatus::Ready => "Ready",
            ArchiveStatus::Skipped => "Skipped",
            ArchiveStatus::Failed => "Failed",
        }
    }

    pub fn ready(&self) -> bool {
        self.status == ArchiveStatus::Ready
    }

    pub fn queued(&self) -> bool {
        self.status == ArchiveStatus::Queued
    }

    pub fn can_generate(&self) -> bool {
        self.status == ArchiveStatus::NotCreated
    }

    pub fn can_retry(&self) -> bool {
        self.status == ArchiveStatus::Failed || self.interrupted
    }
}

impl From<Option<ArchiveAttempt>> for ArchiveStatusResponse {
    fn from(attempt: Option<ArchiveAttempt>) -> Self {
        let mut response = Self {
            status: ArchiveStatus::NotCreated,
            attempt_id: None,
            reason: None,
            details: None,
            interrupted: false,
        };
        if let Some(attempt) = attempt {
            response.attempt_id = Some(attempt.id);
            response.status = match attempt.state {
                ArchiveState::Queued => ArchiveStatus::Queued,
                ArchiveState::Completed(ArchiveCompletion::Ready(archive)) => {
                    response.details = Some(ArchiveDetails {
                        profile: archive.profile,
                        converter: archive.converter.into(),
                        validator: archive.validator.into(),
                    });
                    ArchiveStatus::Ready
                }
                ArchiveState::Completed(ArchiveCompletion::Skipped(reason)) => {
                    response.interrupted = matches!(
                        reason.as_str(),
                        "Document moved" | "Document trashed" | "Vault deleted"
                    );
                    response.reason = Some(skip_message(&reason));
                    ArchiveStatus::Skipped
                }
                ArchiveState::Completed(ArchiveCompletion::Failed(reason)) => {
                    response.reason = Some(failure_message(&reason));
                    ArchiveStatus::Failed
                }
            };
        }
        response
    }
}

fn skip_message(reason: &str) -> &'static str {
    match reason {
        "Document moved" | "Document trashed" | "Vault deleted" => {
            "Generation was interrupted by a move or deletion. You can retry the archival copy."
        }
        "unsupported source media type" => "This file type cannot be converted to PDF/A.",
        "document exceeds the conversion limits" => "This file exceeds the conversion limits.",
        "source PDF could not be inspected safely" => "Folio could not inspect this PDF safely.",
        "source PDF is encrypted" => "This PDF is encrypted.",
        "source PDF contains a signature" => "This PDF contains a digital signature.",
        "source PDF contains an attachment" => "This PDF contains an embedded attachment.",
        _ => "Folio skipped conversion for this original.",
    }
}

fn failure_message(reason: &str) -> &'static str {
    match reason {
        "OcrMyPdf is unavailable" | "VeraPdf is unavailable" => {
            "A tool needed to create or validate the PDF/A is unavailable."
        }
        "PDF/A-2b validation rejected the converted copy"
        | "veraPDF returned an invalid or incomplete validation report" => {
            "The archival copy did not pass PDF/A validation."
        }
        _ => "Folio could not create the archival copy. You can still download the original.",
    }
}

pub(super) fn service(state: &AppState) -> ApiResult<&ArchiveService> {
    state.archive.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Archival generation is unavailable",
        )
    })
}

#[utoipa::path(
    get, path = "/api/documents/{id}/archive", operation_id = "getDocumentArchive",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses((status = 200, description = "Archival state of the current original", body = ArchiveStatusResponse),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Read permission required"),
        NotFound, InternalError)
)]
pub(crate) async fn status(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<impl IntoResponse> {
    state
        .document_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;
    let status = service(&state)?.status(vault, id).await?;
    Ok((NO_STORE, Json(ArchiveStatusResponse::from(status))))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct ArchiveRequest {
    /// Supply the ID of the current failed or skipped attempt to retry it.
    /// Omit this field to enqueue the first attempt or reuse an existing one.
    pub expected_attempt_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ArchiveAdmission {
    pub attempt_id: Uuid,
}

#[utoipa::path(
    post, path = "/api/documents/{id}/archive", operation_id = "requestDocumentArchive",
    tag = "documents", request_body = ArchiveRequest,
    params(("id" = Uuid, Path, description = "Document ID")),
    responses((status = 202, description = "Attempt accepted or already exists", body = ArchiveAdmission),
        BadRequest, NotFound,
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Document edit permission required"),
        (status = 409, description = "The original or expected attempt changed, or the attempt cannot be retried", body = crate::error::ErrorResponse),
        InternalError)
)]
pub(crate) async fn request(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(request): ApiJson<ArchiveRequest>,
) -> ApiResult<impl IntoResponse> {
    let attempt_id = admit(&state, vault, id, request).await?;
    Ok((
        StatusCode::ACCEPTED,
        NO_STORE,
        Json(ArchiveAdmission { attempt_id }),
    ))
}

pub(super) async fn admit(
    state: &AppState,
    vault: folio_core::models::VaultId,
    id: Uuid,
    request: ArchiveRequest,
) -> ApiResult<Uuid> {
    let document = state
        .document_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;
    if !crate::vault_context::can_edit_document(&document) {
        return Err(ApiError::forbidden("Document edit permission required"));
    }
    let service = service(state)?;
    Ok(match request.expected_attempt_id {
        Some(attempt) => service.retry(vault, id, attempt).await?,
        None => service.enqueue(vault, id).await?,
    })
}

#[utoipa::path(
    get, path = "/api/documents/{id}/archive/download", operation_id = "downloadDocumentArchive",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses((status = 200, description = "The current validated PDF/A copy", body = FileContent, content_type = "application/pdf"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Read permission required"),
        (status = 404, description = "Document or verified archival copy unavailable", body = crate::error::ErrorResponse),
        InternalError)
)]
pub(crate) async fn download(
    DefaultVault(vault): DefaultVault,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Response> {
    let permit = acquire_download(&DOWNLOAD_SLOTS)?;
    let bytes = service(&state)?
        .download(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Verified archival copy unavailable"))?;
    if let Some(user) = user {
        if state.access.effective_role(&user, vault).await?.is_none() {
            return Err(ApiError::not_found("Vault not found"));
        }
    }
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(header::CONTENT_LENGTH, bytes.len())
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(&format!("document-{id}-archive.pdf")),
        )
        .body(download_body(bytes, permit))
        .map_err(|_| ApiError::internal("Failed to build response"))?;
    super::file_response::harden(&mut response);
    Ok(response)
}

fn acquire_download(slots: &Semaphore) -> ApiResult<SemaphorePermit<'_>> {
    slots.try_acquire().map_err(|_| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Archival downloads are busy",
        )
        .with_retry_after(5)
    })
}

fn download_body(bytes: Vec<u8>, permit: SemaphorePermit<'static>) -> Body {
    // Slow clients retain the allocation, so keep their slot with the body.
    let stream = futures::stream::once(async move {
        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(bytes))
    })
    .map(move |chunk| {
        let _lease = &permit;
        chunk
    });
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn download_capacity_is_held_until_response_drop() {
        static SLOTS: Semaphore = Semaphore::const_new(1);
        let body = download_body(vec![1, 2, 3], acquire_download(&SLOTS).unwrap());
        let busy = acquire_download(&SLOTS).unwrap_err().into_response();
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(busy.headers()[header::RETRY_AFTER], "5");
        drop(body);
        assert!(acquire_download(&SLOTS).is_ok());
    }

    #[tokio::test]
    async fn consuming_download_releases_capacity_and_preserves_bytes() {
        static SLOTS: Semaphore = Semaphore::const_new(1);
        let body = download_body(vec![1, 2, 3], acquire_download(&SLOTS).unwrap());
        assert!(acquire_download(&SLOTS).is_err());
        assert_eq!(
            axum::body::to_bytes(body, 10).await.unwrap().as_ref(),
            &[1, 2, 3]
        );
        assert!(acquire_download(&SLOTS).is_ok());
    }
}

//! API error handling

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;

use folio_core::error::Error as DomainError;

/// API error response
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Human-readable error message
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// API error type
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    details: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: self.message,
            details: self.details,
        };

        (self.status, Json(body)).into_response()
    }
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::NotFound { entity_type, id } => {
                ApiError::not_found(format!("{} not found: {}", entity_type, id))
            }
            DomainError::Validation(msg) => ApiError::bad_request(msg),
            DomainError::Duplicate {
                entity_type,
                field,
                value,
            } => ApiError::new(
                StatusCode::CONFLICT,
                format!("{} with {} '{}' already exists", entity_type, field, value),
            ),
            DomainError::Conflict(msg) => ApiError::new(StatusCode::CONFLICT, msg),
            DomainError::Repository(msg) => {
                tracing::error!("Repository error: {}", msg);
                ApiError::internal("Database error")
            }
            DomainError::AiService(msg) => {
                tracing::error!("AI service error: {}", msg);
                ApiError::internal("AI service error").with_details(msg)
            }
            DomainError::Storage(msg) => {
                tracing::error!("Storage error: {}", msg);
                ApiError::internal("Storage error")
            }
        }
    }
}

/// Result type alias for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

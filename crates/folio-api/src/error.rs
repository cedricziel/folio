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
    /// Stable machine-readable reason, such as `forbidden`, where one is defined
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// API error type
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    code: Option<&'static str>,
    details: Option<String>,
    retry_after_secs: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            code: None,
            details: None,
            retry_after_secs: None,
        }
    }

    /// Sets the stable machine-readable `code` of the body
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Tells the client when to try again, as a `Retry-After` header
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after_secs = Some(seconds);
        self
    }

    /// 403 for a bearer token that lacks `scope`, with the error code `insufficient_scope`
    pub fn insufficient_scope(scope: &str) -> Self {
        Self {
            code: Some("insufficient_scope"),
            ..Self::new(
                StatusCode::FORBIDDEN,
                format!("This token needs the {scope} scope"),
            )
        }
    }

    /// 403 with the error code `forbidden`
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            code: Some("forbidden"),
            ..Self::new(StatusCode::FORBIDDEN, message)
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
            code: self.code,
            details: self.details,
        };

        let mut response = (self.status, Json(body)).into_response();
        if let Some(seconds) = self.retry_after_secs {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(seconds),
            );
        }
        response
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
            DomainError::Forbidden(msg) => ApiError::forbidden(msg),
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

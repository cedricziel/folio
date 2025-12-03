//! Domain error types

use thiserror::Error;

/// Domain-level errors
#[derive(Error, Debug)]
pub enum Error {
    #[error("Entity not found: {entity_type} with id '{id}'")]
    NotFound {
        entity_type: &'static str,
        id: String,
    },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Duplicate entity: {entity_type} with {field} '{value}' already exists")]
    Duplicate {
        entity_type: &'static str,
        field: &'static str,
        value: String,
    },

    #[error("Repository error: {0}")]
    Repository(String),

    #[error("AI service error: {0}")]
    AiService(String),

    #[error("Storage error: {0}")]
    Storage(String),
}

/// Result type alias for domain operations
pub type Result<T> = std::result::Result<T, Error>;

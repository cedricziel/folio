//! Folio API - HTTP layer
//!
//! This crate provides the REST API and web interface using Axum.

pub mod error;
pub mod events;
pub mod openapi;
pub mod routes;
pub mod state;

pub use events::{DocumentEvent, EventBroadcaster};
pub use routes::create_router;
pub use state::{AppState, IngestionState};

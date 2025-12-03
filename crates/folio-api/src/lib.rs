//! Folio API - HTTP layer
//!
//! This crate provides the REST API and web interface using Axum.

pub mod error;
pub mod routes;
pub mod state;

pub use routes::create_router;
pub use state::AppState;

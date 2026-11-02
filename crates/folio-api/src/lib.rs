//! Folio API - HTTP layer
//!
//! This crate provides the REST API and web interface using Axum.

pub mod client_telemetry;
pub mod error;
pub mod events;
pub mod extract;
pub mod forward;
pub mod openapi;
pub mod routes;
pub mod state;
pub mod telemetry;
pub mod vault;

pub use events::{DocumentEvent, EventBroadcaster};
pub use forward::forward_auth;
pub use routes::{
    auth::{enforce_auth, router as auth_router, AuthState, RequireUser},
    create_router,
};
pub use state::{AppState, IngestionState};
pub use vault::DefaultVault;

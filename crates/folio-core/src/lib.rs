//! Folio Core - Domain layer
//!
//! This crate contains the core domain logic for Folio, including:
//! - Domain models (Document, Tag, Correspondent, DocumentType)
//! - Port traits (repositories, AI services)
//! - Business services
//! - Domain events

pub mod document_transitions;
pub mod domain_event;
pub mod error;
pub mod events;
pub mod models;
pub mod mutation;
pub mod ports;
pub mod processing_events;
pub mod redact;
pub mod services;

pub use domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
pub use error::{Error, Result};
pub use events::{
    DeletedDocument, DocumentEvent, EventBroadcaster, EventOrigin, EventSnapshot, OutboxEvent,
};

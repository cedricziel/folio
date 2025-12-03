//! Folio Core - Domain layer
//!
//! This crate contains the core domain logic for Folio, including:
//! - Domain models (Document, Tag, Correspondent, DocumentType)
//! - Port traits (repositories, AI services)
//! - Business services

pub mod error;
pub mod models;
pub mod ports;
pub mod services;

pub use error::{Error, Result};

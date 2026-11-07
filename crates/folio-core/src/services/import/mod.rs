//! Planning logic for Paperless imports and syncs
//!
//! Everything here is pure: callers pass in reader records and the vault's
//! current state as values, and get plans back. Nothing touches a repository.

pub mod fields;

pub use fields::{
    field_kind_conflicts, folio_kind, ExistingField, FieldConflictReason, FieldKindConflict,
    ASN_FIELD_NAME,
};

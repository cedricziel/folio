//! Planning logic for Paperless imports and syncs
//!
//! Everything here is pure: callers pass in reader records and the vault's
//! current state as values, and get plans back. Nothing touches a repository.

pub mod fields;
pub mod taxonomy;

use serde::{Deserialize, Serialize};

pub use fields::{
    field_kind_conflicts, folio_kind, ExistingField, FieldConflictReason, FieldKindConflict,
    ASN_FIELD_NAME,
};
pub use taxonomy::{
    plan_fields, plan_named, plan_tags, ExistingEntity, FieldPlan, KindPlan, TagPlan, Target,
    TaxonomyKind, TaxonomyMapping, TaxonomyWarning, VaultTaxonomy,
};

/// Which Paperless changes a run applies; a `once` source's first run plans
/// as `NewOnly`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Only documents Folio hasn't seen are created
    NewOnly,
    /// Creates, updates, trashes, restores and renames follow Paperless
    Mirror,
}

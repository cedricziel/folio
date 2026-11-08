//! Planning logic for Paperless imports and syncs
//!
//! Everything here is pure: callers pass in reader records and the vault's
//! current state as values, and get plans back. Nothing touches a repository.

mod diff;
mod fields;
mod fingerprint;
mod payload;
mod taxonomy;
mod trash;
mod values;

#[cfg(test)]
mod test_support;

use serde::{Deserialize, Serialize};

pub use diff::{FolioState, ItemAction, LedgerEntry, SkipReason};
pub use fields::{
    plan_fields, ExistingField, FieldConflict, FieldConflictReason, FieldPlan, FieldTarget,
    NewField, ASN_FIELD_NAME,
};
pub use fingerprint::{
    document_id, fingerprint, DocumentRecords, FINGERPRINT_VERSION, IMPORT_NAMESPACE,
};
pub use payload::{
    plan_document, DraftItem, FailReason, ItemOutcome, ItemPayload, PlanContext, PlannedItem,
};
pub use taxonomy::{
    plan_named, plan_tags, ExistingEntity, FlattenedTag, KindPlan, MappedEntity, NewEntity, Rename,
    TagPlan, Target, TaxonomyKind, TaxonomyMapping, TaxonomyWarning, VaultTaxonomy,
};
pub use trash::{TrashGate, TrashHealth, TrashPlan, TRASH_MAX_COUNT, TRASH_MAX_PERCENT};
pub use values::{
    ItemWarning, PlannedNote, PlannedValue, PlannedValues, ValueContext, DEFAULT_CURRENCY,
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

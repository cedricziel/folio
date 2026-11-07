//! Paperless import runs, their items, the sync ledger and the sync state

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Id, UserId, VaultId};

/// How long a claimed item may stay `in_progress` before another driver may
/// take it over
pub const IMPORT_CLAIM_TIMEOUT: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, String> {
                match value {
                    $($text => Ok(Self::$variant),)+
                    other => Err(format!(
                        concat!("unknown ", stringify!($name), " '{}'"),
                        other
                    )),
                }
            }
        }
    };
}

text_enum!(
    /// Where a run reads its library from
    ImportRunKind {
        PaperlessExport => "paperless_export",
        PaperlessSource => "paperless_source",
    }
);

text_enum!(
    /// What started a run
    ImportTrigger {
        Manual => "manual",
        Schedule => "schedule",
        Initial => "initial",
    }
);

text_enum!(
    /// What an item does to its document
    ImportAction {
        Create => "create",
        Update => "update",
        Trash => "trash",
        Restore => "restore",
    }
);

text_enum!(
    ImportRunStatus {
        Queued => "queued",
        Running => "running",
        Completed => "completed",
        CompletedWithErrors => "completed_with_errors",
        Failed => "failed",
        Cancelled => "cancelled",
    }
);

impl ImportRunStatus {
    /// Queued and running runs block a new run in the same vault
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

text_enum!(
    ImportItemStatus {
        Pending => "pending",
        InProgress => "in_progress",
        Done => "done",
        Skipped => "skipped",
        Failed => "failed",
        WouldCreate => "would_create",
        WouldUpdate => "would_update",
        WouldTrash => "would_trash",
        WouldRestore => "would_restore",
    }
);

text_enum!(
    /// Who last decided a ledger document's fate in Folio
    FolioState {
        Live => "live",
        TrashedByUser => "trashed_by_user",
        PurgedInFolio => "purged_in_folio",
    }
);

text_enum!(
    TaxonomyKind {
        Tag => "tag",
        Correspondent => "correspondent",
        DocumentType => "document_type",
        CustomField => "custom_field",
    }
);

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRun {
    pub id: Id,
    pub vault_id: VaultId,
    pub kind: ImportRunKind,
    pub trigger: ImportTrigger,
    pub source_id: Option<Id>,
    pub source_path: Option<String>,
    pub dry_run: bool,
    pub status: ImportRunStatus,
    /// Highest Paperless `modified` the run fully processed; copied to the
    /// sync state's cursor when the run completes
    pub cursor_after: Option<DateTime<Utc>>,
    pub confirm_trash: bool,
    pub plan_complete: bool,
    pub cancel_requested: bool,
    pub driver_restarts: u32,
    pub params: serde_json::Value,
    pub summary: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A run to create; it starts `queued`
#[derive(Debug, Clone, PartialEq)]
pub struct NewImportRun {
    pub vault_id: VaultId,
    pub kind: ImportRunKind,
    pub trigger: ImportTrigger,
    pub source_id: Option<Id>,
    pub source_path: Option<String>,
    pub dry_run: bool,
    pub params: serde_json::Value,
    pub created_by: Option<UserId>,
}

/// The vault already has a queued or running run
#[derive(Debug, Clone, PartialEq)]
pub enum CreateImportRun {
    Created(Box<ImportRun>),
    Conflict { active_run: Id },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportItem {
    pub run_id: Id,
    pub paperless_id: i64,
    pub action: ImportAction,
    pub document_id: Option<Id>,
    pub title: Option<String>,
    pub status: ImportItemStatus,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
    pub payload: Option<serde_json::Value>,
    pub attempts: u32,
    pub claim: Option<Id>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// An item written by the plan phase
#[derive(Debug, Clone, PartialEq)]
pub struct NewImportItem {
    pub paperless_id: i64,
    pub action: ImportAction,
    pub document_id: Option<Id>,
    pub title: Option<String>,
    pub status: ImportItemStatus,
    pub reason: Option<String>,
    pub payload: Option<serde_json::Value>,
}

/// The final state of a claimed item
#[derive(Debug, Clone, PartialEq)]
pub struct ImportItemOutcome {
    pub status: ImportItemStatus,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
}

/// A document change applied for a claimed item: its ledger row and the
/// item's outcome, written in the document change's transaction
#[derive(Debug, Clone, PartialEq)]
pub struct ImportApplied {
    pub ledger: PaperlessLedgerEntry,
    pub run_id: Id,
    pub claim: Id,
    pub outcome: ImportItemOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportItemCount {
    pub action: ImportAction,
    pub status: ImportItemStatus,
    pub count: u64,
}

/// One row of the sync ledger
#[derive(Debug, Clone, PartialEq)]
pub struct PaperlessLedgerEntry {
    pub vault_id: VaultId,
    pub paperless_id: i64,
    pub document_id: Id,
    pub paperless_modified: DateTime<Utc>,
    pub checksum: String,
    pub fingerprint: String,
    pub needs_reapply: bool,
    pub trashed_in_paperless: bool,
    pub folio_state: FolioState,
    pub last_synced_at: DateTime<Utc>,
    pub last_run_id: Option<Id>,
}

/// A Paperless taxonomy entry and the Folio entity it maps to
#[derive(Debug, Clone, PartialEq)]
pub struct TaxonomyMapping {
    pub vault_id: VaultId,
    pub kind: TaxonomyKind,
    pub paperless_id: i64,
    pub entity_id: Id,
    pub name: String,
}

/// One document of the library-identity sample
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySample {
    pub paperless_id: i64,
    pub added: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperlessSyncState {
    pub vault_id: VaultId,
    pub source_id: Option<Id>,
    pub library_sample: Option<Vec<LibrarySample>>,
    pub cursor_modified: Option<DateTime<Utc>>,
    pub last_poll_at: Option<DateTime<Utc>>,
    pub last_full_reconcile_at: Option<DateTime<Utc>>,
    pub once_done: bool,
    /// Failed Paperless IDs and how many polls have retried them
    pub retry_ids: BTreeMap<i64, u32>,
    pub parked_ids: BTreeSet<i64>,
    pub updated_at: DateTime<Utc>,
}

impl PaperlessSyncState {
    pub fn new(vault_id: VaultId, now: DateTime<Utc>) -> Self {
        Self {
            vault_id,
            source_id: None,
            library_sample: None,
            cursor_modified: None,
            last_poll_at: None,
            last_full_reconcile_at: None,
            once_done: false,
            retry_ids: BTreeMap::new(),
            parked_ids: BTreeSet::new(),
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_round_trip_through_text() {
        for status in ImportItemStatus::ALL {
            assert_eq!(
                status.as_str().parse::<ImportItemStatus>().unwrap(),
                *status
            );
        }
        for status in ImportRunStatus::ALL {
            assert_eq!(status.as_str().parse::<ImportRunStatus>().unwrap(), *status);
        }
        assert!("nope".parse::<ImportAction>().is_err());
    }
}

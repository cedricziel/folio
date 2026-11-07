//! What a run does with each Paperless document, given its ledger row

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::fingerprint::FINGERPRINT_VERSION;
use super::SyncMode;
use crate::models::Id;
use crate::ports::PaperlessDocument;

/// What Folio users did to a synced document since
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolioState {
    Live,
    TrashedByUser,
    PurgedInFolio,
}

/// A `paperless_documents` ledger row, as planning reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub paperless_id: i64,
    pub document_id: Id,
    pub paperless_modified: DateTime<Utc>,
    pub checksum: String,
    pub fingerprint: String,
    pub fingerprint_version: u32,
    pub needs_reapply: bool,
    pub trashed_in_paperless: bool,
    pub folio_state: FolioState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ItemAction {
    Create,
    Update { replace_file: bool },
    Trash,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Unchanged,
    /// Trashed in Paperless before Folio ever saw it
    Trashed,
    TrashedInFolio,
    PurgedInFolio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Apply(ItemAction),
    Skip(SkipReason),
    /// The mode doesn't apply this change; no item is written
    Ignore,
}

/// Compares a document from the reader against its ledger row. `original`
/// is the file Folio keeps (see `DocumentRecords::original`).
pub fn decide(
    root: &PaperlessDocument,
    original: &PaperlessDocument,
    fingerprint: &str,
    ledger: Option<&LedgerEntry>,
    mode: SyncMode,
) -> Decision {
    let Some(row) = ledger else {
        return if root.trashed {
            Decision::Skip(SkipReason::Trashed)
        } else {
            Decision::Apply(ItemAction::Create)
        };
    };
    let decision = match row.folio_state {
        FolioState::TrashedByUser => Decision::Skip(SkipReason::TrashedInFolio),
        FolioState::PurgedInFolio => Decision::Skip(SkipReason::PurgedInFolio),
        FolioState::Live => match (root.trashed, row.trashed_in_paperless) {
            (true, false) => Decision::Apply(ItemAction::Trash),
            (true, true) => Decision::Skip(SkipReason::Unchanged),
            (false, true) => Decision::Apply(ItemAction::Restore),
            (false, false) if original.checksum.value != row.checksum => {
                Decision::Apply(ItemAction::Update { replace_file: true })
            }
            (false, false)
                if row.needs_reapply
                    || row.fingerprint_version != FINGERPRINT_VERSION
                    || row.fingerprint != fingerprint =>
            {
                Decision::Apply(ItemAction::Update {
                    replace_file: false,
                })
            }
            (false, false) => Decision::Skip(SkipReason::Unchanged),
        },
    };
    match (mode, decision) {
        (SyncMode::NewOnly, Decision::Apply(_)) => Decision::Ignore,
        _ => decision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::import::fingerprint::tests::document;

    const FP: &str = "fp";

    fn row() -> LedgerEntry {
        let doc = document(1);
        LedgerEntry {
            paperless_id: 1,
            document_id: Id::new_v4(),
            paperless_modified: doc.modified,
            checksum: doc.checksum.value,
            fingerprint: FP.into(),
            fingerprint_version: FINGERPRINT_VERSION,
            needs_reapply: false,
            trashed_in_paperless: false,
            folio_state: FolioState::Live,
        }
    }

    fn mirror(doc: &PaperlessDocument, fp: &str, row: Option<&LedgerEntry>) -> Decision {
        decide(doc, doc, fp, row, SyncMode::Mirror)
    }

    #[test]
    fn unseen_documents_are_created_unless_trashed() {
        let mut doc = document(1);
        assert_eq!(mirror(&doc, FP, None), Decision::Apply(ItemAction::Create));
        doc.trashed = true;
        assert_eq!(mirror(&doc, FP, None), Decision::Skip(SkipReason::Trashed));
    }

    #[test]
    fn unchanged_documents_are_skipped() {
        assert_eq!(
            mirror(&document(1), FP, Some(&row())),
            Decision::Skip(SkipReason::Unchanged)
        );
    }

    #[test]
    fn changed_state_updates_and_a_changed_file_replaces_it() {
        let doc = document(1);
        let update = Decision::Apply(ItemAction::Update {
            replace_file: false,
        });
        assert_eq!(mirror(&doc, "other", Some(&row())), update);

        let reapply = LedgerEntry {
            needs_reapply: true,
            ..row()
        };
        assert_eq!(mirror(&doc, FP, Some(&reapply)), update);

        let old_version = LedgerEntry {
            fingerprint_version: 0,
            ..row()
        };
        assert_eq!(mirror(&doc, FP, Some(&old_version)), update);

        let mut new_file = document(1);
        new_file.checksum.value = "1".repeat(32);
        assert_eq!(
            mirror(&new_file, FP, Some(&row())),
            Decision::Apply(ItemAction::Update { replace_file: true })
        );
    }

    #[test]
    fn paperless_trash_and_restore_are_mirrored() {
        let mut trashed = document(1);
        trashed.trashed = true;
        assert_eq!(
            mirror(&trashed, FP, Some(&row())),
            Decision::Apply(ItemAction::Trash)
        );

        let in_trash = LedgerEntry {
            trashed_in_paperless: true,
            ..row()
        };
        assert_eq!(
            mirror(&trashed, FP, Some(&in_trash)),
            Decision::Skip(SkipReason::Unchanged)
        );
        assert_eq!(
            mirror(&document(1), FP, Some(&in_trash)),
            Decision::Apply(ItemAction::Restore)
        );
    }

    #[test]
    fn documents_folio_users_trashed_or_purged_are_left_alone() {
        let purged = LedgerEntry {
            folio_state: FolioState::PurgedInFolio,
            ..row()
        };
        assert_eq!(
            mirror(&document(1), "changed", Some(&purged)),
            Decision::Skip(SkipReason::PurgedInFolio)
        );

        let trashed = LedgerEntry {
            folio_state: FolioState::TrashedByUser,
            trashed_in_paperless: true,
            ..row()
        };
        assert_eq!(
            mirror(&document(1), "changed", Some(&trashed)),
            Decision::Skip(SkipReason::TrashedInFolio)
        );
    }

    #[test]
    fn new_only_keeps_only_creates() {
        let doc = document(1);
        let new_only = |row| decide(&doc, &doc, "changed", row, SyncMode::NewOnly);
        assert_eq!(new_only(None), Decision::Apply(ItemAction::Create));
        assert_eq!(new_only(Some(&row())), Decision::Ignore);
    }
}

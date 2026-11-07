//! Trash safety: mirror sync trashes only from a complete listing, never
//! from an empty one, and holds back when a run would trash too much

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::diff::{FolioState, ItemAction, LedgerEntry};
use super::payload::{FailReason, ItemOutcome, PlannedItem};
use crate::models::Id;
use crate::ports::CompleteIdSet;

/// Above this many trash items a run waits for `confirm_trash`
pub const TRASH_MAX_COUNT: usize = 50;
/// Above this share of the live ledger a run waits for `confirm_trash`
pub const TRASH_MAX_PERCENT: usize = 10;

/// A synced document that is gone from Paperless's listing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingDocument {
    pub paperless_id: i64,
    pub document_id: Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashHealth {
    /// Paperless listed nothing while the ledger has live documents; nothing
    /// is trashed and the source's health reports it
    EmptyListing,
}

/// Rows the sync still considers live on both sides
fn is_live(row: &LedgerEntry) -> bool {
    row.folio_state == FolioState::Live && !row.trashed_in_paperless
}

/// Live ledger rows whose Paperless ID is missing from the listing. Only a
/// [`CompleteIdSet`] is accepted, so a failed listing can't trash anything.
pub fn missing_from_listing<'a>(
    ledger: impl IntoIterator<Item = &'a LedgerEntry>,
    listing: &CompleteIdSet,
) -> Result<Vec<MissingDocument>, TrashHealth> {
    let mut live = ledger.into_iter().filter(|row| is_live(row)).peekable();
    if listing.ids().is_empty() && live.peek().is_some() {
        return Err(TrashHealth::EmptyListing);
    }
    let mut missing: Vec<_> = live
        .filter(|row| !listing.contains(row.paperless_id))
        .map(|row| MissingDocument {
            paperless_id: row.paperless_id,
            document_id: row.document_id,
        })
        .collect();
    missing.sort_by_key(|m| m.paperless_id);
    Ok(missing)
}

/// Whether a run's trash items wait for an admin: more than 50, or more than
/// 10% of the live ledger, unless the run was retried with `confirm_trash`
pub fn trash_held(trash_items: usize, live_ledger_rows: usize, confirm_trash: bool) -> bool {
    !confirm_trash
        && (trash_items > TRASH_MAX_COUNT || trash_items * 100 > live_ledger_rows * TRASH_MAX_PERCENT)
}

/// Counts the rows [`trash_held`] measures against
pub fn live_ledger_rows<'a>(ledger: impl IntoIterator<Item = &'a LedgerEntry>) -> usize {
    ledger.into_iter().filter(|row| is_live(row)).count()
}

/// Adds a trash item per missing document not already planned, then applies the safety rules to
/// every trash item: none at all after an empty listing, and all of them
/// failed with `trash_threshold_exceeded` above the threshold. Returns the
/// health problem to record on the source, if any.
pub fn apply_trash_safety(
    items: &mut Vec<PlannedItem>,
    missing: Result<Vec<MissingDocument>, TrashHealth>,
    live_ledger_rows: usize,
    confirm_trash: bool,
) -> Option<TrashHealth> {
    let is_trash = |item: &PlannedItem| item.outcome == ItemOutcome::Pending(ItemAction::Trash);
    let missing = match missing {
        Ok(missing) => missing,
        Err(health) => {
            items.retain(|item| !is_trash(item));
            return Some(health);
        }
    };
    // A document trashed in Paperless is missing from the live listing too
    let planned: HashSet<i64> = items.iter().map(|item| item.paperless_id).collect();
    let missing = missing
        .into_iter()
        .filter(|m| !planned.contains(&m.paperless_id));
    items.extend(missing.map(|m| PlannedItem {
        paperless_id: m.paperless_id,
        document_id: m.document_id,
        title: String::new(),
        outcome: ItemOutcome::Pending(ItemAction::Trash),
        payload: None,
    }));
    let count = items.iter().filter(|item| is_trash(item)).count();
    if trash_held(count, live_ledger_rows, confirm_trash) {
        for item in items.iter_mut().filter(|item| is_trash(item)) {
            item.outcome = ItemOutcome::Failed(FailReason::TrashThresholdExceeded);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::services::import::FINGERPRINT_VERSION;

    fn row(paperless_id: i64) -> LedgerEntry {
        LedgerEntry {
            paperless_id,
            document_id: Id::from_u128(paperless_id as u128),
            paperless_modified: chrono::Utc::now(),
            checksum: String::new(),
            fingerprint: String::new(),
            fingerprint_version: FINGERPRINT_VERSION,
            needs_reapply: false,
            trashed_in_paperless: false,
            folio_state: FolioState::Live,
        }
    }

    fn listing(ids: impl IntoIterator<Item = i64>) -> CompleteIdSet {
        CompleteIdSet::from_complete_listing(ids.into_iter().collect::<BTreeSet<_>>())
    }

    #[test]
    fn documents_missing_from_a_complete_listing_are_trash_candidates() {
        let ledger = [row(1), row(2), row(3)];
        let missing = missing_from_listing(&ledger, &listing([1, 3, 4])).unwrap();
        assert_eq!(
            missing,
            vec![MissingDocument {
                paperless_id: 2,
                document_id: Id::from_u128(2)
            }]
        );
    }

    #[test]
    fn an_empty_listing_against_a_live_ledger_trashes_nothing() {
        let ledger = [row(1), row(2)];
        assert_eq!(
            missing_from_listing(&ledger, &listing([])),
            Err(TrashHealth::EmptyListing)
        );
        assert_eq!(missing_from_listing(&[], &listing([])), Ok(vec![]));
    }

    #[test]
    fn rows_that_are_not_live_are_never_trash_candidates() {
        let purged = LedgerEntry {
            folio_state: FolioState::PurgedInFolio,
            ..row(1)
        };
        let user_trashed = LedgerEntry {
            folio_state: FolioState::TrashedByUser,
            ..row(2)
        };
        let paperless_trashed = LedgerEntry {
            trashed_in_paperless: true,
            ..row(3)
        };
        let ledger = [purged, user_trashed, paperless_trashed, row(4)];
        assert_eq!(missing_from_listing(&ledger, &listing([4])), Ok(vec![]));
        // Only non-live rows: an empty listing is no warning sign
        assert_eq!(missing_from_listing(&ledger[..3], &listing([])), Ok(vec![]));
    }

    #[test]
    fn four_hundred_of_a_thousand_missing_hits_the_threshold() {
        let ledger: Vec<_> = (1..=1_000).map(row).collect();
        let missing = missing_from_listing(&ledger, &listing(401..=1_000)).unwrap();
        assert_eq!(missing.len(), 400);

        let live = live_ledger_rows(&ledger);
        let mut items = vec![];
        assert_eq!(apply_trash_safety(&mut items, Ok(missing.clone()), live, false), None);
        assert_eq!(items.len(), 400);
        assert!(items
            .iter()
            .all(|i| i.outcome == ItemOutcome::Failed(FailReason::TrashThresholdExceeded)));

        let mut confirmed = vec![];
        apply_trash_safety(&mut confirmed, Ok(missing), live, true);
        assert!(confirmed
            .iter()
            .all(|i| i.outcome == ItemOutcome::Pending(ItemAction::Trash)));
    }

    fn pending(paperless_id: i64, action: ItemAction) -> PlannedItem {
        PlannedItem {
            paperless_id,
            document_id: Id::from_u128(paperless_id as u128),
            title: String::new(),
            outcome: ItemOutcome::Pending(action),
            payload: None,
        }
    }

    #[test]
    fn an_empty_listing_drops_every_trash_item_and_reports_it() {
        let mut items = vec![pending(1, ItemAction::Trash), pending(2, ItemAction::Create)];
        let health = apply_trash_safety(&mut items, Err(TrashHealth::EmptyListing), 10, true);
        assert_eq!(health, Some(TrashHealth::EmptyListing));
        assert_eq!(items, vec![pending(2, ItemAction::Create)]);
    }

    #[test]
    fn trash_items_from_paperless_count_towards_the_threshold() {
        let ledger: Vec<_> = (1..=100).map(row).collect();
        let mut items: Vec<_> = (1..=10).map(|id| pending(id, ItemAction::Trash)).collect();
        items.push(pending(200, ItemAction::Create));
        let missing = missing_from_listing(&ledger, &listing(12..=100)).unwrap();

        apply_trash_safety(&mut items, Ok(missing), live_ledger_rows(&ledger), false);

        let held = items
            .iter()
            .filter(|i| i.outcome == ItemOutcome::Failed(FailReason::TrashThresholdExceeded))
            .count();
        assert_eq!(held, 11);
        assert!(items.contains(&pending(200, ItemAction::Create)));
    }

    #[test]
    fn the_lower_of_fifty_items_and_ten_percent_holds_trashing() {
        assert!(!trash_held(50, 10_000, false));
        assert!(trash_held(51, 10_000, false));
        assert!(!trash_held(10, 100, false));
        assert!(trash_held(11, 100, false));
        assert!(!trash_held(0, 0, false));
    }
}

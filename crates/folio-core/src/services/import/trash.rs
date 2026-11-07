//! Trash safety: mirror sync trashes only from a complete listing, never
//! from an empty one, and holds back when a run would trash too much

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::diff::{FolioState, ItemAction, LedgerEntry};
use super::payload::{FailReason, ItemOutcome, PlannedItem};
use crate::ports::CompleteIdSet;

/// Above this many trash items a run waits for `confirm_trash`
pub const TRASH_MAX_COUNT: usize = 50;
/// Above this share of the live ledger a run waits for `confirm_trash`
pub const TRASH_MAX_PERCENT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashHealth {
    /// Paperless listed nothing while the ledger has live documents; nothing
    /// is trashed and the source's health reports it
    EmptyListing,
}

/// Adds a trash item for each live ledger row missing from the listing, then
/// applies the safety rules to every trash item: none at all after an empty
/// listing, and all of them failed with `trash_threshold_exceeded` when there
/// are more than 50, or more than 10% of the live ledger, unless the run was
/// retried with `confirm_trash`. Only a [`CompleteIdSet`] is accepted, so a
/// failed listing can't trash anything. Returns the health problem to record
/// on the source, if any.
pub fn apply_trash_safety(
    items: &mut Vec<PlannedItem>,
    ledger: &[LedgerEntry],
    listing: &CompleteIdSet,
    confirm_trash: bool,
) -> Option<TrashHealth> {
    let is_trash = |item: &PlannedItem| item.outcome == ItemOutcome::Pending(ItemAction::Trash);
    let mut live: Vec<&LedgerEntry> = ledger
        .iter()
        .filter(|row| row.folio_state == FolioState::Live && !row.trashed_in_paperless)
        .collect();
    if listing.ids().is_empty() && !live.is_empty() {
        items.retain(|item| !is_trash(item));
        return Some(TrashHealth::EmptyListing);
    }
    let live_rows = live.len();

    // A document trashed in Paperless is missing from the live listing too
    let planned: HashSet<i64> = items.iter().map(|item| item.paperless_id).collect();
    live.retain(|row| !listing.contains(row.paperless_id) && !planned.contains(&row.paperless_id));
    live.sort_by_key(|row| row.paperless_id);
    items.extend(live.into_iter().map(|row| PlannedItem {
        paperless_id: row.paperless_id,
        document_id: row.document_id,
        title: String::new(),
        outcome: ItemOutcome::Pending(ItemAction::Trash),
        payload: None,
    }));

    let count = items.iter().filter(|item| is_trash(item)).count();
    let held = count > TRASH_MAX_COUNT || count * 100 > live_rows * TRASH_MAX_PERCENT;
    if held && !confirm_trash {
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
    use crate::models::Id;
    use crate::services::import::test_support::ledger_row as row;

    fn listing(ids: impl IntoIterator<Item = i64>) -> CompleteIdSet {
        CompleteIdSet::from_complete_listing(ids.into_iter().collect::<BTreeSet<_>>())
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

    const HELD: ItemOutcome = ItemOutcome::Failed(FailReason::TrashThresholdExceeded);

    /// Plans trash with no other items, expecting no health problem
    fn plan(ledger: &[LedgerEntry], listed: CompleteIdSet, confirm: bool) -> Vec<PlannedItem> {
        let mut items = vec![];
        assert_eq!(
            apply_trash_safety(&mut items, ledger, &listed, confirm),
            None
        );
        items
    }

    #[test]
    fn documents_missing_from_a_complete_listing_are_trashed() {
        let ledger: Vec<_> = (1..=100).map(row).collect();
        let items = plan(&ledger, listing((1..=100).filter(|id| *id != 2)), false);
        assert_eq!(items, vec![pending(2, ItemAction::Trash)]);
    }

    #[test]
    fn an_empty_listing_against_a_live_ledger_trashes_nothing() {
        let mut items = vec![
            pending(1, ItemAction::Trash),
            pending(2, ItemAction::Create),
        ];
        let health = apply_trash_safety(&mut items, &[row(1), row(3)], &listing([]), true);
        assert_eq!(health, Some(TrashHealth::EmptyListing));
        assert_eq!(items, vec![pending(2, ItemAction::Create)]);

        assert!(plan(&[], listing([]), false).is_empty());
    }

    #[test]
    fn rows_that_are_not_live_are_never_trashed() {
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
        assert!(plan(&ledger, listing([4]), false).is_empty());
        // Only non-live rows: an empty listing is no warning sign
        assert!(plan(&ledger[..3], listing([]), false).is_empty());
    }

    #[test]
    fn four_hundred_of_a_thousand_missing_hits_the_threshold() {
        let ledger: Vec<_> = (1..=1_000).map(row).collect();

        let held = plan(&ledger, listing(401..=1_000), false);
        assert_eq!(held.len(), 400);
        assert!(held.iter().all(|i| i.outcome == HELD));

        let confirmed = plan(&ledger, listing(401..=1_000), true);
        assert!(confirmed
            .iter()
            .all(|i| i.outcome == ItemOutcome::Pending(ItemAction::Trash)));
    }

    #[test]
    fn the_lower_of_fifty_items_and_ten_percent_holds_trashing() {
        let held = |size: i64, missing: i64| {
            let ledger: Vec<_> = (1..=size).map(row).collect();
            plan(&ledger, listing(missing + 1..=size), false)[0].outcome == HELD
        };
        assert!(!held(10_000, 50));
        assert!(held(10_000, 51));
        assert!(!held(100, 10));
        assert!(held(100, 11));
    }

    #[test]
    fn trash_items_from_paperless_count_once_towards_the_threshold() {
        let ledger: Vec<_> = (1..=100).map(row).collect();
        let mut items: Vec<_> = (1..=10).map(|id| pending(id, ItemAction::Trash)).collect();
        items.push(pending(200, ItemAction::Create));

        apply_trash_safety(&mut items, &ledger, &listing(12..=100), false);

        assert_eq!(items.iter().filter(|i| i.outcome == HELD).count(), 11);
        assert!(items.contains(&pending(200, ItemAction::Create)));
    }
}

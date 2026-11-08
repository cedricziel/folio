//! Trash safety: mirror sync trashes only from a complete listing, never
//! from an empty one, and holds back when a run would trash too much

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::diff::{FolioState, ItemAction, LedgerEntry};
use super::payload::{DraftItem, FailReason, ItemOutcome, PlannedItem};
use super::SyncMode;
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

/// The only way out of planning: every [`DraftItem`] passes through here, so
/// no trash item escapes the safety rules.
///
/// Trash items exist only in `mirror` mode with a [`CompleteIdSet`]; without
/// one (the listing failed, or the run doesn't list) every trash item is
/// dropped. An empty listing against live ledger rows drops them too and
/// reports [`TrashHealth::EmptyListing`]. Otherwise each live ledger row
/// missing from the listing becomes a trash item, and when there are more
/// than 50 trash items, or more than 10% of the live ledger, all of them
/// fail with `trash_threshold_exceeded` unless the run has `confirm_trash`.
pub struct TrashGate<'a> {
    ledger: &'a [LedgerEntry],
    listing: Option<&'a CompleteIdSet>,
    confirm_trash: bool,
    seen: HashSet<i64>,
    trash: Vec<PlannedItem>,
}

/// What the gate releases once every draft went through it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashPlan {
    pub items: Vec<PlannedItem>,
    /// To record on the source
    pub health: Option<TrashHealth>,
}

impl<'a> TrashGate<'a> {
    pub fn new(
        ledger: &'a [LedgerEntry],
        listing: Option<&'a CompleteIdSet>,
        mode: SyncMode,
        confirm_trash: bool,
    ) -> Self {
        Self {
            ledger,
            listing: listing.filter(|_| mode == SyncMode::Mirror),
            confirm_trash,
            seen: HashSet::new(),
            trash: Vec::new(),
        }
    }

    /// Releases every item but trash at once, so planning streams; trash
    /// items wait for [`Self::finish`]
    pub fn admit(&mut self, DraftItem(item): DraftItem) -> Option<PlannedItem> {
        self.seen.insert(item.paperless_id);
        if !is_trash(&item) {
            return Some(item);
        }
        if self.listing.is_some() {
            self.trash.push(item);
        }
        None
    }

    pub fn finish(self) -> TrashPlan {
        let Some(listing) = self.listing else {
            return TrashPlan {
                items: Vec::new(),
                health: None,
            };
        };
        let mut live: Vec<&LedgerEntry> = self
            .ledger
            .iter()
            .filter(|row| row.folio_state == FolioState::Live && !row.trashed_in_paperless)
            .collect();
        if listing.ids().is_empty() && !live.is_empty() {
            return TrashPlan {
                items: Vec::new(),
                health: Some(TrashHealth::EmptyListing),
            };
        }
        let live_rows = live.len();

        // A document trashed in Paperless is missing from the live listing too
        live.retain(|row| {
            !listing.contains(row.paperless_id) && !self.seen.contains(&row.paperless_id)
        });
        live.sort_by_key(|row| row.paperless_id);
        let mut items = self.trash;
        items.extend(live.into_iter().map(|row| PlannedItem {
            paperless_id: row.paperless_id,
            document_id: row.document_id,
            title: None,
            outcome: ItemOutcome::Pending(ItemAction::Trash),
            payload: None,
        }));

        let count = items.len();
        let held = count > TRASH_MAX_COUNT || count * 100 > live_rows * TRASH_MAX_PERCENT;
        if held && !self.confirm_trash {
            for item in &mut items {
                item.outcome = ItemOutcome::Failed(FailReason::TrashThresholdExceeded);
            }
        }
        TrashPlan {
            items,
            health: None,
        }
    }
}

fn is_trash(item: &PlannedItem) -> bool {
    item.outcome == ItemOutcome::Pending(ItemAction::Trash)
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

    fn draft(paperless_id: i64, action: ItemAction) -> DraftItem {
        DraftItem(PlannedItem {
            paperless_id,
            document_id: Id::from_u128(paperless_id as u128),
            title: Some(format!("Document {paperless_id}")),
            outcome: ItemOutcome::Pending(action),
            payload: None,
        })
    }

    const HELD: ItemOutcome = ItemOutcome::Failed(FailReason::TrashThresholdExceeded);
    const TRASH: ItemOutcome = ItemOutcome::Pending(ItemAction::Trash);

    /// Runs drafts through a gate and returns everything it released
    fn gate(
        drafts: Vec<DraftItem>,
        ledger: &[LedgerEntry],
        listing: Option<&CompleteIdSet>,
        mode: SyncMode,
        confirm: bool,
    ) -> TrashPlan {
        let mut gate = TrashGate::new(ledger, listing, mode, confirm);
        let mut items: Vec<_> = drafts.into_iter().filter_map(|d| gate.admit(d)).collect();
        let mut plan = gate.finish();
        items.append(&mut plan.items);
        plan.items = items;
        plan
    }

    fn mirror(ledger: &[LedgerEntry], listed: &CompleteIdSet) -> Vec<PlannedItem> {
        gate(vec![], ledger, Some(listed), SyncMode::Mirror, false).items
    }

    fn ledger(size: i64) -> Vec<LedgerEntry> {
        (1..=size).map(row).collect()
    }

    #[test]
    fn a_failed_listing_produces_no_trash_even_for_documents_in_paperless_trash() {
        let plan = gate(
            vec![draft(1, ItemAction::Trash), draft(2, ItemAction::Create)],
            &ledger(10),
            None,
            SyncMode::Mirror,
            true,
        );
        assert_eq!(plan.items, vec![draft(2, ItemAction::Create).0]);
        assert_eq!(plan.health, None);
    }

    #[test]
    fn only_mirror_mode_trashes() {
        let plan = gate(
            vec![draft(1, ItemAction::Trash)],
            &ledger(10),
            Some(&listing(3..=10)),
            SyncMode::NewOnly,
            true,
        );
        assert!(plan.items.is_empty());
    }

    #[test]
    fn documents_missing_from_a_complete_listing_are_trashed() {
        let items = mirror(&ledger(100), &listing((1..=100).filter(|id| *id != 2)));
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].paperless_id, items[0].outcome), (2, TRASH));
        assert_eq!(items[0].title, None);
    }

    #[test]
    fn an_empty_listing_against_a_live_ledger_trashes_nothing() {
        let plan = gate(
            vec![draft(1, ItemAction::Trash), draft(2, ItemAction::Create)],
            &[row(1), row(3)],
            Some(&listing([])),
            SyncMode::Mirror,
            true,
        );
        assert_eq!(plan.health, Some(TrashHealth::EmptyListing));
        assert_eq!(plan.items, vec![draft(2, ItemAction::Create).0]);

        assert!(mirror(&[], &listing([])).is_empty());
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
        let rows = [purged, user_trashed, paperless_trashed, row(4)];
        assert!(mirror(&rows, &listing([4])).is_empty());
        // Only non-live rows: an empty listing is no warning sign
        let plan = gate(
            vec![],
            &rows[..3],
            Some(&listing([])),
            SyncMode::Mirror,
            false,
        );
        assert_eq!(
            plan,
            TrashPlan {
                items: vec![],
                health: None
            }
        );
    }

    #[test]
    fn four_hundred_of_a_thousand_missing_hits_the_threshold() {
        let rows = ledger(1_000);
        let present = listing(401..=1_000);

        let held = mirror(&rows, &present);
        assert_eq!(held.len(), 400);
        assert!(held.iter().all(|i| i.outcome == HELD));

        let confirmed = gate(vec![], &rows, Some(&present), SyncMode::Mirror, true).items;
        assert_eq!(confirmed.len(), 400);
        assert!(confirmed.iter().all(|i| i.outcome == TRASH));
    }

    #[test]
    fn the_lower_of_fifty_items_and_ten_percent_holds_trashing() {
        let held = |size: i64, missing: i64| {
            mirror(&ledger(size), &listing(missing + 1..=size))[0].outcome == HELD
        };
        assert!(!held(10_000, 50));
        assert!(held(10_000, 51));
        assert!(!held(500, 50));
        assert!(held(500, 51));
        assert!(!held(100, 10));
        assert!(held(100, 11));
        assert!(held(5, 1));
    }

    #[test]
    fn trash_items_from_paperless_count_once_towards_the_threshold() {
        let mut drafts: Vec<_> = (1..=10).map(|id| draft(id, ItemAction::Trash)).collect();
        drafts.push(draft(200, ItemAction::Create));

        let plan = gate(
            drafts,
            &ledger(100),
            Some(&listing(12..=100)),
            SyncMode::Mirror,
            false,
        );

        assert_eq!(plan.items.iter().filter(|i| i.outcome == HELD).count(), 11);
        assert!(plan.items.contains(&draft(200, ItemAction::Create).0));
    }
}

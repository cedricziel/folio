//! One planned item per Paperless document, with the self-contained payload
//! the apply phase needs

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::diff::{decide, Decision, ItemAction, LedgerEntry, SkipReason};
use super::fingerprint::{document_id, fingerprint_of, DocumentRecords, FINGERPRINT_VERSION};
use super::values::{plan_values, ItemWarning, PlannedValues, ValueContext};
use super::SyncMode;
use crate::models::{Id, VaultId};
use crate::ports::PaperlessChecksum;

/// Everything the apply phase writes for one document beyond the item's own
/// columns (Paperless ID, document ID, title), in Paperless terms; taxonomy
/// and fields are resolved through `paperless_taxonomy` at apply
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemPayload {
    pub fingerprint_version: u32,
    pub fingerprint: String,
    pub original_handle: String,
    pub checksum: PaperlessChecksum,
    pub size: Option<u64>,
    pub content: String,
    pub mime_type: String,
    pub original_filename: Option<String>,
    pub created: NaiveDate,
    pub added: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub correspondent: Option<i64>,
    pub document_type: Option<i64>,
    /// Sorted, without inbox tags
    pub tags: Vec<i64>,
    pub in_inbox: bool,
    pub trashed: bool,
    pub skipped_versions: usize,
    #[serde(flatten)]
    pub values: PlannedValues,
}

impl ItemPayload {
    /// Sets `needs_reapply` on the ledger row, so the next sync retries
    /// drops that may succeed later even if nothing changes in Paperless
    pub fn needs_reapply(&self) -> bool {
        self.values.warnings.iter().any(ItemWarning::retryable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum ItemOutcome {
    Pending(ItemAction),
    Skipped(SkipReason),
    Failed(FailReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// A trash item held back until a retry with `confirm_trash`
    TrashThresholdExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedItem {
    pub paperless_id: i64,
    pub document_id: Id,
    /// `None` for a trash item of a document the reader no longer lists
    pub title: Option<String>,
    pub outcome: ItemOutcome,
    /// Present for creates, updates and restores
    pub payload: Option<ItemPayload>,
}

/// A planned item that hasn't passed trash safety yet; only
/// [`super::trash::TrashGate`] turns it into a [`PlannedItem`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftItem(pub(super) PlannedItem);

pub struct PlanContext<'a> {
    pub vault: VaultId,
    pub mode: SyncMode,
    pub inbox_tags: &'a BTreeSet<i64>,
    pub values: ValueContext<'a>,
}

/// Plans one document; `None` when the mode doesn't apply its change
pub fn plan_document(
    records: &DocumentRecords,
    ledger: Option<&LedgerEntry>,
    ctx: &PlanContext<'_>,
) -> Option<DraftItem> {
    let root = &records.document;
    let original = records.original();
    let tags = records.tag_set();
    let in_inbox = !tags.is_disjoint(ctx.inbox_tags);
    let notes = records.live_notes();
    let fingerprint = fingerprint_of(records, original, tags.clone(), &notes, in_inbox);
    let document_id = ledger.map_or_else(|| document_id(ctx.vault, root.id), |r| r.document_id);

    let (outcome, payload) = match decide(root, original, &fingerprint, ledger, ctx.mode) {
        Decision::Ignore => return None,
        Decision::Skip(reason) => (ItemOutcome::Skipped(reason), None),
        Decision::Apply(ItemAction::Trash) => (ItemOutcome::Pending(ItemAction::Trash), None),
        Decision::Apply(action) => {
            let payload = ItemPayload {
                fingerprint_version: FINGERPRINT_VERSION,
                fingerprint,
                original_handle: original.original_handle.clone(),
                checksum: original.checksum.clone(),
                size: original.size,
                content: original.content.clone(),
                mime_type: original.mime_type.clone(),
                original_filename: original.original_filename.clone(),
                created: root.created,
                added: root.added,
                modified: root.modified,
                correspondent: root.correspondent,
                document_type: root.document_type,
                tags: tags.difference(ctx.inbox_tags).copied().collect(),
                in_inbox,
                trashed: root.trashed,
                skipped_versions: records.skipped_versions(),
                values: plan_values(
                    document_id,
                    &records.fields,
                    &notes,
                    root.archive_serial_number,
                    &ctx.values,
                ),
            };
            (ItemOutcome::Pending(action), Some(payload))
        }
    };
    Some(DraftItem(PlannedItem {
        paperless_id: root.id,
        document_id,
        title: Some(root.title.clone()),
        outcome,
        payload,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::ports::{
        PaperlessCustomField, PaperlessCustomFieldInstance, PaperlessFieldType,
        PaperlessFieldValue, PaperlessTag,
    };
    use crate::services::import::diff::FolioState;
    use crate::services::import::taxonomy::{plan_tags, TaxonomyMapping, VaultTaxonomy};
    use crate::services::import::test_support::{custom_field, document, note};
    use crate::services::import::values::DEFAULT_CURRENCY;
    use crate::services::import::ExistingEntity;

    const VAULT: VaultId = VaultId(Id::from_u128(7));

    struct Fixture {
        inbox: BTreeSet<i64>,
        fields: HashMap<i64, PaperlessCustomField>,
        asn_owners: HashMap<i64, Id>,
    }

    impl Fixture {
        fn new() -> Self {
            let link = custom_field(1, "Link", PaperlessFieldType::Url);
            Self {
                inbox: BTreeSet::from([4]),
                fields: HashMap::from([(1, link)]),
                asn_owners: HashMap::new(),
            }
        }

        fn plan(
            &self,
            records: &DocumentRecords,
            ledger: Option<&LedgerEntry>,
        ) -> Option<PlannedItem> {
            self.plan_in(SyncMode::Mirror, records, ledger)
        }

        fn plan_in(
            &self,
            mode: SyncMode,
            records: &DocumentRecords,
            ledger: Option<&LedgerEntry>,
        ) -> Option<PlannedItem> {
            let ctx = PlanContext {
                vault: VAULT,
                mode,
                inbox_tags: &self.inbox,
                values: ValueContext {
                    fields: &self.fields,
                    asn_owners: &self.asn_owners,
                    default_currency: DEFAULT_CURRENCY,
                },
            };
            plan_document(records, ledger, &ctx).map(|draft| draft.0)
        }
    }

    fn records() -> DocumentRecords {
        let mut records = DocumentRecords::new(document(1));
        records.notes = vec![note(1, 1, "hello")];
        records.fields = vec![PaperlessCustomFieldInstance {
            document: 1,
            field: 1,
            value: Some(PaperlessFieldValue::Url("https://example.com".into())),
        }];
        records
    }

    /// The ledger row a successful apply of `item` writes
    fn ledger_after(item: &PlannedItem) -> LedgerEntry {
        let payload = item.payload.as_ref().unwrap();
        LedgerEntry {
            paperless_id: item.paperless_id,
            document_id: item.document_id,
            checksum: payload.checksum.value.clone(),
            fingerprint: payload.fingerprint.clone(),
            fingerprint_version: payload.fingerprint_version,
            needs_reapply: payload.needs_reapply(),
            trashed_in_paperless: payload.trashed,
            folio_state: FolioState::Live,
        }
    }

    #[test]
    fn a_new_document_gets_a_create_with_its_full_payload() {
        let item = Fixture::new().plan(&records(), None).unwrap();

        assert_eq!(item.outcome, ItemOutcome::Pending(ItemAction::Create));
        assert_eq!(item.document_id, document_id(VAULT, 1));
        let payload = item.payload.unwrap();
        assert_eq!(payload.tags, vec![3]);
        assert!(payload.in_inbox);
        assert_eq!(payload.values.values.len(), 1);
        assert_eq!(payload.values.notes.len(), 1);
        assert_eq!(payload.values.asn, Some(17));
    }

    #[test]
    fn a_re_plan_produces_an_identical_payload() {
        let fixture = Fixture::new();
        let mut rich = records();
        rich.document.tags = vec![3, 4, 7, 9];
        rich.notes = vec![note(1, 1, "one"), note(2, 1, "two"), note(3, 1, "three")];
        rich.fields.push(PaperlessCustomFieldInstance {
            document: 1,
            field: 8,
            value: Some(PaperlessFieldValue::Integer(5)),
        });
        rich.versions = vec![version(10, 1), version(11, 2), version(12, 3)];
        let mut shuffled = rich.clone();
        shuffled.document.tags.rotate_left(1);
        shuffled.notes.rotate_left(1);
        shuffled.fields.reverse();
        shuffled.versions.rotate_left(1);

        let first = serde_json::to_string(&fixture.plan(&rich, None)).unwrap();
        let second = serde_json::to_string(&fixture.plan(&shuffled, None)).unwrap();

        assert_eq!(first, second);
    }

    fn version(id: i64, index: i64) -> crate::ports::PaperlessDocument {
        let mut version = document(id);
        version.content = format!("version {index}");
        version.version_of = Some(crate::ports::PaperlessVersionInfo {
            root_document: 1,
            index: Some(index),
            label: None,
        });
        version
    }

    #[test]
    fn a_synced_document_is_skipped_as_unchanged() {
        let fixture = Fixture::new();
        let created = fixture.plan(&records(), None).unwrap();
        let again = fixture
            .plan(&records(), Some(&ledger_after(&created)))
            .unwrap();
        assert_eq!(again.outcome, ItemOutcome::Skipped(SkipReason::Unchanged));
        assert_eq!(again.payload, None);
    }

    #[test]
    fn a_tag_rename_changes_no_document_plan() {
        let tag = |id, name: &str, is_inbox_tag| PaperlessTag {
            id,
            name: name.into(),
            color: "#000000".into(),
            is_inbox_tag,
            parent: None,
        };
        let bills = ExistingEntity {
            id: Id::from_u128(3),
            name: "Bills".into(),
        };
        let mapping = TaxonomyMapping {
            paperless_id: 3,
            entity_id: bills.id,
            synced_name: "Bills".into(),
        };
        let vault = VaultTaxonomy {
            existing: std::slice::from_ref(&bills),
            mappings: std::slice::from_ref(&mapping),
        };
        let plan_with = |name: &str, ledger: Option<&LedgerEntry>| {
            let tags = plan_tags(
                &[tag(3, name, false), tag(4, "Inbox", true)],
                vault,
                SyncMode::Mirror,
            );
            let mut fixture = Fixture::new();
            fixture.inbox = tags.inbox_tags.iter().copied().collect();
            (tags, fixture.plan(&records(), ledger).unwrap())
        };

        let (_, synced) = plan_with("Bills", None);
        let (renamed, fresh) = plan_with("Invoices", None);
        let (_, resynced) = plan_with("Invoices", Some(&ledger_after(&synced)));

        assert_eq!(renamed.plan.renames.len(), 1);
        assert_eq!(fresh, synced);
        assert_eq!(
            resynced.outcome,
            ItemOutcome::Skipped(SkipReason::Unchanged)
        );
    }

    #[test]
    fn a_document_with_a_retryable_drop_is_planned_again() {
        let mut fixture = Fixture::new();
        fixture.asn_owners.insert(17, Id::from_u128(99));

        let created = fixture.plan(&records(), None).unwrap();
        let payload = created.payload.as_ref().unwrap();
        assert!(matches!(
            payload.values.warnings[..],
            [ItemWarning::AsnDropped { asn: 17, .. }]
        ));
        assert!(payload.needs_reapply());

        let next = fixture
            .plan(&records(), Some(&ledger_after(&created)))
            .unwrap();
        assert_eq!(
            next.outcome,
            ItemOutcome::Pending(ItemAction::Update {
                replace_file: false
            })
        );
    }

    #[test]
    fn a_value_that_can_never_be_held_is_not_retried() {
        let fixture = Fixture::new();
        let mut bad = records();
        bad.fields[0].value = Some(PaperlessFieldValue::Url("ftp://example.com".into()));

        let created = fixture.plan(&bad, None).unwrap();
        let payload = created.payload.as_ref().unwrap();
        assert_eq!(payload.values.warnings.len(), 1);
        assert!(!payload.needs_reapply());

        let next = fixture.plan(&bad, Some(&ledger_after(&created))).unwrap();
        assert_eq!(next.outcome, ItemOutcome::Skipped(SkipReason::Unchanged));
    }

    #[test]
    fn a_purged_document_is_never_re_created() {
        let fixture = Fixture::new();
        let created = fixture.plan(&records(), None).unwrap();
        let purged = LedgerEntry {
            folio_state: FolioState::PurgedInFolio,
            ..ledger_after(&created)
        };
        let mut changed = records();
        changed.document.title = "Changed in Paperless".into();

        let item = fixture.plan(&changed, Some(&purged)).unwrap();

        assert_eq!(
            item.outcome,
            ItemOutcome::Skipped(SkipReason::PurgedInFolio)
        );
        assert_eq!(item.payload, None);
    }

    #[test]
    fn new_only_writes_no_item_for_changes() {
        let fixture = Fixture::new();
        let created = fixture.plan(&records(), None).unwrap();
        let mut changed = records();
        changed.document.title = "Changed".into();
        let ledger = ledger_after(&created);
        assert_eq!(
            fixture.plan_in(SyncMode::NewOnly, &changed, Some(&ledger)),
            None
        );
    }
}

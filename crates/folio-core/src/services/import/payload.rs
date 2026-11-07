//! One planned item per Paperless document, with the self-contained payload
//! the apply phase needs

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::diff::{decide, Decision, ItemAction, LedgerEntry, SkipReason};
use super::fingerprint::{document_id, fingerprint, DocumentRecords, FINGERPRINT_VERSION};
use super::values::{plan_values, PlannedValues, ValueContext};
use super::SyncMode;
use crate::models::{Id, VaultId};
use crate::ports::PaperlessChecksum;

/// Everything the apply phase writes for one document, in Paperless terms;
/// taxonomy and fields are resolved through `paperless_taxonomy` at apply
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemPayload {
    pub fingerprint_version: u32,
    pub fingerprint: String,
    pub document_id: Id,
    pub paperless_id: i64,
    pub original_handle: String,
    pub checksum: PaperlessChecksum,
    pub size: Option<u64>,
    pub title: String,
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
    /// Sets `needs_reapply` on the ledger row, so the next sync retries the
    /// dropped values even if nothing changes in Paperless
    pub fn drops_values(&self) -> bool {
        !self.values.warnings.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum ItemOutcome {
    Pending(ItemAction),
    Skipped(SkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedItem {
    pub paperless_id: i64,
    pub document_id: Id,
    pub title: String,
    pub outcome: ItemOutcome,
    /// Present for items that will be applied
    pub payload: Option<ItemPayload>,
}

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
) -> Option<PlannedItem> {
    let root = &records.document;
    let original = records.original();
    let fingerprint = fingerprint(records, ctx.inbox_tags);
    let document_id = ledger.map_or_else(|| document_id(ctx.vault, root.id), |r| r.document_id);

    let outcome = match decide(root, original, &fingerprint, ledger, ctx.mode) {
        Decision::Apply(action) => ItemOutcome::Pending(action),
        Decision::Skip(reason) => ItemOutcome::Skipped(reason),
        Decision::Ignore => return None,
    };
    let payload = matches!(outcome, ItemOutcome::Pending(_)).then(|| {
        let tags: BTreeSet<i64> = root.tags.iter().copied().collect();
        let instances: Vec<_> = records
            .fields
            .iter()
            .map(|f| (f.field, f.value.as_ref()))
            .collect();
        ItemPayload {
            fingerprint_version: FINGERPRINT_VERSION,
            fingerprint,
            document_id,
            paperless_id: root.id,
            original_handle: original.original_handle.clone(),
            checksum: original.checksum.clone(),
            size: original.size,
            title: root.title.clone(),
            content: original.content.clone(),
            mime_type: original.mime_type.clone(),
            original_filename: original.original_filename.clone(),
            created: root.created,
            added: root.added,
            modified: root.modified,
            correspondent: root.correspondent,
            document_type: root.document_type,
            in_inbox: !tags.is_disjoint(ctx.inbox_tags),
            tags: tags.difference(ctx.inbox_tags).copied().collect(),
            trashed: root.trashed,
            skipped_versions: records.skipped_versions(),
            values: plan_values(
                document_id,
                &instances,
                &records.live_notes(),
                root.archive_serial_number,
                &ctx.values,
            ),
        }
    });
    Some(PlannedItem {
        paperless_id: root.id,
        document_id,
        title: root.title.clone(),
        outcome,
        payload,
    })
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
    use crate::services::import::fingerprint::tests::{document, note};
    use crate::services::import::taxonomy::{plan_tags, TaxonomyMapping, VaultTaxonomy};
    use crate::services::import::values::{ItemWarning, DEFAULT_CURRENCY};
    use crate::services::import::ExistingEntity;

    const VAULT: VaultId = VaultId(Id::from_u128(7));

    struct Fixture {
        inbox: BTreeSet<i64>,
        fields: HashMap<i64, PaperlessCustomField>,
        asn_owners: HashMap<i64, Id>,
    }

    impl Fixture {
        fn new() -> Self {
            let link = PaperlessCustomField {
                id: 1,
                name: "Link".into(),
                data_type: PaperlessFieldType::Url,
                select_options: vec![],
                default_currency: None,
            };
            Self {
                inbox: BTreeSet::from([4]),
                fields: HashMap::from([(1, link)]),
                asn_owners: HashMap::new(),
            }
        }

        fn plan(&self, records: &DocumentRecords, ledger: Option<&LedgerEntry>) -> Option<PlannedItem> {
            let ctx = PlanContext {
                vault: VAULT,
                mode: SyncMode::Mirror,
                inbox_tags: &self.inbox,
                values: ValueContext {
                    fields: &self.fields,
                    asn_owners: &self.asn_owners,
                    default_currency: DEFAULT_CURRENCY,
                },
            };
            plan_document(records, ledger, &ctx)
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
            paperless_modified: payload.modified,
            checksum: payload.checksum.value.clone(),
            fingerprint: payload.fingerprint.clone(),
            fingerprint_version: payload.fingerprint_version,
            needs_reapply: payload.drops_values(),
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
        let first = serde_json::to_string(&fixture.plan(&records(), None)).unwrap();
        let mut shuffled = records();
        shuffled.document.tags.reverse();
        let second = serde_json::to_string(&fixture.plan(&shuffled, None)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_synced_document_is_skipped_as_unchanged() {
        let fixture = Fixture::new();
        let created = fixture.plan(&records(), None).unwrap();
        let again = fixture.plan(&records(), Some(&ledger_after(&created))).unwrap();
        assert_eq!(again.outcome, ItemOutcome::Skipped(SkipReason::Unchanged));
        assert_eq!(again.payload, None);
    }

    #[test]
    fn a_tag_rename_changes_no_fingerprint() {
        let fixture = Fixture::new();
        let tag = |name: &str| PaperlessTag {
            id: 3,
            name: name.into(),
            color: "#000000".into(),
            is_inbox_tag: false,
            parent: None,
        };
        let synced = fixture.plan(&records(), None).unwrap();
        let ledger = ledger_after(&synced);

        let entity = ExistingEntity {
            id: Id::from_u128(3),
            name: "Bills".into(),
        };
        let mapping = TaxonomyMapping {
            paperless_id: 3,
            entity_id: entity.id,
            synced_name: "Bills".into(),
        };
        let vault = VaultTaxonomy {
            existing: std::slice::from_ref(&entity),
            mappings: std::slice::from_ref(&mapping),
        };
        let tags = plan_tags(&[tag("Invoices")], vault, SyncMode::Mirror);
        assert_eq!(tags.plan.renames.len(), 1);

        let after = fixture.plan(&records(), Some(&ledger)).unwrap();
        assert_eq!(after.outcome, ItemOutcome::Skipped(SkipReason::Unchanged));
    }

    #[test]
    fn a_document_with_a_dropped_value_is_planned_again() {
        let fixture = Fixture::new();
        let mut bad = records();
        bad.fields[0].value = Some(PaperlessFieldValue::Url("ftp://example.com".into()));

        let created = fixture.plan(&bad, None).unwrap();
        let payload = created.payload.as_ref().unwrap();
        assert!(matches!(
            payload.values.warnings[..],
            [ItemWarning::ValueDropped { field_id: 1, .. }]
        ));
        assert!(payload.drops_values());

        let next = fixture.plan(&bad, Some(&ledger_after(&created))).unwrap();
        assert_eq!(
            next.outcome,
            ItemOutcome::Pending(ItemAction::Update {
                replace_file: false
            })
        );
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

        assert_eq!(item.outcome, ItemOutcome::Skipped(SkipReason::PurgedInFolio));
        assert_eq!(item.payload, None);
    }

    #[test]
    fn new_only_writes_no_item_for_changes() {
        let fixture = Fixture::new();
        let created = fixture.plan(&records(), None).unwrap();
        let mut changed = records();
        changed.document.title = "Changed".into();
        let ctx = PlanContext {
            vault: VAULT,
            mode: SyncMode::NewOnly,
            inbox_tags: &fixture.inbox,
            values: ValueContext {
                fields: &fixture.fields,
                asn_owners: &fixture.asn_owners,
                default_currency: DEFAULT_CURRENCY,
            },
        };
        assert_eq!(
            plan_document(&changed, Some(&ledger_after(&created)), &ctx),
            None
        );
    }
}

//! Custom fields: how Paperless types map onto Folio metadata kinds, which
//! fields can't be imported safely, and where each one lands

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use super::taxonomy::Target;
use crate::models::{metadata_name_key, Id, MetadataFieldInput, MetadataKind, MetadataOptionInput};
use crate::ports::{PaperlessCustomField, PaperlessFieldType};

/// The unique integer field that holds Paperless archive serial numbers
pub const ASN_FIELD_NAME: &str = "ASN";

/// A live metadata field already in the vault
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingField {
    pub id: Id,
    pub name: String,
    pub kind: MetadataKind,
    pub unique: bool,
    /// Labels of the live options of a select field
    pub options: Vec<String>,
}

/// A field the import can't write into; any conflict fails the run before
/// anything is written
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldConflict {
    /// The field's name in the vault, or in Paperless when it has none yet
    pub field: String,
    /// The Paperless field; `None` for the vault's ASN field
    pub paperless_id: Option<i64>,
    pub reason: FieldConflictReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldConflictReason {
    /// The existing field's kind can't hold the values
    KindMismatch {
        existing: MetadataKind,
        required: MetadataKind,
    },
    /// Two Paperless fields with the same name key need different kinds
    PaperlessKindClash {
        other_paperless_id: i64,
        kind: MetadataKind,
        other_kind: MetadataKind,
    },
    /// The existing field is unique, so imported values could collide
    UniqueField,
    /// The vault's ASN field exists but isn't unique
    AsnNotUnique,
    /// A Paperless field named like the ASN field, which archive serial
    /// numbers are written into
    ReservedAsnName,
    /// The name isn't a valid field name
    InvalidName,
}

/// The Folio kind a Paperless custom-field type is imported as
pub fn folio_kind(data_type: PaperlessFieldType) -> MetadataKind {
    match data_type {
        PaperlessFieldType::String => MetadataKind::Text,
        PaperlessFieldType::LongText => MetadataKind::LongText,
        PaperlessFieldType::Url => MetadataKind::Url,
        PaperlessFieldType::Date => MetadataKind::Date,
        PaperlessFieldType::Boolean => MetadataKind::Boolean,
        PaperlessFieldType::Integer => MetadataKind::Integer,
        PaperlessFieldType::Float => MetadataKind::Decimal,
        PaperlessFieldType::Monetary => MetadataKind::Money,
        PaperlessFieldType::Select => MetadataKind::Select,
        PaperlessFieldType::DocumentLink => MetadataKind::DocumentLink,
    }
}

/// Paperless names are trimmed, as field creation trims them, before keying
fn name_key(name: &str) -> String {
    metadata_name_key(name.trim())
}

/// Every reason the Paperless fields can't be imported into this vault
pub fn field_conflicts(
    paperless: &[PaperlessCustomField],
    existing: &[ExistingField],
) -> Vec<FieldConflict> {
    let existing: HashMap<String, &ExistingField> =
        existing.iter().map(|f| (name_key(&f.name), f)).collect();
    let asn_key = name_key(ASN_FIELD_NAME);
    let mut sorted: Vec<&PaperlessCustomField> = paperless.iter().collect();
    sorted.sort_by_key(|f| f.id);

    let mut conflicts = Vec::new();
    let mut first_by_key: HashMap<String, &PaperlessCustomField> = HashMap::new();
    for field in sorted {
        let kind = folio_kind(field.data_type);
        let conflict = |reason| FieldConflict {
            field: field.name.clone(),
            paperless_id: Some(field.id),
            reason,
        };
        if valid_field_name(&field.name, kind).is_none() {
            conflicts.push(conflict(FieldConflictReason::InvalidName));
            continue;
        }
        let key = name_key(&field.name);
        if key == asn_key {
            conflicts.push(conflict(FieldConflictReason::ReservedAsnName));
            continue;
        }
        let first = *first_by_key.entry(key.clone()).or_insert(field);
        let first_kind = folio_kind(first.data_type);
        if first_kind != kind {
            conflicts.push(conflict(FieldConflictReason::PaperlessKindClash {
                other_paperless_id: first.id,
                kind,
                other_kind: first_kind,
            }));
        }
        let reason = match existing.get(&key) {
            Some(found) if found.kind != kind => Some(FieldConflictReason::KindMismatch {
                existing: found.kind,
                required: kind,
            }),
            Some(found) if found.unique => Some(FieldConflictReason::UniqueField),
            _ => None,
        };
        if let Some(reason) = reason {
            conflicts.push(FieldConflict {
                field: existing[&key].name.clone(),
                paperless_id: Some(field.id),
                reason,
            });
        }
    }
    if let Some(asn) = existing.get(&asn_key) {
        let reason = if asn.kind != MetadataKind::Integer {
            Some(FieldConflictReason::KindMismatch {
                existing: asn.kind,
                required: MetadataKind::Integer,
            })
        } else if !asn.unique {
            Some(FieldConflictReason::AsnNotUnique)
        } else {
            None
        };
        if let Some(reason) = reason {
            conflicts.push(FieldConflict {
                field: asn.name.clone(),
                paperless_id: None,
                reason,
            });
        }
    }
    conflicts
}

fn valid_field_name(name: &str, kind: MetadataKind) -> Option<String> {
    let input = MetadataFieldInput {
        name: name.to_owned(),
        kind,
        required: false,
        unique: false,
    };
    input.validate().ok().map(|input| input.name)
}

/// Trimmed and validated like options entered in Folio; invalid labels are
/// left out, so values that use them are dropped at apply
pub(super) fn valid_option_label(label: &str) -> Option<String> {
    let input = MetadataOptionInput {
        label: label.to_owned(),
        position: 0,
    };
    input.validate().ok().map(|input| input.label)
}

/// Where each Paperless custom field lands, and the fields and select options
/// to create first
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FieldPlan {
    /// Paperless field ID to target, in ID order
    pub fields: BTreeMap<i64, FieldTarget>,
    pub creates: Vec<NewField>,
    /// Labels to add to existing select fields, in Paperless order
    pub missing_options: BTreeMap<Id, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldTarget {
    pub kind: MetadataKind,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewField {
    pub name: String,
    pub kind: MetadataKind,
    /// Select option labels, in Paperless order
    pub options: Vec<String>,
}

/// Maps each field onto an existing field with the same name key, or a new
/// one. Run only once [`field_conflicts`] came back empty.
pub fn plan_fields(paperless: &[PaperlessCustomField], existing: &[ExistingField]) -> FieldPlan {
    let mut sorted: Vec<&PaperlessCustomField> = paperless.iter().collect();
    sorted.sort_by_key(|f| f.id);
    let existing_by_id: HashMap<Id, &ExistingField> = existing.iter().map(|f| (f.id, f)).collect();
    let mut taken: HashMap<String, Target> = existing
        .iter()
        .map(|f| (name_key(&f.name), Target::Existing(f.id)))
        .collect();

    let mut plan = FieldPlan::default();
    for field in sorted {
        let kind = folio_kind(field.data_type);
        let target = *taken.entry(name_key(&field.name)).or_insert_with(|| {
            plan.creates.push(NewField {
                name: field.name.trim().to_owned(),
                kind,
                options: Vec::new(),
            });
            Target::New(plan.creates.len() - 1)
        });
        plan.fields.insert(field.id, FieldTarget { kind, target });
        if kind != MetadataKind::Select {
            continue;
        }
        let labels = field
            .select_options
            .iter()
            .filter_map(|option| valid_option_label(&option.label));
        let (have, add): (&[String], &mut Vec<String>) = match target {
            Target::New(index) => (&[], &mut plan.creates[index].options),
            Target::Existing(id) => (
                &existing_by_id[&id].options,
                plan.missing_options.entry(id).or_default(),
            ),
        };
        for label in labels {
            if !have.contains(&label) && !add.contains(&label) {
                add.push(label);
            }
        }
    }
    plan.missing_options.retain(|_, labels| !labels.is_empty());
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::PaperlessSelectOption;
    use crate::services::import::test_support::custom_field as paperless;

    fn existing(name: &str, kind: MetadataKind) -> ExistingField {
        ExistingField {
            id: Id::new_v4(),
            name: name.into(),
            kind,
            unique: false,
            options: vec![],
        }
    }

    fn reasons(conflicts: &[FieldConflict]) -> Vec<FieldConflictReason> {
        conflicts.iter().map(|c| c.reason).collect()
    }

    fn select(id: i64, name: &str, labels: &[&str]) -> PaperlessCustomField {
        let mut field = paperless(id, name, PaperlessFieldType::Select);
        field.select_options = labels
            .iter()
            .map(|label| PaperlessSelectOption {
                id: Some(format!("opt-{label}")),
                label: (*label).into(),
            })
            .collect();
        field
    }

    #[test]
    fn monetary_field_conflicts_with_existing_text_field_of_the_same_name() {
        let conflicts = field_conflicts(
            &[paperless(7, "Amount", PaperlessFieldType::Monetary)],
            &[existing("Amount", MetadataKind::Text)],
        );

        assert_eq!(
            conflicts,
            vec![FieldConflict {
                field: "Amount".into(),
                paperless_id: Some(7),
                reason: FieldConflictReason::KindMismatch {
                    existing: MetadataKind::Text,
                    required: MetadataKind::Money,
                },
            }]
        );
    }

    #[test]
    fn names_match_case_insensitively_like_the_vault_index() {
        let conflicts = field_conflicts(
            &[paperless(1, " amount ", PaperlessFieldType::Integer)],
            &[existing("Amount", MetadataKind::Text)],
        );
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn matching_kinds_and_new_fields_are_not_conflicts() {
        let conflicts = field_conflicts(
            &[
                paperless(1, "Amount", PaperlessFieldType::Monetary),
                paperless(2, "Due", PaperlessFieldType::Date),
                paperless(3, "due", PaperlessFieldType::Date),
            ],
            &[existing("Amount", MetadataKind::Money)],
        );
        assert!(conflicts.is_empty());
    }

    #[test]
    fn paperless_fields_sharing_a_name_key_must_share_a_kind() {
        let conflicts = field_conflicts(
            &[
                paperless(4, "total", PaperlessFieldType::Integer),
                paperless(2, "Total", PaperlessFieldType::Monetary),
            ],
            &[],
        );
        assert_eq!(
            reasons(&conflicts),
            vec![FieldConflictReason::PaperlessKindClash {
                other_paperless_id: 2,
                kind: MetadataKind::Integer,
                other_kind: MetadataKind::Money,
            }]
        );
    }

    #[test]
    fn unique_existing_fields_are_conflicts() {
        let mut invoice = existing("Invoice no", MetadataKind::Text);
        invoice.unique = true;
        let conflicts = field_conflicts(
            &[paperless(1, "Invoice no", PaperlessFieldType::String)],
            &[invoice],
        );
        assert_eq!(reasons(&conflicts), vec![FieldConflictReason::UniqueField]);
    }

    #[test]
    fn a_paperless_field_named_asn_is_a_conflict() {
        let conflicts = field_conflicts(&[paperless(1, "asn", PaperlessFieldType::Integer)], &[]);
        assert_eq!(
            reasons(&conflicts),
            vec![FieldConflictReason::ReservedAsnName]
        );
    }

    #[test]
    fn invalid_field_names_are_conflicts() {
        let long = "x".repeat(129);
        let conflicts = field_conflicts(
            &[
                paperless(1, "  ", PaperlessFieldType::String),
                paperless(2, &long, PaperlessFieldType::String),
            ],
            &[],
        );
        assert_eq!(
            reasons(&conflicts),
            vec![
                FieldConflictReason::InvalidName,
                FieldConflictReason::InvalidName
            ]
        );
    }

    #[test]
    fn asn_field_must_be_a_unique_integer() {
        let text = field_conflicts(&[], &[existing("asn", MetadataKind::Text)]);
        assert_eq!(
            reasons(&text),
            vec![FieldConflictReason::KindMismatch {
                existing: MetadataKind::Text,
                required: MetadataKind::Integer
            }]
        );

        let not_unique = field_conflicts(&[], &[existing("ASN", MetadataKind::Integer)]);
        assert_eq!(
            reasons(&not_unique),
            vec![FieldConflictReason::AsnNotUnique]
        );

        let mut ok = existing("ASN", MetadataKind::Integer);
        ok.unique = true;
        assert!(field_conflicts(&[], &[ok]).is_empty());
    }

    #[test]
    fn fields_map_onto_existing_fields_by_name_key_or_are_created() {
        let amount = existing("Amount", MetadataKind::Money);

        let plan = plan_fields(
            &[
                paperless(2, "amount", PaperlessFieldType::Monetary),
                paperless(1, " Due ", PaperlessFieldType::Date),
                paperless(3, "due", PaperlessFieldType::Date),
            ],
            std::slice::from_ref(&amount),
        );

        assert_eq!(plan.fields[&2].target, Target::Existing(amount.id));
        assert_eq!(plan.fields[&1].target, Target::New(0));
        assert_eq!(plan.fields[&3].target, Target::New(0));
        assert_eq!(
            plan.creates,
            vec![NewField {
                name: "Due".into(),
                kind: MetadataKind::Date,
                options: vec![],
            }]
        );
    }

    #[test]
    fn select_options_are_planned_in_paperless_order() {
        let mut status = existing("Status", MetadataKind::Select);
        status.options = vec!["Open".into()];

        let plan = plan_fields(
            &[
                select(1, "Status", &["Open", "Paid", " Overdue "]),
                select(2, "Priority", &["High", "", "Low"]),
                select(3, "priority", &["Low", "Urgent"]),
            ],
            std::slice::from_ref(&status),
        );

        assert_eq!(
            plan.missing_options,
            BTreeMap::from([(status.id, vec!["Paid".to_string(), "Overdue".to_string()])])
        );
        assert_eq!(plan.creates[0].options, vec!["High", "Low", "Urgent"]);
    }

    #[test]
    fn every_paperless_type_has_the_designed_kind() {
        use MetadataKind as K;
        use PaperlessFieldType as P;
        let table = [
            (P::String, K::Text),
            (P::LongText, K::LongText),
            (P::Url, K::Url),
            (P::Date, K::Date),
            (P::Boolean, K::Boolean),
            (P::Integer, K::Integer),
            (P::Float, K::Decimal),
            (P::Monetary, K::Money),
            (P::Select, K::Select),
            (P::DocumentLink, K::DocumentLink),
        ];
        for (paperless, kind) in table {
            assert_eq!(folio_kind(paperless), kind, "{paperless:?}");
        }
    }
}

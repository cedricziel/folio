//! Custom-field kinds: how Paperless types map onto Folio metadata kinds, and
//! which existing vault fields can't hold the values an import would write

use std::collections::HashMap;

use serde::Serialize;

use crate::models::{metadata_name_key, Id, MetadataKind};
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
}

/// An existing field that an import would have to write into but can't
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldKindConflict {
    /// The field's name in the vault
    pub field: String,
    /// The Paperless field it collides with; `None` for the ASN field
    pub paperless_id: Option<i64>,
    pub existing_kind: MetadataKind,
    pub required_kind: MetadataKind,
    pub reason: FieldConflictReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldConflictReason {
    KindMismatch,
    /// The ASN field exists but isn't unique, so collisions can't be enforced
    AsnNotUnique,
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
pub(crate) fn name_key(name: &str) -> String {
    metadata_name_key(name.trim())
}

/// Every existing field, matched by name, whose kind can't hold the values of
/// the Paperless field (or the ASN) that would be written into it. A
/// non-empty result fails the run before anything is written.
pub fn field_kind_conflicts(
    paperless: &[PaperlessCustomField],
    existing: &[ExistingField],
) -> Vec<FieldKindConflict> {
    let by_key: HashMap<String, &ExistingField> =
        existing.iter().map(|f| (name_key(&f.name), f)).collect();
    let by_key = |name: &str| by_key.get(&name_key(name)).copied();
    let mut sorted: Vec<&PaperlessCustomField> = paperless.iter().collect();
    sorted.sort_by_key(|f| f.id);

    let mut conflicts = Vec::new();
    for field in sorted {
        let required = folio_kind(field.data_type);
        if let Some(found) = by_key(&field.name).filter(|f| f.kind != required) {
            conflicts.push(FieldKindConflict {
                field: found.name.clone(),
                paperless_id: Some(field.id),
                existing_kind: found.kind,
                required_kind: required,
                reason: FieldConflictReason::KindMismatch,
            });
        }
    }
    if let Some(asn) = by_key(ASN_FIELD_NAME) {
        let reason = if asn.kind != MetadataKind::Integer {
            Some(FieldConflictReason::KindMismatch)
        } else if !asn.unique {
            Some(FieldConflictReason::AsnNotUnique)
        } else {
            None
        };
        if let Some(reason) = reason {
            conflicts.push(FieldKindConflict {
                field: asn.name.clone(),
                paperless_id: None,
                existing_kind: asn.kind,
                required_kind: MetadataKind::Integer,
                reason,
            });
        }
    }
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::import::test_support::custom_field as paperless;

    fn existing(name: &str, kind: MetadataKind) -> ExistingField {
        ExistingField {
            id: Id::new_v4(),
            name: name.into(),
            kind,
            unique: false,
        }
    }

    #[test]
    fn monetary_field_conflicts_with_existing_text_field_of_the_same_name() {
        let conflicts = field_kind_conflicts(
            &[paperless(7, "Amount", PaperlessFieldType::Monetary)],
            &[existing("Amount", MetadataKind::Text)],
        );

        assert_eq!(
            conflicts,
            vec![FieldKindConflict {
                field: "Amount".into(),
                paperless_id: Some(7),
                existing_kind: MetadataKind::Text,
                required_kind: MetadataKind::Money,
                reason: FieldConflictReason::KindMismatch,
            }]
        );
    }

    #[test]
    fn names_match_case_insensitively_like_the_vault_index() {
        let conflicts = field_kind_conflicts(
            &[paperless(1, "amount", PaperlessFieldType::Integer)],
            &[existing("Amount", MetadataKind::Text)],
        );
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn matching_kinds_and_new_fields_are_not_conflicts() {
        let conflicts = field_kind_conflicts(
            &[
                paperless(1, "Amount", PaperlessFieldType::Monetary),
                paperless(2, "Due", PaperlessFieldType::Date),
            ],
            &[existing("Amount", MetadataKind::Money)],
        );
        assert!(conflicts.is_empty());
    }

    #[test]
    fn asn_field_must_be_a_unique_integer() {
        let text = field_kind_conflicts(&[], &[existing("asn", MetadataKind::Text)]);
        assert_eq!(text[0].reason, FieldConflictReason::KindMismatch);

        let not_unique = field_kind_conflicts(&[], &[existing("ASN", MetadataKind::Integer)]);
        assert_eq!(not_unique[0].reason, FieldConflictReason::AsnNotUnique);

        let mut ok = existing("ASN", MetadataKind::Integer);
        ok.unique = true;
        assert!(field_kind_conflicts(&[], &[ok]).is_empty());
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

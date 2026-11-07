//! Pre-validation of a document's custom-field values, notes and ASN
//!
//! Values go through the same validators the metadata repository runs, so
//! the apply transaction never fails on one. A value Folio can't hold is
//! dropped with a warning; the document is still imported.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::fields::{folio_kind, valid_option_label};
use crate::models::{EntryBody, Id, MetadataKind, MetadataValue};
use crate::ports::{
    PaperlessCustomField, PaperlessCustomFieldInstance, PaperlessFieldType, PaperlessFieldValue,
    PaperlessNote,
};
use crate::Error;

/// Used when neither the value nor its field names a currency
pub const DEFAULT_CURRENCY: &str = "EUR";

/// A validated value, keyed by its Paperless field
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlannedValue {
    Value {
        value: MetadataValue,
    },
    /// Select values carry the option label; apply resolves the option
    Option {
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedNote {
    pub external_id: String,
    pub body: String,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemWarning {
    ValueDropped {
        field_id: i64,
        field: String,
        reason: String,
        /// The drop may not happen again, e.g. once the field is known
        retryable: bool,
    },
    NoteDropped {
        note_id: i64,
        reason: String,
    },
    /// Another document holds the ASN; it may give it up
    AsnDropped {
        asn: i64,
        reason: String,
    },
    /// A linked document the vault doesn't hold yet; a later sync may
    LinkDropped {
        field_id: i64,
        paperless_id: i64,
    },
}

impl ItemWarning {
    /// Whether a later sync could apply what was dropped, even when nothing
    /// changes in Paperless; such drops set `needs_reapply` on the ledger
    pub fn retryable(&self) -> bool {
        match self {
            Self::ValueDropped { retryable, .. } => *retryable,
            Self::NoteDropped { .. } => false,
            Self::AsnDropped { .. } | Self::LinkDropped { .. } => true,
        }
    }
}

/// Values, notes and ASN that will be written, plus what was dropped
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedValues {
    pub values: BTreeMap<i64, PlannedValue>,
    /// Raw Paperless document IDs per link field; resolved through the ledger at apply
    pub links: BTreeMap<i64, Vec<i64>>,
    pub asn: Option<i64>,
    pub notes: Vec<PlannedNote>,
    pub warnings: Vec<ItemWarning>,
}

/// What the vault already holds that values must not collide with
pub struct ValueContext<'a> {
    pub fields: &'a HashMap<i64, PaperlessCustomField>,
    /// ASN to the document that holds it
    pub asn_owners: &'a HashMap<i64, Id>,
    /// The run's or source's fallback currency
    pub default_currency: &'a str,
}

/// Validates a document's values. `notes` are the live notes in ID order.
pub fn plan_values(
    document_id: Id,
    instances: &[PaperlessCustomFieldInstance],
    notes: &[&PaperlessNote],
    asn: Option<i64>,
    ctx: &ValueContext<'_>,
) -> PlannedValues {
    let mut planned = PlannedValues::default();
    let mut by_field: BTreeMap<i64, Vec<&PaperlessCustomFieldInstance>> = BTreeMap::new();
    for instance in instances {
        by_field.entry(instance.field).or_default().push(instance);
    }

    for (field_id, instances) in by_field {
        let Some(field) = ctx.fields.get(&field_id) else {
            planned.warnings.push(ItemWarning::ValueDropped {
                field_id,
                field: format!("#{field_id}"),
                reason: "Unknown custom field".into(),
                retryable: true,
            });
            continue;
        };
        let dropped = |reason: String| ItemWarning::ValueDropped {
            field_id,
            field: field.name.clone(),
            reason,
            retryable: false,
        };
        // No order-independent way to pick one
        let [instance] = instances[..] else {
            planned
                .warnings
                .push(dropped("The field has more than one value".into()));
            continue;
        };
        let Some(value) = &instance.value else {
            continue;
        };
        match (value, field.data_type) {
            (PaperlessFieldValue::DocumentLink(ids), PaperlessFieldType::DocumentLink) => {
                if !ids.is_empty() {
                    planned.links.insert(field_id, ids.clone());
                }
            }
            (PaperlessFieldValue::DocumentLink(_), _) => planned
                .warnings
                .push(dropped("Value kind does not match the field".into())),
            (value, _) => match convert(field, value, ctx.default_currency) {
                Ok(Some(value)) => {
                    planned.values.insert(field_id, value);
                }
                Ok(None) => {}
                Err(reason) => planned.warnings.push(dropped(reason)),
            },
        }
    }

    for note in notes {
        match EntryBody::parse(note.body.clone()) {
            Ok(body) => planned.notes.push(PlannedNote {
                external_id: note.id.to_string(),
                body: body.into_inner(),
                created: note.created,
            }),
            Err(error) => planned.warnings.push(ItemWarning::NoteDropped {
                note_id: note.id,
                reason: reason(error),
            }),
        }
    }

    planned.asn = asn.filter(|asn| match ctx.asn_owners.get(asn) {
        Some(owner) if *owner != document_id => {
            planned.warnings.push(ItemWarning::AsnDropped {
                asn: *asn,
                reason: "Another document in the vault already has this ASN".into(),
            });
            false
        }
        _ => true,
    });
    planned
}

fn convert(
    field: &PaperlessCustomField,
    value: &PaperlessFieldValue,
    default_currency: &str,
) -> Result<Option<PlannedValue>, String> {
    use PaperlessFieldValue as P;
    let kind = folio_kind(field.data_type);
    let metadata = match value {
        P::Text(text) => MetadataValue::Text(text.clone()),
        P::LongText(text) => MetadataValue::LongText(text.clone()),
        P::Url(url) => MetadataValue::Url(url.clone()),
        P::Date(date) => MetadataValue::Date(date.format("%Y-%m-%d").to_string()),
        P::Boolean(flag) => MetadataValue::Boolean(*flag),
        P::Integer(number) => MetadataValue::Integer(number.to_string()),
        // Display is the shortest form that round-trips, without an exponent
        P::Float(number) => MetadataValue::Decimal(number.to_string()),
        P::Monetary(raw) => {
            let Some((currency, amount)) = split_monetary(raw) else {
                return Ok(None);
            };
            let currency = currency
                .or(field.default_currency.as_deref())
                .unwrap_or(default_currency)
                .to_ascii_uppercase();
            MetadataValue::Money {
                amount: amount.to_owned(),
                currency,
            }
        }
        P::Select(raw) => {
            if kind != MetadataKind::Select {
                return Err("Value kind does not match the field".into());
            }
            return select_label(field, raw)
                .map(|label| Some(PlannedValue::Option { label }))
                .ok_or_else(|| "Unknown select option".into());
        }
        P::DocumentLink(_) => unreachable!("plan_values keeps links raw"),
    };
    kind.validate_value(Some(metadata), false)
        .map(|value| value.map(|value| PlannedValue::Value { value }))
        .map_err(reason)
}

/// Splits Paperless's `EUR123.45` form into the currency and the amount
/// text; `None` for an empty value
fn split_monetary(raw: &str) -> Option<(Option<&str>, &str)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (currency, amount) = match raw.get(..3) {
        Some(prefix) if prefix.bytes().all(|b| b.is_ascii_alphabetic()) => {
            (Some(prefix), &raw[3..])
        }
        _ => (None, raw),
    };
    Some((currency, amount))
}

fn select_label(field: &PaperlessCustomField, raw: &str) -> Option<String> {
    let by_id = field
        .select_options
        .iter()
        .find(|option| option.id.as_deref() == Some(raw));
    let by_index = || {
        raw.parse::<usize>()
            .ok()
            .and_then(|index| field.select_options.get(index))
            .filter(|option| option.id.is_none())
    };
    by_id
        .or_else(by_index)
        .and_then(|option| valid_option_label(&option.label))
}

fn reason(error: Error) -> String {
    match error {
        Error::Validation(message) => message,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::PaperlessSelectOption;
    use crate::services::import::test_support::{custom_field as field, note};

    struct Fixture {
        fields: HashMap<i64, PaperlessCustomField>,
        asn_owners: HashMap<i64, Id>,
    }

    impl Fixture {
        fn new(fields: Vec<PaperlessCustomField>) -> Self {
            Self {
                fields: fields.into_iter().map(|f| (f.id, f)).collect(),
                asn_owners: HashMap::new(),
            }
        }

        fn plan(
            &self,
            values: &[(i64, PaperlessFieldValue)],
            notes: &[PaperlessNote],
            asn: Option<i64>,
        ) -> PlannedValues {
            let instances: Vec<_> = values
                .iter()
                .map(|(field, value)| PaperlessCustomFieldInstance {
                    document: 1,
                    field: *field,
                    value: Some(value.clone()),
                })
                .collect();
            let notes: Vec<_> = notes.iter().collect();
            let ctx = ValueContext {
                fields: &self.fields,
                asn_owners: &self.asn_owners,
                default_currency: DEFAULT_CURRENCY,
            };
            plan_values(Id::from_u128(1), &instances, &notes, asn, &ctx)
        }
    }

    fn value(planned: &PlannedValues, field: i64) -> Option<&MetadataValue> {
        match planned.values.get(&field) {
            Some(PlannedValue::Value { value }) => Some(value),
            _ => None,
        }
    }

    #[test]
    fn an_ftp_url_is_dropped_with_a_warning() {
        let fixture = Fixture::new(vec![field(1, "Link", PaperlessFieldType::Url)]);

        let planned = fixture.plan(
            &[(1, PaperlessFieldValue::Url("ftp://example.com/x".into()))],
            &[],
            None,
        );

        assert!(planned.values.is_empty());
        assert!(matches!(
            &planned.warnings[..],
            [ItemWarning::ValueDropped { field_id: 1, field, .. }] if field == "Link"
        ));
    }

    #[test]
    fn http_urls_are_kept() {
        let fixture = Fixture::new(vec![field(1, "Link", PaperlessFieldType::Url)]);
        let planned = fixture.plan(
            &[(1, PaperlessFieldValue::Url("https://example.com/x".into()))],
            &[],
            None,
        );
        assert_eq!(
            value(&planned, 1),
            Some(&MetadataValue::Url("https://example.com/x".into()))
        );
        assert!(planned.warnings.is_empty());
    }

    #[test]
    fn a_20_kb_note_and_a_blank_note_are_dropped_with_warnings() {
        let fixture = Fixture::new(vec![]);
        let long = "x".repeat(20 * 1024);

        let planned = fixture.plan(
            &[],
            &[note(1, 1, &long), note(2, 1, "  "), note(3, 1, "kept")],
            None,
        );

        assert_eq!(
            planned.notes,
            vec![PlannedNote {
                external_id: "3".into(),
                body: "kept".into(),
                created: note(3, 1, "").created,
            }]
        );
        assert!(matches!(
            &planned.warnings[..],
            [
                ItemWarning::NoteDropped { note_id: 1, .. },
                ItemWarning::NoteDropped { note_id: 2, .. }
            ]
        ));
    }

    #[test]
    fn a_duplicate_asn_is_dropped_with_a_warning() {
        let mut fixture = Fixture::new(vec![]);
        fixture.asn_owners.insert(17, Id::from_u128(2));
        fixture.asn_owners.insert(18, Id::from_u128(1));

        let taken = fixture.plan(&[], &[], Some(17));
        assert_eq!(taken.asn, None);
        assert!(matches!(
            &taken.warnings[..],
            [ItemWarning::AsnDropped { asn: 17, .. }]
        ));

        let own = fixture.plan(&[], &[], Some(18));
        assert_eq!(own.asn, Some(18));
        assert!(own.warnings.is_empty());
    }

    #[test]
    fn monetary_values_are_parsed_from_the_paperless_string() {
        let mut with_default = field(2, "Fee", PaperlessFieldType::Monetary);
        with_default.default_currency = Some("USD".into());
        let fixture = Fixture::new(vec![
            field(1, "Amount", PaperlessFieldType::Monetary),
            with_default,
        ]);

        let planned = fixture.plan(
            &[
                (1, PaperlessFieldValue::Monetary("CHF0.10".into())),
                (2, PaperlessFieldValue::Monetary("12.50".into())),
            ],
            &[],
            None,
        );

        assert_eq!(
            value(&planned, 1),
            Some(&MetadataValue::Money {
                amount: "0.1".into(),
                currency: "CHF".into()
            })
        );
        assert_eq!(
            value(&planned, 2),
            Some(&MetadataValue::Money {
                amount: "12.5".into(),
                currency: "USD".into()
            })
        );

        let fallback = fixture.plan(
            &[(
                1,
                PaperlessFieldValue::Monetary("99999999999999999999.99".into()),
            )],
            &[],
            None,
        );
        assert_eq!(
            value(&fallback, 1),
            Some(&MetadataValue::Money {
                amount: "99999999999999999999.99".into(),
                currency: "EUR".into()
            })
        );
    }

    #[test]
    fn empty_monetary_values_are_skipped_and_garbage_warns() {
        let fixture = Fixture::new(vec![field(1, "Amount", PaperlessFieldType::Monetary)]);
        let empty = fixture.plan(
            &[(1, PaperlessFieldValue::Monetary(String::new()))],
            &[],
            None,
        );
        assert!(empty.values.is_empty() && empty.warnings.is_empty());

        let garbage = fixture.plan(
            &[(1, PaperlessFieldValue::Monetary("EUR1e5".into()))],
            &[],
            None,
        );
        assert_eq!(garbage.warnings.len(), 1);
    }

    #[test]
    fn floats_use_the_shortest_round_trip_form() {
        let fixture = Fixture::new(vec![field(1, "Rate", PaperlessFieldType::Float)]);
        let planned = fixture.plan(&[(1, PaperlessFieldValue::Float(0.1))], &[], None);
        assert_eq!(
            value(&planned, 1),
            Some(&MetadataValue::Decimal("0.1".into()))
        );

        let nan = fixture.plan(&[(1, PaperlessFieldValue::Float(f64::NAN))], &[], None);
        assert_eq!(nan.warnings.len(), 1);
    }

    #[test]
    fn select_values_map_to_option_labels_by_id_or_legacy_index() {
        let mut with_ids = field(1, "Status", PaperlessFieldType::Select);
        with_ids.select_options = vec![
            PaperlessSelectOption {
                id: Some("abc".into()),
                label: "Open".into(),
            },
            PaperlessSelectOption {
                id: Some("def".into()),
                label: "Paid".into(),
            },
        ];
        let mut legacy = field(2, "Legacy", PaperlessFieldType::Select);
        legacy.select_options = vec![
            PaperlessSelectOption {
                id: None,
                label: "First".into(),
            },
            PaperlessSelectOption {
                id: None,
                label: "Second".into(),
            },
        ];
        let fixture = Fixture::new(vec![with_ids, legacy]);

        let planned = fixture.plan(
            &[
                (1, PaperlessFieldValue::Select("def".into())),
                (2, PaperlessFieldValue::Select("1".into())),
            ],
            &[],
            None,
        );
        assert_eq!(
            planned.values[&1],
            PlannedValue::Option {
                label: "Paid".into()
            }
        );
        assert_eq!(
            planned.values[&2],
            PlannedValue::Option {
                label: "Second".into()
            }
        );

        let unknown = fixture.plan(&[(1, PaperlessFieldValue::Select("zzz".into()))], &[], None);
        assert_eq!(unknown.warnings.len(), 1);
    }

    #[test]
    fn overlong_text_and_mismatched_kinds_are_dropped() {
        let fixture = Fixture::new(vec![
            field(1, "Name", PaperlessFieldType::String),
            field(2, "Count", PaperlessFieldType::Integer),
        ]);
        let planned = fixture.plan(
            &[
                (1, PaperlessFieldValue::Text("x".repeat(1_025))),
                (2, PaperlessFieldValue::Text("seven".into())),
            ],
            &[],
            None,
        );
        assert!(planned.values.is_empty());
        assert_eq!(planned.warnings.len(), 2);
    }

    #[test]
    fn document_links_are_kept_raw_for_the_apply_pass() {
        let fixture = Fixture::new(vec![field(1, "See", PaperlessFieldType::DocumentLink)]);
        let planned = fixture.plan(
            &[(1, PaperlessFieldValue::DocumentLink(vec![4, 2]))],
            &[],
            None,
        );
        assert_eq!(planned.links[&1], vec![4, 2]);
        assert!(planned.values.is_empty());
    }

    #[test]
    fn duplicate_values_for_a_field_are_dropped_whatever_their_order() {
        let fixture = Fixture::new(vec![field(1, "Name", PaperlessFieldType::String)]);
        let a = PaperlessFieldValue::Text("a".into());
        let b = PaperlessFieldValue::Text("b".into());

        let first = fixture.plan(&[(1, a.clone()), (1, b.clone())], &[], None);
        let second = fixture.plan(&[(1, b), (1, a)], &[], None);

        assert_eq!(first, second);
        assert!(first.values.is_empty());
        assert!(matches!(
            &first.warnings[..],
            [ItemWarning::ValueDropped {
                retryable: false,
                ..
            }]
        ));
    }

    #[test]
    fn links_into_a_field_of_another_kind_are_dropped() {
        let fixture = Fixture::new(vec![field(1, "Name", PaperlessFieldType::String)]);
        let planned = fixture.plan(
            &[(1, PaperlessFieldValue::DocumentLink(vec![2]))],
            &[],
            None,
        );
        assert!(planned.links.is_empty());
        assert_eq!(planned.warnings.len(), 1);
    }

    #[test]
    fn only_drops_a_later_sync_could_fix_are_retryable() {
        let mut fixture = Fixture::new(vec![field(1, "Link", PaperlessFieldType::Url)]);
        fixture.asn_owners.insert(17, Id::from_u128(2));

        let planned = fixture.plan(
            &[
                (1, PaperlessFieldValue::Url("ftp://x".into())),
                (9, PaperlessFieldValue::Integer(1)),
            ],
            &[note(1, 1, " ")],
            Some(17),
        );

        let retryable: Vec<_> = planned
            .warnings
            .iter()
            .map(ItemWarning::retryable)
            .collect();
        // Invalid URL, unknown field, blank note, taken ASN
        assert_eq!(retryable, vec![false, true, false, true]);
    }
}

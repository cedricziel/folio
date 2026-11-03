use folio_core::models::{Id, MetadataKind, MetadataValue};
use folio_core::{Error, Result};
use sha2::{Digest, Sha256};

/// The typed columns of one value; exactly the columns of its kind are set.
#[derive(Default)]
pub(crate) struct Columns {
    pub text: Option<String>,
    pub boolean: Option<bool>,
    pub integer: Option<i64>,
    pub decimal: Option<String>,
    pub date: Option<String>,
    pub currency: Option<String>,
    pub option: Option<Id>,
    pub target: Option<Id>,
}

impl Columns {
    pub fn from_value(value: Option<&MetadataValue>) -> Result<Self> {
        let mut columns = Self::default();
        match value {
            None => {}
            Some(
                MetadataValue::Text(text)
                | MetadataValue::LongText(text)
                | MetadataValue::Url(text),
            ) => columns.text = Some(text.clone()),
            Some(MetadataValue::Boolean(value)) => columns.boolean = Some(*value),
            Some(MetadataValue::Integer(value)) => {
                columns.integer = Some(value.parse().map_err(|_| {
                    Error::Validation("Integer must fit in a signed 64-bit value".into())
                })?)
            }
            Some(MetadataValue::Decimal(value)) => columns.decimal = Some(value.clone()),
            Some(MetadataValue::Date(value)) => columns.date = Some(value.clone()),
            Some(MetadataValue::Money { amount, currency }) => {
                columns.decimal = Some(amount.clone());
                columns.currency = Some(currency.clone());
            }
            Some(MetadataValue::Select(id)) => columns.option = Some(*id),
            Some(MetadataValue::DocumentLink(id)) => columns.target = Some(*id),
        }
        Ok(columns)
    }

    /// The retained value, or `None` for a cleared assignment.
    pub fn into_value(self, kind: MetadataKind, cleared: bool) -> Result<Option<MetadataValue>> {
        if cleared {
            return Ok(None);
        }
        let corrupt = || Error::Repository(format!("Stored {} value is incomplete", kind.as_str()));
        Ok(Some(match kind {
            MetadataKind::Text => MetadataValue::Text(self.text.ok_or_else(corrupt)?),
            MetadataKind::LongText => MetadataValue::LongText(self.text.ok_or_else(corrupt)?),
            MetadataKind::Url => MetadataValue::Url(self.text.ok_or_else(corrupt)?),
            MetadataKind::Boolean => MetadataValue::Boolean(self.boolean.ok_or_else(corrupt)?),
            MetadataKind::Integer => {
                MetadataValue::Integer(self.integer.ok_or_else(corrupt)?.to_string())
            }
            MetadataKind::Decimal => MetadataValue::Decimal(self.decimal.ok_or_else(corrupt)?),
            MetadataKind::Date => MetadataValue::Date(self.date.ok_or_else(corrupt)?),
            MetadataKind::Money => MetadataValue::Money {
                amount: self.decimal.ok_or_else(corrupt)?,
                currency: self.currency.ok_or_else(corrupt)?,
            },
            MetadataKind::Select => MetadataValue::Select(self.option.ok_or_else(corrupt)?),
            MetadataKind::DocumentLink => {
                MetadataValue::DocumentLink(self.target.ok_or_else(corrupt)?)
            }
        }))
    }
}

pub(crate) fn parse_kind(kind: &str) -> Result<MetadataKind> {
    serde_json::from_value(serde_json::Value::String(kind.into()))
        .map_err(|e| Error::Repository(e.to_string()))
}

/// SHA-256 hex digest of the canonical form, so uniqueness has a bounded index key.
pub(crate) fn canonical_key(value: &MetadataValue) -> Result<String> {
    let canonical = serde_json::to_vec(value).map_err(|e| Error::Repository(e.to_string()))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

pub(crate) fn change(
    field_id: Id,
    kind: MetadataKind,
    before: Option<&MetadataValue>,
    after: Option<&MetadataValue>,
) -> serde_json::Value {
    serde_json::json!({"field_id": field_id, "kind": kind, "before": before, "after": after})
}

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MetadataKind {
    Text,
    LongText,
    Boolean,
    Integer,
    Decimal,
    Date,
    Money,
    Select,
    Url,
    DocumentLink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MetadataValue {
    Text(String),
    LongText(String),
    Boolean(bool),
    Integer(String),
    Decimal(String),
    Date(String),
    Money { amount: String, currency: String },
    Select(uuid::Uuid),
    Url(String),
    DocumentLink(uuid::Uuid),
}

impl MetadataKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::LongText => "long_text",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Decimal => "decimal",
            Self::Date => "date",
            Self::Money => "money",
            Self::Select => "select",
            Self::Url => "url",
            Self::DocumentLink => "document_link",
        }
    }

    /// Validate an explicit replacement or clear; callers omit unchanged fields entirely.
    pub fn validate_value(
        self,
        value: Option<MetadataValue>,
        required: bool,
    ) -> Result<Option<MetadataValue>> {
        match value {
            Some(value) => value.validate(self, required).map(Some),
            None if required => invalid("A required field cannot be cleared"),
            None => Ok(None),
        }
    }
}

impl MetadataValue {
    pub fn kind(&self) -> MetadataKind {
        match self {
            Self::Text(_) => MetadataKind::Text,
            Self::LongText(_) => MetadataKind::LongText,
            Self::Boolean(_) => MetadataKind::Boolean,
            Self::Integer(_) => MetadataKind::Integer,
            Self::Decimal(_) => MetadataKind::Decimal,
            Self::Date(_) => MetadataKind::Date,
            Self::Money { .. } => MetadataKind::Money,
            Self::Select(_) => MetadataKind::Select,
            Self::Url(_) => MetadataKind::Url,
            Self::DocumentLink(_) => MetadataKind::DocumentLink,
        }
    }

    /// Validate and canonicalize a present value. Repositories enforce reference access and uniqueness.
    pub fn validate(self, kind: MetadataKind, required: bool) -> Result<Self> {
        if self.kind() != kind {
            return invalid("Value kind does not match the field");
        }
        match self {
            Self::Text(value) => {
                validate_text(&value, 1_024, required)?;
                Ok(Self::Text(value))
            }
            Self::LongText(value) => {
                validate_text(&value, 65_536, required)?;
                Ok(Self::LongText(value))
            }
            Self::Integer(value) => {
                let digits = value.strip_prefix('-').unwrap_or(&value);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return invalid(
                        "Integer must contain decimal digits with an optional minus sign",
                    );
                }
                let integer: i64 = value.parse().map_err(|_| {
                    Error::Validation("Integer must fit in a signed 64-bit value".into())
                })?;
                Ok(Self::Integer(integer.to_string()))
            }
            Self::Decimal(value) => Ok(Self::Decimal(canonical_decimal(&value)?)),
            Self::Money { amount, currency } => {
                if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
                    return invalid("Currency must contain three uppercase ASCII letters");
                }
                Ok(Self::Money {
                    amount: canonical_decimal(&amount)?,
                    currency,
                })
            }
            Self::Date(value) => {
                if value.len() != 10
                    || NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                        .ok()
                        .is_none_or(|date| date.format("%Y-%m-%d").to_string() != value)
                {
                    return invalid("Date must be a valid calendar date in YYYY-MM-DD format");
                }
                Ok(Self::Date(value))
            }
            Self::Url(value) => {
                validate_text(&value, 8_192, true)?;
                let parsed = url::Url::parse(&value)
                    .map_err(|_| Error::Validation("URL must be absolute HTTP or HTTPS".into()))?;
                if !matches!(parsed.scheme(), "http" | "https")
                    || parsed.host_str().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || value.chars().any(char::is_whitespace)
                {
                    return invalid(
                        "URL must be absolute HTTP or HTTPS without credentials or whitespace",
                    );
                }
                let normalized: String = parsed.into();
                validate_text(&normalized, 8_192, true)?;
                Ok(Self::Url(normalized))
            }
            value => Ok(value),
        }
    }
}

fn validate_text(value: &str, max: usize, required: bool) -> Result<()> {
    if value.chars().count() > max {
        return invalid("Text exceeds the field length limit");
    }
    if required && value.trim().is_empty() {
        return invalid("A required field cannot be blank");
    }
    Ok(())
}

fn canonical_decimal(value: &str) -> Result<String> {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if integer.is_empty()
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || unsigned.ends_with('.')
        || fraction.len() > 18
    {
        return invalid("Decimal requires digits, at most 18 fractional digits, and no exponent");
    }
    let integer = integer.trim_start_matches('0');
    let fraction = fraction.trim_end_matches('0');
    let significant = format!("{integer}{fraction}");
    if significant.trim_start_matches('0').len() > 38 {
        return invalid("Decimal exceeds 38 significant digits");
    }
    if integer.is_empty() && fraction.is_empty() {
        return Ok("0".into());
    }
    let sign = if value.starts_with('-') { "-" } else { "" };
    let integer = if integer.is_empty() { "0" } else { integer };
    if fraction.is_empty() {
        Ok(format!("{sign}{integer}"))
    } else {
        Ok(format!("{sign}{integer}.{fraction}"))
    }
}

fn invalid<T>(message: &str) -> Result<T> {
    Err(Error::Validation(message.into()))
}

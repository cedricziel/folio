use std::{fmt, str::FromStr};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

/// A Gregorian date from year 1 through 9999, serialized as YYYY-MM-DD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(value_type = String, format = Date))]
pub struct DocumentDate(NaiveDate);

impl FromStr for DocumentDate {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = "expected a valid YYYY-MM-DD calendar date";
        let bytes = value.as_bytes();
        if bytes.len() != 10
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || !bytes
                .iter()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        {
            return Err(invalid);
        }
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid)?;
        if !(1..=9999).contains(&date.year()) {
            return Err(invalid);
        }
        Ok(Self(date))
    }
}

impl fmt::Display for DocumentDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<String> for DocumentDate {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<DocumentDate> for String {
    fn from(value: DocumentDate) -> Self {
        value.to_string()
    }
}

//! Apple app identifiers for the app site association

use std::fmt;
use std::str::FromStr;

/// `TEAMID.bundle.id`: a 10-character uppercase alphanumeric Team ID and a bundle identifier
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleAppId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid Apple app ID `{0}`: expected TEAMID.bundle.id with a 10-character uppercase Team ID"
)]
pub struct InvalidAppleAppId(pub String);

impl AppleAppId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Split on commas and newlines, trim, and skip blank entries
    pub fn parse_list(input: &str) -> Result<Vec<Self>, InvalidAppleAppId> {
        input
            .split([',', '\n'])
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::parse)
            .collect()
    }
}

impl FromStr for AppleAppId {
    type Err = InvalidAppleAppId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = s.split_once('.').is_some_and(|(team, bundle)| {
            team.len() == 10
                && team
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                && bundle.starts_with(|c: char| c.is_ascii_alphanumeric())
                && bundle
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        });
        if valid {
            Ok(Self(s.to_string()))
        } else {
            Err(InvalidAppleAppId(s.to_string()))
        }
    }
}

impl fmt::Display for AppleAppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_team_id_and_bundle_id() {
        let id: AppleAppId = "ABCDE12345.com.example.folio".parse().unwrap();
        assert_eq!(id.as_str(), "ABCDE12345.com.example.folio");
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in ["com.example.folio", "abcde12345.x", "ABCDE1234.x", ""] {
            assert!(bad.parse::<AppleAppId>().is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn parse_list_splits_on_commas_and_newlines_and_trims() {
        let ids = AppleAppId::parse_list(
            " ABCDE12345.com.example.folio ,ABCDE12345.com.example.beta\n\n  FGHIJ67890.org.x \r\n",
        )
        .unwrap();
        let ids: Vec<_> = ids.iter().map(AppleAppId::as_str).collect();
        assert_eq!(
            ids,
            [
                "ABCDE12345.com.example.folio",
                "ABCDE12345.com.example.beta",
                "FGHIJ67890.org.x"
            ]
        );
    }

    #[test]
    fn parse_list_names_the_invalid_entry() {
        let err =
            AppleAppId::parse_list("ABCDE12345.com.example.folio\nnot-an-app-id").unwrap_err();
        assert_eq!(err, InvalidAppleAppId("not-an-app-id".into()));
    }
}

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

    /// Split on commas and newlines, then [`Self::parse_all`]
    pub fn parse_list(input: &str) -> Result<Vec<Self>, InvalidAppleAppId> {
        Self::parse_all(input.split([',', '\n']))
    }

    /// Trim each entry, skip blank ones and drop repeats, keeping the first occurrence's order
    pub fn parse_all<'a>(
        entries: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<Self>, InvalidAppleAppId> {
        let mut ids: Vec<Self> = Vec::new();
        for entry in entries.into_iter().map(str::trim) {
            if entry.is_empty() {
                continue;
            }
            let id: Self = entry.parse()?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
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
                && bundle.split('.').all(|segment| {
                    segment.starts_with(|c: char| c.is_ascii_alphanumeric())
                        && segment
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
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
        for bad in [
            "com.example.folio",
            "abcde12345.x",
            "ABCDE1234.x",
            "",
            "ABCDE12345.com.",
            "ABCDE12345..com.x",
            "ABCDE12345.com..x",
            "ABCDE12345.a.-b",
            "ABCDE12345.-a",
        ] {
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
    fn hyphens_inside_segments_are_allowed() {
        assert!("ABCDE12345.com.example-co.folio-beta"
            .parse::<AppleAppId>()
            .is_ok());
    }

    #[test]
    fn parse_list_drops_duplicates_keeping_first_order() {
        let ids = AppleAppId::parse_list("FGHIJ67890.b,ABCDE12345.a\nFGHIJ67890.b, ABCDE12345.a")
            .unwrap();
        let ids: Vec<_> = ids.iter().map(AppleAppId::as_str).collect();
        assert_eq!(ids, ["FGHIJ67890.b", "ABCDE12345.a"]);
    }

    #[test]
    fn parse_all_trims_skips_blanks_and_dedupes() {
        let ids = AppleAppId::parse_all([" ABCDE12345.a", "", "ABCDE12345.a"]).unwrap();
        assert_eq!(ids, ["ABCDE12345.a".parse::<AppleAppId>().unwrap()]);
    }

    #[test]
    fn parse_list_names_the_invalid_entry() {
        let err =
            AppleAppId::parse_list("ABCDE12345.com.example.folio\nnot-an-app-id").unwrap_err();
        assert_eq!(err, InvalidAppleAppId("not-an-app-id".into()));
    }
}

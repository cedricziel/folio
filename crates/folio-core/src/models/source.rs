//! Where a document came from

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::workflow::glob_match;
use crate::error::{Error, Result};

/// How a document entered Folio
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Uploaded through the web UI
    #[default]
    Upload,
    /// Uploaded through the JSON API
    Api,
    /// Picked up from a consume folder
    Folder,
    /// Extracted from an email
    Email,
    /// Created by an MCP client
    Mcp,
    /// Received through a share link
    Share,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Api => "api",
            Self::Folder => "folder",
            Self::Email => "email",
            Self::Mcp => "mcp",
            Self::Share => "share",
        }
    }

    /// Parse the wire name; unknown values fall back to `upload`
    pub fn parse(value: &str) -> Self {
        [
            Self::Upload,
            Self::Api,
            Self::Folder,
            Self::Email,
            Self::Mcp,
            Self::Share,
        ]
        .into_iter()
        .find(|k| k.as_str() == value)
        .unwrap_or_default()
    }
}

/// Where in a consume folder a file was found
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FolderOrigin {
    /// Path relative to the consume folder, using `/` separators
    pub relative_path: String,
    pub original_filename: String,
}

/// The email an attachment (or the message itself) came from
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailOrigin {
    /// Name of the configured mail account
    pub account: String,
    pub mailbox: String,
    pub message_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub date: Option<DateTime<Utc>>,
    pub attachment_filename: Option<String>,
    pub attachment_content_type: Option<String>,
}

/// The source of a document, with kind-specific metadata
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentSource {
    pub kind: SourceKind,
    /// Name of the ingestion source that produced the document, if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<FolderOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<EmailOrigin>,
}

impl DocumentSource {
    pub fn of(kind: SourceKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }

    pub fn folder(source_name: &str, origin: FolderOrigin) -> Self {
        Self {
            kind: SourceKind::Folder,
            source_name: Some(source_name.to_string()),
            folder: Some(origin),
            email: None,
        }
    }

    pub fn email(source_name: &str, origin: EmailOrigin) -> Self {
        Self {
            kind: SourceKind::Email,
            source_name: Some(source_name.to_string()),
            folder: None,
            email: Some(origin),
        }
    }

    /// One line for people, such as `Email from billing@shop.test: Your invoice`
    pub fn describe(&self) -> String {
        let name = self.source_name.as_deref();
        match (self.kind, &self.folder, &self.email) {
            (SourceKind::Folder, Some(f), _) => {
                format!("Folder {}: {}", name.unwrap_or_default(), f.relative_path)
            }
            (SourceKind::Email, _, Some(e)) => {
                format!("Email from {}: {}", e.from, e.subject)
            }
            (kind, _, _) => {
                let label = match kind {
                    SourceKind::Upload => "Web upload",
                    SourceKind::Api => "API",
                    SourceKind::Folder => "Folder",
                    SourceKind::Email => "Email",
                    SourceKind::Mcp => "MCP",
                    SourceKind::Share => "Share link",
                };
                label.to_string()
            }
        }
    }

    /// Kind-specific metadata as stored in the database; `None` when empty
    pub fn metadata_json(&self) -> Option<String> {
        if self.source_name.is_none() && self.folder.is_none() && self.email.is_none() {
            return None;
        }
        #[derive(Serialize)]
        struct Meta<'a> {
            source_name: &'a Option<String>,
            folder: &'a Option<FolderOrigin>,
            email: &'a Option<EmailOrigin>,
        }
        serde_json::to_string(&Meta {
            source_name: &self.source_name,
            folder: &self.folder,
            email: &self.email,
        })
        .ok()
    }

    /// Rebuild from the stored kind and metadata; bad metadata is dropped
    pub fn from_stored(kind: &str, metadata: Option<&str>) -> Self {
        #[derive(Deserialize, Default)]
        struct Meta {
            source_name: Option<String>,
            folder: Option<FolderOrigin>,
            email: Option<EmailOrigin>,
        }
        let meta: Meta = metadata
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or_default();
        Self {
            kind: SourceKind::parse(kind),
            source_name: meta.source_name,
            folder: meta.folder,
            email: meta.email,
        }
    }
}

/// How to compare a piece of text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TextMatch {
    /// `*` and `?` wildcards, case-insensitive, against the whole text
    Glob(String),
    /// Case-insensitive substring
    Contains(String),
    /// Regular expression, searched anywhere in the text
    Regex(String),
}

impl TextMatch {
    pub fn validate(&self) -> Result<()> {
        if let Self::Regex(pattern) = self {
            regex::Regex::new(pattern)
                .map_err(|e| Error::Validation(format!("Invalid regex '{pattern}': {e}")))?;
        }
        Ok(())
    }

    pub fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Glob(pattern) => glob_match(pattern, text),
            Self::Contains(needle) => text.to_lowercase().contains(&needle.to_lowercase()),
            Self::Regex(pattern) => regex::Regex::new(pattern).is_ok_and(|re| re.is_match(text)),
        }
    }
}

/// Conditions on where a document came from; every set condition has to match
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SourceConditions {
    /// How the document arrived
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<SourceKind>,
    /// Glob against the folder-relative path, for example `Insurance/*`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_path_glob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_from: Option<TextMatch>,
    /// Matches when any recipient matches
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_to: Option<TextMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_subject: Option<TextMatch>,
    /// Glob against the attachment file name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_attachment_glob: Option<String>,
    /// Glob against the mailbox name, for example `INBOX/Invoices`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_mailbox_glob: Option<String>,
}

impl SourceConditions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self) -> Result<()> {
        for m in [&self.email_from, &self.email_to, &self.email_subject]
            .into_iter()
            .flatten()
        {
            m.validate()?;
        }
        Ok(())
    }

    /// Conditions about a kind of metadata the source lacks never match
    pub fn matches(&self, source: &DocumentSource) -> bool {
        if self.kind.is_some_and(|k| k != source.kind) {
            return false;
        }
        if let Some(pattern) = &self.folder_path_glob {
            let Some(folder) = &source.folder else {
                return false;
            };
            if !glob_match(pattern, &folder.relative_path) {
                return false;
            }
        }

        let email_set = self.email_from.is_some()
            || self.email_to.is_some()
            || self.email_subject.is_some()
            || self.email_attachment_glob.is_some()
            || self.email_mailbox_glob.is_some();
        if !email_set {
            return true;
        }
        let Some(email) = &source.email else {
            return false;
        };
        if self
            .email_from
            .as_ref()
            .is_some_and(|m| !m.is_match(&email.from))
        {
            return false;
        }
        if self
            .email_to
            .as_ref()
            .is_some_and(|m| !email.to.iter().any(|t| m.is_match(t)))
        {
            return false;
        }
        if self
            .email_subject
            .as_ref()
            .is_some_and(|m| !m.is_match(&email.subject))
        {
            return false;
        }
        if let Some(pattern) = &self.email_attachment_glob {
            match &email.attachment_filename {
                Some(name) if glob_match(pattern, name) => {}
                _ => return false,
            }
        }
        if let Some(pattern) = &self.email_mailbox_glob {
            if !glob_match(pattern, &email.mailbox) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(from: &str, subject: &str, attachment: &str) -> DocumentSource {
        DocumentSource::email(
            "work",
            EmailOrigin {
                account: "work".into(),
                mailbox: "INBOX/Invoices".into(),
                message_id: "<1@x>".into(),
                from: from.into(),
                to: vec!["me@home.test".into(), "other@home.test".into()],
                subject: subject.into(),
                date: None,
                attachment_filename: Some(attachment.into()),
                attachment_content_type: Some("application/pdf".into()),
            },
        )
    }

    #[test]
    fn email_conditions_cover_every_field() {
        let source = mail(
            "Billing <billing@shop.test>",
            "Your invoice 42",
            "inv-42.pdf",
        );
        let all = SourceConditions {
            kind: Some(SourceKind::Email),
            email_from: Some(TextMatch::Glob("*@shop.test>".into())),
            email_to: Some(TextMatch::Contains("OTHER@".into())),
            email_subject: Some(TextMatch::Regex(r"invoice \d+".into())),
            email_attachment_glob: Some("inv-*.pdf".into()),
            email_mailbox_glob: Some("INBOX/*".into()),
            ..Default::default()
        };
        assert!(all.matches(&source));

        let wrong_subject = SourceConditions {
            email_subject: Some(TextMatch::Regex(r"^invoice".into())),
            ..Default::default()
        };
        assert!(!wrong_subject.matches(&source));
        let wrong_to = SourceConditions {
            email_to: Some(TextMatch::Glob("nobody@*".into())),
            ..Default::default()
        };
        assert!(!wrong_to.matches(&source));
    }

    #[test]
    fn conditions_on_missing_metadata_do_not_match() {
        let upload = DocumentSource::of(SourceKind::Upload);
        let folder = SourceConditions {
            folder_path_glob: Some("*".into()),
            ..Default::default()
        };
        let email = SourceConditions {
            email_from: Some(TextMatch::Contains("a".into())),
            ..Default::default()
        };
        assert!(!folder.matches(&upload));
        assert!(!email.matches(&upload));
        assert!(SourceConditions::default().matches(&upload));
    }

    #[test]
    fn folder_glob_matches_relative_path() {
        let source = DocumentSource::folder(
            "inbox",
            FolderOrigin {
                relative_path: "Insurance/2025/x.pdf".into(),
                original_filename: "x.pdf".into(),
            },
        );
        let cond = SourceConditions {
            kind: Some(SourceKind::Folder),
            folder_path_glob: Some("insurance/*".into()),
            ..Default::default()
        };
        assert!(cond.matches(&source));
    }

    #[test]
    fn invalid_regex_is_rejected() {
        let cond = SourceConditions {
            email_from: Some(TextMatch::Regex("(".into())),
            ..Default::default()
        };
        assert!(cond.validate().is_err());
    }

    #[test]
    fn source_round_trips_through_storage_columns() {
        let source = mail("a@b.test", "s", "f.pdf");
        let back =
            DocumentSource::from_stored(source.kind.as_str(), source.metadata_json().as_deref());
        assert_eq!(back, source);
        assert_eq!(
            DocumentSource::from_stored("upload", None),
            DocumentSource::of(SourceKind::Upload)
        );
    }
}

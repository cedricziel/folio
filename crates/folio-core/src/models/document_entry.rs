use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Id, UserId, VaultId, VaultRole};
use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum EntryKind {
    Note,
    Comment,
}

impl EntryKind {
    /// Return the stable spelling shared by database kinds and JSON values.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Comment => "comment",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EntryBody(String);

impl EntryBody {
    /// Validate nonblank plain text, bounded by UTF-8 bytes; preserve the author's whitespace.
    pub fn parse(body: String) -> Result<Self> {
        if body.trim().is_empty() || body.len() > 16 * 1024 {
            return Err(Error::Validation(
                "Entry text must contain content and be at most 16384 bytes".into(),
            ));
        }
        Ok(Self(body))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub document_id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub kind: EntryKind,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EntryPermissions {
    pub can_edit: bool,
    pub can_delete: bool,
}

impl EntryPermissions {
    /// Authors with a writable role may edit; editors may delete any entry but cannot rewrite it.
    /// Callers must establish current vault membership before evaluating these capabilities.
    pub fn for_member(role: VaultRole, user: &UserId, author: Option<&UserId>) -> Self {
        let own = role >= VaultRole::Contributor && author == Some(user);
        Self {
            can_edit: own,
            can_delete: own || role >= VaultRole::Editor,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryCursor {
    vault_id: VaultId,
    document_id: Id,
    kind: EntryKind,
    pub created_at: DateTime<Utc>,
    pub id: Id,
}

#[derive(Clone, Debug)]
pub struct EntryPageQuery {
    pub kind: EntryKind,
    pub limit: u32,
    pub cursor: Option<EntryCursor>,
}

impl EntryPageQuery {
    /// Reject unbounded reads and continuations from a different collection.
    pub fn validate(&self, vault: VaultId, document: Id) -> Result<()> {
        if !(1..=100).contains(&self.limit) {
            return Err(Error::Validation(
                "Page limit must be between 1 and 100".into(),
            ));
        }
        if self.cursor.as_ref().is_some_and(|cursor| {
            cursor.vault_id != vault || cursor.document_id != document || cursor.kind != self.kind
        }) {
            return Err(Error::Validation(
                "Cursor does not match this entry query".into(),
            ));
        }
        Ok(())
    }

    /// Build a bounded page from ordered records fetched with one extra row.
    pub fn finish(
        &self,
        vault: VaultId,
        document: Id,
        mut entries: Vec<DocumentEntry>,
    ) -> EntryPage {
        let more = entries.len() > self.limit as usize;
        entries.truncate(self.limit as usize);
        let next_cursor = entries.last().filter(|_| more).map(|last| EntryCursor {
            vault_id: vault,
            document_id: document,
            kind: self.kind,
            created_at: last.created_at,
            id: last.id,
        });
        EntryPage {
            entries,
            next_cursor,
        }
    }
}

#[derive(Debug)]
pub struct EntryPage {
    pub entries: Vec<DocumentEntry>,
    pub next_cursor: Option<EntryCursor>,
}

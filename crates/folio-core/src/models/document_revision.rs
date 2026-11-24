use chrono::{DateTime, Utc};

use super::{Id, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalVerification {
    Pending,
    Verified,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionOriginal {
    pub id: Id,
    pub storage_key: String,
    pub filename: Option<String>,
    pub mime_type: String,
    pub verification: OriginalVerification,
    pub sha256: Option<String>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRevision {
    pub id: Id,
    pub number: i64,
    pub is_current: bool,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<UserId>,
    pub restored_from_revision_id: Option<Id>,
    pub original: RevisionOriginal,
}

#[derive(Debug, Clone)]
pub struct DocumentRevisionPage {
    pub revisions: Vec<DocumentRevision>,
    pub next_before: Option<i64>,
}

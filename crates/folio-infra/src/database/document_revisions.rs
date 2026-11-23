use chrono::{DateTime, Utc};
use folio_core::models::{DocumentRevision, OriginalVerification, RevisionOriginal, UserId};
use folio_core::{Error, Result};

pub(super) const READ_REVISIONS: &str = "
    SELECT CAST(r.id AS TEXT) AS id, r.number, r.created_at, r.created_by,
           CAST(r.restored_from_revision_id AS TEXT) AS restored_from_revision_id,
           COALESCE(d.current_revision_id = r.id, FALSE) AS is_current,
           CAST(f.id AS TEXT) AS file_id, f.storage_key, f.original_filename, f.mime_type,
           f.verification_state, f.sha256, f.byte_size
      FROM documents d
      JOIN vaults v ON v.id = d.vault_id AND v.deleted_at IS NULL
      JOIN document_revisions r ON r.document_id = d.id AND r.vault_id = d.vault_id
      JOIN document_files f ON f.id = r.original_file_id AND f.document_id = r.document_id
                            AND f.vault_id = r.vault_id AND f.role = 'original'
";

#[derive(sqlx::FromRow)]
pub(super) struct RevisionRow {
    id: String,
    number: i64,
    is_current: bool,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
    restored_from_revision_id: Option<String>,
    file_id: String,
    storage_key: String,
    original_filename: Option<String>,
    mime_type: String,
    verification_state: String,
    sha256: Option<String>,
    byte_size: Option<i64>,
}

impl TryFrom<RevisionRow> for DocumentRevision {
    type Error = Error;

    fn try_from(row: RevisionRow) -> Result<Self> {
        let parse_id = |value: &str| {
            uuid::Uuid::parse_str(value).map_err(|error| Error::Repository(error.to_string()))
        };
        let verification = match row.verification_state.as_str() {
            "pending" => OriginalVerification::Pending,
            "verified" => OriginalVerification::Verified,
            "unavailable" => OriginalVerification::Unavailable,
            _ => {
                return Err(Error::Repository(
                    "Unknown original verification state".into(),
                ))
            }
        };
        let byte_size = row
            .byte_size
            .map(u64::try_from)
            .transpose()
            .map_err(|error| Error::Repository(error.to_string()))?;
        Ok(Self {
            id: parse_id(&row.id)?,
            number: row.number,
            is_current: row.is_current,
            created_at: row.created_at,
            created_by: row.created_by.map(UserId),
            restored_from_revision_id: row
                .restored_from_revision_id
                .as_deref()
                .map(parse_id)
                .transpose()?,
            original: RevisionOriginal {
                id: parse_id(&row.file_id)?,
                storage_key: row.storage_key,
                filename: row.original_filename,
                mime_type: row.mime_type,
                verification,
                sha256: row.sha256,
                byte_size,
            },
        })
    }
}

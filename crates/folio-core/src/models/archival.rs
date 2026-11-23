use super::{FileIdentity, Id, VaultId};
use crate::{Error, Result};

/// Identifies the verified original a conversion attempt is allowed to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSource {
    pub vault_id: VaultId,
    pub document_id: Id,
    pub revision_id: Id,
    pub original: FileIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveTool {
    pub name: String,
    pub version: String,
}

/// The identity and provenance of the exact bytes accepted by a PDF/A validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedArchive {
    pub file: FileIdentity,
    /// Canonical PDF/A profile, for example `PDF/A-2b`.
    pub profile: String,
    pub converter: ArchiveTool,
    pub validator: ArchiveTool,
}

impl ValidatedArchive {
    pub fn validate_metadata(&self, source: &ArchiveSource, attempt: Id) -> Result<()> {
        let key =
            crate::ports::keys::revision_artifact(source.document_id, source.revision_id, attempt);
        if self.file.storage_key != key
            || self.file.storage_key == source.original.storage_key
            || self.file.mime_type != "application/pdf"
            || !(1..=i64::MAX as u64).contains(&self.file.byte_size)
            || self.file.sha256.len() != 64
            || !self
                .file
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Validation("Invalid archival file identity".into()));
        }
        if !matches!(
            self.profile.as_str(),
            "PDF/A-1a"
                | "PDF/A-1b"
                | "PDF/A-2a"
                | "PDF/A-2b"
                | "PDF/A-2u"
                | "PDF/A-3a"
                | "PDF/A-3b"
                | "PDF/A-3u"
                | "PDF/A-4"
                | "PDF/A-4e"
                | "PDF/A-4f"
        ) {
            return Err(Error::Validation("Invalid PDF/A profile".into()));
        }
        for tool in [&self.converter, &self.validator] {
            if tool.name.trim().is_empty() || tool.version.trim().is_empty() {
                return Err(Error::Validation(
                    "Archival tool name and version are required".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveCompletion {
    Ready(ValidatedArchive),
    Skipped(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveState {
    Queued,
    Completed(ArchiveCompletion),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveAttempt {
    pub id: Id,
    pub source: ArchiveSource,
    pub state: ArchiveState,
}

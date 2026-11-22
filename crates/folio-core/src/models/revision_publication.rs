use sha2::{Digest, Sha256};

use super::{FileIdentity, Id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionPublication {
    pub key: Id,
    pub expected_current: Id,
    pub source: RevisionPublicationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionPublicationResult {
    pub revision_id: Id,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionPublicationSource {
    Replacement {
        revision: Id,
        filename: String,
        identity: FileIdentity,
    },
    Restore {
        revision: Id,
    },
}

impl RevisionPublication {
    /// Bind a receipt to the operation, expected pointer and bytes, across attempts with different staging keys.
    pub fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"folio-revision-publication-v1");
        hash.update(self.expected_current.as_bytes());
        match &self.source {
            RevisionPublicationSource::Replacement {
                filename, identity, ..
            } => {
                hash.update(b"replace");
                hash.update(identity.byte_size.to_be_bytes());
                for part in [filename, &identity.mime_type, &identity.sha256] {
                    hash.update((part.len() as u64).to_be_bytes());
                    hash.update(part.as_bytes());
                }
            }
            RevisionPublicationSource::Restore { revision } => {
                hash.update(b"restore");
                hash.update(revision.as_bytes());
            }
        }
        hex::encode(hash.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_replacement_retries_share_a_fingerprint_across_staging_attempts() {
        let current = Id::from_u128(1);
        let replacement = |revision, key, filename: &str, bytes: &[u8]| RevisionPublication {
            key: Id::from_u128(2),
            expected_current: current,
            source: RevisionPublicationSource::Replacement {
                revision,
                filename: filename.to_owned(),
                identity: FileIdentity::from_bytes(key, "application/pdf", bytes),
            },
        };
        let first = replacement(Id::from_u128(3), "first", "invoice.pdf", b"scan");
        let retry = replacement(Id::from_u128(4), "retry", "invoice.pdf", b"scan");
        assert_eq!(first.fingerprint(), retry.fingerprint());
        let other_file = replacement(Id::from_u128(5), "other", "invoice.pdf", b"other scan");
        assert_ne!(first.fingerprint(), other_file.fingerprint());
        let renamed = replacement(Id::from_u128(6), "rename", "renamed.pdf", b"scan");
        assert_ne!(first.fingerprint(), renamed.fingerprint());
    }

    #[test]
    fn restore_fingerprint_changes_with_the_expected_pointer_or_source() {
        let restore = |expected, revision| RevisionPublication {
            key: Id::from_u128(1),
            expected_current: Id::from_u128(expected),
            source: RevisionPublicationSource::Restore { revision },
        };
        let first = restore(2, Id::from_u128(3));
        let new_current = restore(4, Id::from_u128(3));
        let different_source = restore(2, Id::from_u128(4));
        assert_ne!(first.fingerprint(), new_current.fingerprint());
        assert_ne!(first.fingerprint(), different_source.fingerprint());
    }
}

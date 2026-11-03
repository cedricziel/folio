use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestItemKey(String);

impl IngestItemKey {
    /// Identifies a file generation independently of the content deduplication key.
    pub fn folder(relative_path: &[u8], generation: &[u8], content: &[u8]) -> Self {
        Self::digest(&[b"folder-v1", relative_path, generation, content])
    }

    /// Requires server-assigned IMAP identity; part is the stable MIME part index.
    pub fn email(mailbox: &str, validity: u32, uid: u32, part: u32) -> Result<Self> {
        if validity == 0 || uid == 0 {
            return Err(Error::Validation(
                "Email item identity is unavailable".into(),
            ));
        }
        Ok(Self::digest(&[
            b"email-v1",
            mailbox.as_bytes(),
            &validity.to_be_bytes(),
            &uid.to_be_bytes(),
            &part.to_be_bytes(),
        ]))
    }

    /// Preserves ingestion on servers without UIDVALIDITY when Message-ID is available.
    pub fn email_message(mailbox: &str, message_id: &str, part: u32) -> Result<Self> {
        if message_id.trim().is_empty() {
            return Err(Error::Validation(
                "Email item identity is unavailable".into(),
            ));
        }
        Ok(Self::digest(&[
            b"email-message-v1",
            mailbox.as_bytes(),
            message_id.as_bytes(),
            &part.to_be_bytes(),
        ]))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn digest(parts: &[&[u8]]) -> Self {
        let mut hash = Sha256::new();
        for part in parts {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part);
        }
        Self(hex::encode(hash.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_identity_survives_polling_but_changes_with_mailbox_generation() {
        let first = IngestItemKey::email("INBOX", 7, 42, 0).unwrap();
        assert_eq!(first, IngestItemKey::email("INBOX", 7, 42, 0).unwrap());
        assert_ne!(first, IngestItemKey::email("INBOX", 8, 42, 0).unwrap());
        assert_ne!(first, IngestItemKey::email("INBOX", 7, 42, 1).unwrap());
        assert_ne!(first, IngestItemKey::email("Archive", 7, 42, 0).unwrap());
    }

    #[test]
    fn missing_email_identity_cannot_be_replaced_by_zero() {
        assert!(IngestItemKey::email("INBOX", 0, 42, 0).is_err());
        assert!(IngestItemKey::email("INBOX", 7, 0, 0).is_err());
    }

    #[test]
    fn folder_identity_distinguishes_equal_content_at_different_paths() {
        let first = IngestItemKey::folder(b"inbox/a.pdf", b"generation-one", b"pdf");
        assert_eq!(
            first,
            IngestItemKey::folder(b"inbox/a.pdf", b"generation-one", b"pdf")
        );
        assert_ne!(
            first,
            IngestItemKey::folder(b"inbox/b.pdf", b"generation-one", b"pdf")
        );
        assert_ne!(
            first,
            IngestItemKey::folder(b"inbox/a.pdf", b"generation-two", b"pdf")
        );
        assert_ne!(
            first,
            IngestItemKey::folder(b"inbox/a.pdf", b"generation-one", b"changed")
        );
    }

    #[test]
    fn locator_hash_is_unambiguous_and_debug_hides_input() {
        let first = IngestItemKey::folder(b"private/customer", b"a", b"bc");
        let second = IngestItemKey::folder(b"private/customer", b"ab", b"c");
        assert_ne!(first, second);
        assert_eq!(first.as_str().len(), 64);
        assert!(!format!("{first:?}").contains("customer"));
    }

    #[test]
    fn message_id_fallback_is_scoped_and_rejects_missing_identity() {
        let first = IngestItemKey::email_message("INBOX", "message@example.test", 1).unwrap();
        assert_eq!(
            first,
            IngestItemKey::email_message("INBOX", "message@example.test", 1).unwrap()
        );
        assert_ne!(
            first,
            IngestItemKey::email_message("Archive", "message@example.test", 1).unwrap()
        );
        assert!(IngestItemKey::email_message("INBOX", "", 1).is_err());
    }
}

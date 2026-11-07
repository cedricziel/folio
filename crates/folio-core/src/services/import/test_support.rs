//! Builders shared by the planning tests

use chrono::NaiveDate;

use super::diff::{FolioState, LedgerEntry};
use super::fingerprint::FINGERPRINT_VERSION;
use crate::models::Id;
use crate::ports::{
    ChecksumAlgorithm, PaperlessChecksum, PaperlessCustomField, PaperlessDocument,
    PaperlessFieldType, PaperlessNote,
};

pub fn document(id: i64) -> PaperlessDocument {
    PaperlessDocument {
        id,
        title: format!("Document {id}"),
        content: "Some text".into(),
        mime_type: "application/pdf".into(),
        original_filename: Some(format!("{id}.pdf")),
        checksum: PaperlessChecksum {
            value: "0".repeat(32),
            algorithm: ChecksumAlgorithm::Md5,
        },
        size: Some(1024),
        created: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        added: "2024-03-02T10:00:00Z".parse().unwrap(),
        modified: "2024-03-03T10:00:00Z".parse().unwrap(),
        correspondent: Some(1),
        document_type: Some(2),
        tags: vec![3, 4],
        archive_serial_number: Some(17),
        trashed: false,
        version_of: None,
        original_handle: format!("originals/{id}.pdf"),
    }
}

pub fn note(id: i64, document: i64, body: &str) -> PaperlessNote {
    PaperlessNote {
        id,
        document,
        body: body.into(),
        created: "2024-03-04T10:00:00Z".parse().unwrap(),
        deleted: false,
    }
}

pub fn custom_field(id: i64, name: &str, data_type: PaperlessFieldType) -> PaperlessCustomField {
    PaperlessCustomField {
        id,
        name: name.into(),
        data_type,
        select_options: vec![],
        default_currency: None,
    }
}

/// A live row for [`document`] with an unchanged file
pub fn ledger_row(paperless_id: i64) -> LedgerEntry {
    LedgerEntry {
        paperless_id,
        document_id: Id::from_u128(paperless_id as u128),
        checksum: document(paperless_id).checksum.value,
        fingerprint: String::new(),
        fingerprint_version: FINGERPRINT_VERSION,
        needs_reapply: false,
        trashed_in_paperless: false,
        folio_state: FolioState::Live,
    }
}

//! Document identity: deterministic Folio IDs and the fingerprint of the
//! Paperless-side state the ledger compares against

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::models::{Id, VaultId};
use crate::ports::{
    PaperlessCustomFieldInstance, PaperlessDocument, PaperlessFieldValue, PaperlessNote,
};

/// Bumped whenever the canonical form changes; a row with another version
/// counts as changed
pub const FINGERPRINT_VERSION: u32 = 1;

/// Namespace for imported document IDs; never change it, or re-imports
/// would stop finding their documents
pub const IMPORT_NAMESPACE: Uuid = Uuid::from_u128(0x6f0c_1d2e_8a4b_5c3d_9e7f_a1b2_c3d4_e5f6);

/// The Folio ID of a Paperless document in a vault, the same on every run
pub fn document_id(vault: VaultId, paperless_id: i64) -> Id {
    Uuid::new_v5(
        &IMPORT_NAMESPACE,
        format!("{}/paperless/{paperless_id}", vault.0).as_bytes(),
    )
}

/// One Paperless document with everything joined to it
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRecords {
    /// The root document
    pub document: PaperlessDocument,
    /// Paperless 3 file versions of the root, in any order
    pub versions: Vec<PaperlessDocument>,
    pub notes: Vec<PaperlessNote>,
    pub fields: Vec<PaperlessCustomFieldInstance>,
}

impl DocumentRecords {
    pub fn new(document: PaperlessDocument) -> Self {
        Self {
            document,
            versions: Vec::new(),
            notes: Vec::new(),
            fields: Vec::new(),
        }
    }

    /// The file whose bytes and text Folio keeps: the version with the
    /// highest index, else the root
    pub fn original(&self) -> &PaperlessDocument {
        self.versions
            .iter()
            .max_by_key(|v| (v.version_of.as_ref().and_then(|info| info.index), v.id))
            .unwrap_or(&self.document)
    }

    /// Older files left behind by [`Self::original`]
    pub fn skipped_versions(&self) -> usize {
        self.versions.len()
    }

    pub fn tag_set(&self) -> BTreeSet<i64> {
        self.document.tags.iter().copied().collect()
    }

    /// Notes that still exist in Paperless, by ID
    pub fn live_notes(&self) -> Vec<&PaperlessNote> {
        let mut notes: Vec<_> = self.notes.iter().filter(|n| !n.deleted).collect();
        notes.sort_by_key(|n| n.id);
        notes
    }
}

#[derive(Serialize)]
struct CanonicalV1<'a> {
    fingerprint_version: u32,
    title: &'a str,
    content_sha256: String,
    created: NaiveDate,
    correspondent: Option<i64>,
    document_type: Option<i64>,
    tags: BTreeSet<i64>,
    custom_fields: Vec<CanonicalField<'a>>,
    asn: Option<i64>,
    notes: Vec<CanonicalNote<'a>>,
    inbox: bool,
}

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalField<'a> {
    field: i64,
    value: Option<CanonicalValue<'a>>,
}

/// A custom-field value in a form fixed for fingerprint v1, independent of
/// how the port's types serialize
#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type", content = "value")]
enum CanonicalValue<'a> {
    #[serde(rename = "string")]
    Text(&'a str),
    #[serde(rename = "longtext")]
    LongText(&'a str),
    #[serde(rename = "url")]
    Url(&'a str),
    #[serde(rename = "date")]
    Date(String),
    #[serde(rename = "boolean")]
    Boolean(bool),
    #[serde(rename = "integer")]
    Integer(i64),
    /// As text, so NaN and infinities stay distinct from an empty value
    #[serde(rename = "float")]
    Float(String),
    #[serde(rename = "monetary")]
    Monetary(&'a str),
    #[serde(rename = "select")]
    Select(&'a str),
    #[serde(rename = "documentlink")]
    DocumentLink(&'a [i64]),
}

impl<'a> From<&'a PaperlessFieldValue> for CanonicalValue<'a> {
    fn from(value: &'a PaperlessFieldValue) -> Self {
        use PaperlessFieldValue as P;
        match value {
            P::Text(text) => Self::Text(text),
            P::LongText(text) => Self::LongText(text),
            P::Url(url) => Self::Url(url),
            P::Date(date) => Self::Date(date.format("%Y-%m-%d").to_string()),
            P::Boolean(flag) => Self::Boolean(*flag),
            P::Integer(number) => Self::Integer(*number),
            P::Float(number) => Self::Float(number.to_string()),
            P::Monetary(raw) => Self::Monetary(raw),
            P::Select(option) => Self::Select(option),
            P::DocumentLink(ids) => Self::DocumentLink(ids),
        }
    }
}

#[derive(Serialize)]
struct CanonicalNote<'a> {
    id: i64,
    body: &'a str,
    created: DateTime<Utc>,
}

/// SHA-256 over the canonical JSON of what the sync mirrors. It uses
/// Paperless IDs, not names, so renaming a tag changes no fingerprint.
pub fn fingerprint(records: &DocumentRecords, inbox_tags: &BTreeSet<i64>) -> String {
    let tags = records.tag_set();
    let inbox = !tags.is_disjoint(inbox_tags);
    fingerprint_of(
        records,
        records.original(),
        tags,
        &records.live_notes(),
        inbox,
    )
}

/// [`fingerprint`] over parts the caller already derived from `records`
pub(super) fn fingerprint_of(
    records: &DocumentRecords,
    original: &PaperlessDocument,
    tags: BTreeSet<i64>,
    live_notes: &[&PaperlessNote],
    inbox: bool,
) -> String {
    let document = &records.document;
    let mut custom_fields: Vec<CanonicalField> = records
        .fields
        .iter()
        .map(|f| CanonicalField {
            field: f.field,
            value: f.value.as_ref().map(CanonicalValue::from),
        })
        .collect();
    // Sorting by value too keeps duplicate instances order-independent
    custom_fields.sort();
    let canonical = CanonicalV1 {
        fingerprint_version: FINGERPRINT_VERSION,
        title: &document.title,
        content_sha256: hex::encode(Sha256::digest(original.content.as_bytes())),
        created: document.created,
        correspondent: document.correspondent,
        document_type: document.document_type,
        inbox,
        tags,
        custom_fields,
        asn: document.archive_serial_number,
        notes: live_notes
            .iter()
            .map(|n| CanonicalNote {
                id: n.id,
                body: &n.body,
                // The export has milliseconds, the API microseconds
                created: n.created.trunc_subsecs(0),
            })
            .collect(),
    };
    let mut hasher = HashWriter(Sha256::new());
    serde_json::to_writer(&mut hasher, &canonical).expect("canonical fingerprint serializes");
    hex::encode(hasher.0.finalize())
}

/// Streams the canonical JSON into the hash instead of buffering it
struct HashWriter(Sha256);

impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::PaperlessVersionInfo;
    use crate::services::import::test_support::{document, note};

    fn records() -> DocumentRecords {
        let mut records = DocumentRecords::new(document(1));
        records.notes = vec![note(2, 1, "second"), note(1, 1, "first")];
        records.fields = vec![
            PaperlessCustomFieldInstance {
                document: 1,
                field: 9,
                value: Some(PaperlessFieldValue::Monetary("EUR12.50".into())),
            },
            PaperlessCustomFieldInstance {
                document: 1,
                field: 3,
                value: Some(PaperlessFieldValue::Float(0.1)),
            },
        ];
        records
    }

    #[test]
    fn document_ids_are_derived_from_vault_and_paperless_id() {
        let vault = VaultId(Uuid::from_u128(1));
        assert_eq!(document_id(vault, 42), document_id(vault, 42));
        assert_ne!(document_id(vault, 42), document_id(vault, 43));
        assert_ne!(
            document_id(vault, 42),
            document_id(VaultId(Uuid::from_u128(2)), 42)
        );
        assert_eq!(
            document_id(vault, 42),
            Uuid::parse_str("bbe09c1a-d038-5062-8a28-5a530ff1a1db").unwrap()
        );
    }

    #[test]
    fn fingerprint_ignores_record_order() {
        let a = records();
        let mut b = records();
        b.notes.reverse();
        b.fields.reverse();
        b.fields.push(b.fields[0].clone());
        b.document.tags.reverse();
        let mut a = a;
        a.fields.push(a.fields[1].clone());
        assert_eq!(
            fingerprint(&a, &BTreeSet::new()),
            fingerprint(&b, &BTreeSet::new())
        );
    }

    #[test]
    fn fingerprint_tracks_mirrored_state() {
        let base = fingerprint(&records(), &BTreeSet::new());
        let changes: [fn(&mut DocumentRecords); 15] = [
            |r| r.document.title.push('!'),
            |r| r.document.content.push('!'),
            |r| r.document.created = r.document.created.succ_opt().unwrap(),
            |r| r.document.correspondent = Some(99),
            |r| r.document.document_type = None,
            |r| r.document.tags.push(5),
            |r| r.document.archive_serial_number = None,
            |r| r.notes[0].body.push('!'),
            |r| r.notes[0].created += chrono::Duration::seconds(1),
            |r| r.notes[0].deleted = true,
            |r| r.notes.push(note(3, 1, "third")),
            |r| r.fields[0].value = None,
            |r| r.fields[1].value = Some(PaperlessFieldValue::Float(f64::NAN)),
            |r| r.fields[1].field = 4,
            |r| r.versions.push(newer_version()),
        ];
        for change in changes {
            let mut changed = records();
            change(&mut changed);
            assert_ne!(fingerprint(&changed, &BTreeSet::new()), base);
        }
        assert_ne!(fingerprint(&records(), &BTreeSet::from([3])), base);
    }

    fn newer_version() -> PaperlessDocument {
        let mut version = document(2);
        version.content = "Newer text".into();
        version.version_of = Some(PaperlessVersionInfo {
            root_document: 1,
            index: Some(1),
            label: None,
        });
        version
    }

    #[test]
    fn nan_and_an_empty_value_fingerprint_differently() {
        let mut nan = records();
        nan.fields[1].value = Some(PaperlessFieldValue::Float(f64::NAN));
        let mut empty = records();
        empty.fields[1].value = None;
        assert_ne!(
            fingerprint(&nan, &BTreeSet::new()),
            fingerprint(&empty, &BTreeSet::new())
        );
    }

    #[test]
    fn note_times_count_in_whole_seconds() {
        let mut millis = records();
        millis.notes[0].created += chrono::Duration::milliseconds(123);
        let mut micros = records();
        micros.notes[0].created += chrono::Duration::microseconds(123_456);
        assert_eq!(
            fingerprint(&millis, &BTreeSet::new()),
            fingerprint(&micros, &BTreeSet::new())
        );
    }

    #[test]
    fn fingerprint_v1_is_pinned() {
        assert_eq!(
            fingerprint(&records(), &BTreeSet::from([3])),
            "6a482659973a3069c8a2def3fb62b5e8ccb3ffd489438391c30a8b7ca27afc06"
        );
    }

    #[test]
    fn fingerprint_ignores_what_is_compared_elsewhere() {
        let base = fingerprint(&records(), &BTreeSet::new());
        let mut changed = records();
        changed.document.modified = Utc::now();
        changed.document.checksum.value = "1".repeat(32);
        changed.document.original_handle = "elsewhere".into();
        assert_eq!(fingerprint(&changed, &BTreeSet::new()), base);
    }

    #[test]
    fn the_highest_version_is_the_original() {
        let mut records = records();
        let version = |id, index| {
            let mut v = document(id);
            v.content = format!("version {index}");
            v.version_of = Some(PaperlessVersionInfo {
                root_document: 1,
                index: Some(index),
                label: None,
            });
            v
        };
        records.versions = vec![version(8, 2), version(7, 1)];
        assert_eq!(records.original().id, 8);
        assert_eq!(records.skipped_versions(), 2);
    }
}

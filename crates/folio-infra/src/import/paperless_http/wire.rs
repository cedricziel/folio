//! Response shapes of the Paperless REST API (version 9), and their
//! conversion to the records the export adapter yields

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use folio_core::ports::{
    PaperlessChecksum, PaperlessCustomFieldInstance, PaperlessDocument, PaperlessFieldType,
    PaperlessFieldValue, PaperlessNote, PaperlessRecord, PaperlessVersionInfo,
};
use serde::Deserialize;
use serde_json::Value;

use crate::import::paperless_fs::CustomFieldFields;

/// `StandardPagination`, without the `all` list of every ID
#[derive(Deserialize)]
pub(super) struct Page<T> {
    pub next: Option<String>,
    pub results: Vec<T>,
}

/// The first page of the ID listing; API versions before 10 add `all`
#[derive(Deserialize)]
pub(super) struct IdPage {
    pub count: u64,
    pub all: Option<Vec<i64>>,
}

#[derive(Deserialize)]
pub(super) struct CustomField {
    pub id: i64,
    #[serde(flatten)]
    pub fields: CustomFieldFields,
}

/// What a listing needs from a document before reading the rest of it
#[derive(Deserialize)]
pub(super) struct Key {
    pub id: i64,
    modified: Option<String>,
}

impl Key {
    pub fn modified(&self) -> Option<DateTime<Utc>> {
        self.modified.as_deref()?.parse().ok()
    }
}

#[derive(Deserialize)]
pub(super) struct Document {
    pub id: i64,
    correspondent: Option<i64>,
    document_type: Option<i64>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    tags: Vec<i64>,
    created: NaiveDate,
    pub modified: DateTime<Utc>,
    added: DateTime<Utc>,
    deleted_at: Option<String>,
    archive_serial_number: Option<i64>,
    original_file_name: Option<String>,
    mime_type: String,
    #[serde(default)]
    notes: Vec<Note>,
    #[serde(default)]
    custom_fields: Vec<FieldInstance>,
    /// Paperless 3 only; set on a file version
    #[serde(default)]
    pub root_document: Option<i64>,
    /// Paperless 3 only; the root and every file version, newest first
    #[serde(default)]
    pub versions: Vec<Version>,
}

#[derive(Deserialize)]
struct Note {
    id: i64,
    note: String,
    created: DateTime<Utc>,
}

#[derive(Deserialize)]
struct FieldInstance {
    field: i64,
    value: Value,
}

#[derive(Deserialize)]
pub(super) struct Version {
    pub id: i64,
    pub added: DateTime<Utc>,
    pub version_label: Option<String>,
    pub checksum: Option<String>,
    pub is_root: bool,
}

/// `/api/documents/{id}/metadata/`
#[derive(Deserialize)]
pub(super) struct Metadata {
    pub original_checksum: String,
    pub original_size: Option<u64>,
    original_mime_type: Option<String>,
    original_filename: Option<String>,
}

/// `GET /api/documents/{id}/?fields=content`
#[derive(Deserialize)]
pub(super) struct Content {
    pub content: String,
}

/// The records of one document: the document, then its notes and values
pub(super) struct DocumentRecords {
    pub document: PaperlessDocument,
    pub rest: Vec<PaperlessRecord>,
    /// As listed: the root and every file version, newest first
    pub versions: Vec<Version>,
}

impl Document {
    pub fn has_versions(&self) -> bool {
        self.versions.len() > 1
    }

    /// The checksum Paperless 3 lists for the document's own file
    pub fn listed_checksum(&self) -> Option<&str> {
        self.versions
            .iter()
            .find(|v| v.is_root)
            .and_then(|v| v.checksum.as_deref())
    }

    /// `None` when the checksum isn't one, or a value doesn't fit its
    /// field's type or names no field
    pub fn into_records(
        self,
        checksum: &str,
        size: Option<u64>,
        field_types: &HashMap<i64, PaperlessFieldType>,
    ) -> Option<DocumentRecords> {
        let id = self.id;
        let checksum = PaperlessChecksum::parse(checksum)?;
        let mut rest = Vec::with_capacity(self.notes.len() + self.custom_fields.len());
        rest.extend(self.notes.into_iter().map(|note| {
            PaperlessRecord::Note(PaperlessNote {
                id: note.id,
                document: id,
                body: note.note,
                created: note.created,
                deleted: false,
            })
        }));
        for instance in self.custom_fields {
            let data_type = *field_types.get(&instance.field)?;
            rest.push(PaperlessRecord::CustomFieldInstance(
                PaperlessCustomFieldInstance {
                    document: id,
                    field: instance.field,
                    value: field_value(data_type, instance.value)?,
                },
            ));
        }
        let document = PaperlessDocument {
            id,
            title: self.title,
            content: self.content,
            mime_type: self.mime_type,
            original_filename: self.original_file_name,
            checksum,
            size,
            created: self.created,
            added: self.added,
            modified: self.modified,
            correspondent: self.correspondent,
            document_type: self.document_type,
            tags: self.tags,
            archive_serial_number: self.archive_serial_number,
            trashed: self.deleted_at.is_some(),
            version_of: None,
            original_handle: id.to_string(),
        };
        Some(DocumentRecords {
            document,
            rest,
            versions: self.versions,
        })
    }
}

/// A file version of `root`. Paperless 3 copies only the root's title,
/// created date and owner into a version, and numbers versions from 1 in the
/// order they were added, which is the reverse of `versions`.
pub(super) fn version_record(
    root: &PaperlessDocument,
    version: &Version,
    index: i64,
    content: String,
    file: Metadata,
) -> Option<PaperlessDocument> {
    Some(PaperlessDocument {
        id: version.id,
        title: root.title.clone(),
        content,
        mime_type: file.original_mime_type?,
        original_filename: file.original_filename,
        checksum: PaperlessChecksum::parse(&file.original_checksum)?,
        size: file.original_size,
        created: root.created,
        added: version.added,
        modified: version.added,
        correspondent: None,
        document_type: None,
        tags: Vec::new(),
        archive_serial_number: None,
        trashed: false,
        version_of: Some(PaperlessVersionInfo {
            root_document: root.id,
            index: Some(index),
            label: version.version_label.clone(),
        }),
        original_handle: version.id.to_string(),
    })
}

/// `Some(None)` for an empty value, `None` for one that doesn't fit the type
fn field_value(data_type: PaperlessFieldType, value: Value) -> Option<Option<PaperlessFieldValue>> {
    use PaperlessFieldType as T;
    use PaperlessFieldValue as V;
    if value.is_null() {
        return Some(None);
    }
    let text = |value: Value| match value {
        Value::String(s) => Some(s),
        _ => None,
    };
    let parsed = match data_type {
        T::String => V::Text(text(value)?),
        T::LongText => V::LongText(text(value)?),
        T::Url => V::Url(text(value)?),
        T::Monetary => V::Monetary(text(value)?),
        T::Date => V::Date(text(value)?.parse().ok()?),
        T::Boolean => V::Boolean(value.as_bool()?),
        T::Integer => V::Integer(value.as_i64()?),
        T::Float => V::Float(value.as_f64()?),
        T::Select => V::Select(match value {
            Value::String(id) => id,
            Value::Number(index) => index.as_i64()?.to_string(),
            _ => return None,
        }),
        T::DocumentLink => V::DocumentLink(serde_json::from_value(value).ok()?),
    };
    Some(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_the_field_type() {
        use PaperlessFieldType as T;
        use PaperlessFieldValue as V;
        let v = |t, json: &str| field_value(t, serde_json::from_str(json).unwrap());
        assert_eq!(v(T::Float, "5"), Some(Some(V::Float(5.0))));
        assert_eq!(v(T::Select, "\"abc\""), Some(Some(V::Select("abc".into()))));
        assert_eq!(v(T::Select, "1"), Some(Some(V::Select("1".into()))));
        assert_eq!(v(T::Integer, "null"), Some(None));
        assert_eq!(v(T::Integer, "\"3\""), None);
        assert_eq!(v(T::Date, "\"2024-02-30\""), None);
        assert_eq!(
            v(T::DocumentLink, "[1, 2]"),
            Some(Some(V::DocumentLink(vec![1, 2])))
        );
    }
}

//! Port for reading a Paperless-ngx library, from an export or a live instance

use std::collections::BTreeMap;
use std::io;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use futures::stream::BoxStream;
use futures::TryStreamExt;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

/// A Paperless library as the import engine sees it; every adapter yields the
/// same records
#[async_trait]
pub trait PaperlessReader: Send + Sync {
    async fn source_info(&self) -> Result<PaperlessSourceInfo, PaperlessReadError>;

    /// Streams every record without holding the whole library in memory.
    /// Taxonomy may arrive interleaved with documents, so callers join by ID.
    async fn scan(&self) -> Result<RecordStream, PaperlessReadError>;

    async fn open_original(
        &self,
        document: &PaperlessDocument,
    ) -> Result<OriginalFile, PaperlessReadError>;

    /// IDs of every document the source holds, trashed ones and file versions
    /// included; mirror sync compares these to find deletions
    async fn list_document_ids(&self) -> Result<Vec<i64>, PaperlessReadError> {
        self.scan()
            .await?
            .try_filter_map(|record| async move {
                Ok(match record {
                    PaperlessRecord::Document(doc) => Some(doc.id),
                    _ => None,
                })
            })
            .try_collect()
            .await
    }

    /// Records of models the import doesn't use, by model name, counted
    /// during the last completed scan
    fn skipped_models(&self) -> BTreeMap<String, u64> {
        BTreeMap::new()
    }
}

pub type RecordStream = BoxStream<'static, Result<PaperlessRecord, PaperlessReadError>>;

pub struct OriginalFile {
    pub size: Option<u64>,
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
}

#[derive(Debug, thiserror::Error)]
pub enum PaperlessReadError {
    #[error("{file} is missing")]
    Missing { file: String },
    #[error("{file} is not valid Paperless data: {message}")]
    Malformed { file: String, message: String },
    #[error("unsafe path: {path}")]
    UnsafePath { path: String },
    #[error("could not read {file}: {source}")]
    Io {
        file: String,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperlessSourceInfo {
    pub version: PaperlessVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PaperlessVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl PaperlessVersion {
    /// 2.16 made `created` a date; older versions aren't supported
    pub const MIN_SUPPORTED: Self = Self {
        major: 2,
        minor: 16,
        patch: 0,
    };

    /// Parses `X.Y.Z`, ignoring a pre-release or build suffix such as `-dev`
    pub fn parse(raw: &str) -> Option<Self> {
        let core = raw.trim().split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    pub fn is_supported(self) -> bool {
        self >= Self::MIN_SUPPORTED
    }
}

impl std::fmt::Display for PaperlessVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaperlessRecord {
    Correspondent(PaperlessNamed),
    DocumentType(PaperlessNamed),
    Tag(PaperlessTag),
    CustomField(PaperlessCustomField),
    CustomFieldInstance(PaperlessCustomFieldInstance),
    Note(PaperlessNote),
    Document(PaperlessDocument),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessNamed {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessTag {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub is_inbox_tag: bool,
    pub parent: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaperlessFieldType {
    String,
    LongText,
    Url,
    Date,
    Boolean,
    Integer,
    Float,
    Monetary,
    Select,
    DocumentLink,
}

impl PaperlessFieldType {
    /// Parses Paperless's `data_type` value
    pub fn from_paperless(raw: &str) -> Option<Self> {
        Some(match raw {
            "string" => Self::String,
            "longtext" => Self::LongText,
            "url" => Self::Url,
            "date" => Self::Date,
            "boolean" => Self::Boolean,
            "integer" => Self::Integer,
            "float" => Self::Float,
            "monetary" => Self::Monetary,
            "select" => Self::Select,
            "documentlink" => Self::DocumentLink,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessCustomField {
    pub id: i64,
    pub name: String,
    pub data_type: PaperlessFieldType,
    /// In Paperless order
    pub select_options: Vec<PaperlessSelectOption>,
    pub default_currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessSelectOption {
    /// Absent before select options had IDs, when values were option indexes
    pub id: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperlessCustomFieldInstance {
    pub id: i64,
    pub document: i64,
    pub field: i64,
    pub values: PaperlessFieldColumns,
    pub deleted: bool,
}

/// Paperless keeps one column per value type; the field's type picks the column
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaperlessFieldColumns {
    pub text: Option<String>,
    pub long_text: Option<String>,
    pub url: Option<String>,
    pub date: Option<NaiveDate>,
    pub boolean: Option<bool>,
    pub integer: Option<i64>,
    pub float: Option<f64>,
    /// Raw Paperless form: an optional ISO currency code, then the amount, as in `EUR123.45`
    pub monetary: Option<String>,
    /// Option ID, or the option index before options had IDs
    pub select: Option<String>,
    pub document_ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PaperlessFieldValue {
    Text(String),
    LongText(String),
    Url(String),
    Date(NaiveDate),
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Monetary(String),
    Select(String),
    DocumentLink(Vec<i64>),
}

impl PaperlessFieldColumns {
    /// The value in the column for `data_type`, if set
    pub fn value(&self, data_type: PaperlessFieldType) -> Option<PaperlessFieldValue> {
        use PaperlessFieldType as T;
        use PaperlessFieldValue as V;
        match data_type {
            T::String => self.text.clone().map(V::Text),
            T::LongText => self.long_text.clone().map(V::LongText),
            T::Url => self.url.clone().map(V::Url),
            T::Date => self.date.map(V::Date),
            T::Boolean => self.boolean.map(V::Boolean),
            T::Integer => self.integer.map(V::Integer),
            T::Float => self.float.map(V::Float),
            T::Monetary => self.monetary.clone().map(V::Monetary),
            T::Select => self.select.clone().map(V::Select),
            T::DocumentLink => self.document_ids.clone().map(V::DocumentLink),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessNote {
    pub id: i64,
    pub document: i64,
    pub body: String,
    pub created: DateTime<Utc>,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessDocument {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub mime_type: String,
    pub original_filename: Option<String>,
    pub checksum: PaperlessChecksum,
    /// Original size in bytes, when the source knows it without reading the file
    pub size: Option<u64>,
    pub created: NaiveDate,
    pub added: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub correspondent: Option<i64>,
    pub document_type: Option<i64>,
    pub tags: Vec<i64>,
    pub archive_serial_number: Option<i64>,
    pub trashed: bool,
    /// Set on a file version; the root document carries `None`
    pub version_of: Option<PaperlessVersionInfo>,
    /// Opaque to everyone but the adapter that produced the record, which
    /// uses it in [`PaperlessReader::open_original`]
    pub original_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessChecksum {
    /// Lowercase hex
    pub value: String,
    pub algorithm: ChecksumAlgorithm,
}

impl PaperlessChecksum {
    /// Picks the algorithm by length, since a v3 library can hold either for
    /// documents consumed under v2
    pub fn parse(raw: &str) -> Option<Self> {
        if !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let algorithm = match raw.len() {
            32 => ChecksumAlgorithm::Md5,
            64 => ChecksumAlgorithm::Sha256,
            _ => return None,
        };
        Some(Self {
            value: raw.to_ascii_lowercase(),
            algorithm,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperlessVersionInfo {
    pub root_document: i64,
    pub index: Option<i64>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Md5,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDigests {
    /// Lowercase hex digest in the source's algorithm
    pub source: String,
    pub sha256: String,
    pub size: u64,
}

/// Hashes `reader` with `algorithm` and SHA-256 in one pass
pub async fn digest_original(
    mut reader: impl AsyncRead + Unpin,
    algorithm: ChecksumAlgorithm,
) -> io::Result<FileDigests> {
    let mut sha256 = Sha256::new();
    let mut md5 = (algorithm == ChecksumAlgorithm::Md5).then(Md5::new);
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        sha256.update(&buf[..n]);
        if let Some(md5) = md5.as_mut() {
            md5.update(&buf[..n]);
        }
        size += n as u64;
    }
    let sha256 = hex::encode(sha256.finalize());
    let source = match md5 {
        Some(md5) => hex::encode(md5.finalize()),
        None => sha256.clone(),
    };
    Ok(FileDigests {
        source,
        sha256,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_gate_accepts_2_16_and_later() {
        let v = |s| PaperlessVersion::parse(s).unwrap();
        assert!(v("2.16.0").is_supported());
        assert!(v("2.20.15").is_supported());
        assert!(v("3.4.2").is_supported());
        assert!(v("2.20.0-dev").is_supported());
        assert!(!v("2.15.3").is_supported());
        assert!(!v("1.17.4").is_supported());
        assert_eq!(v("2.16"), v("2.16.0"));
    }

    #[test]
    fn version_parse_rejects_garbage() {
        for raw in ["", "two", "2", "2.x.1", "2.16.0.1"] {
            assert_eq!(PaperlessVersion::parse(raw), None, "{raw}");
        }
    }

    #[test]
    fn checksum_algorithm_follows_length() {
        let md5 = PaperlessChecksum::parse(&"A".repeat(32)).unwrap();
        assert_eq!(md5.algorithm, ChecksumAlgorithm::Md5);
        assert_eq!(md5.value, "a".repeat(32));
        assert_eq!(
            PaperlessChecksum::parse(&"b".repeat(64)).unwrap().algorithm,
            ChecksumAlgorithm::Sha256
        );
        assert_eq!(PaperlessChecksum::parse(&"a".repeat(40)), None);
        assert_eq!(PaperlessChecksum::parse(&"g".repeat(32)), None);
    }

    #[tokio::test]
    async fn digest_computes_source_algorithm_and_sha256_in_one_pass() {
        let md5 = digest_original(&b"abc"[..], ChecksumAlgorithm::Md5)
            .await
            .unwrap();
        assert_eq!(md5.source, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(md5.size, 3);

        let sha = digest_original(&b"abc"[..], ChecksumAlgorithm::Sha256)
            .await
            .unwrap();
        assert_eq!(sha.source, sha.sha256);
    }

    #[test]
    fn field_columns_pick_the_column_for_the_type() {
        let cols = PaperlessFieldColumns {
            monetary: Some("EUR12.50".into()),
            document_ids: Some(vec![3, 4]),
            ..Default::default()
        };
        assert_eq!(
            cols.value(PaperlessFieldType::Monetary),
            Some(PaperlessFieldValue::Monetary("EUR12.50".into()))
        );
        assert_eq!(
            cols.value(PaperlessFieldType::DocumentLink),
            Some(PaperlessFieldValue::DocumentLink(vec![3, 4]))
        );
        assert_eq!(cols.value(PaperlessFieldType::String), None);
    }
}

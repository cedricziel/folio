//! Reads a Paperless-ngx `document_exporter` directory from the filesystem
//!
//! The manifest is a JSON array of Django-serialized records. Each element is
//! deserialized on its own, so memory stays bounded by the largest record.
//! Records of models the import doesn't use, including users, mail accounts,
//! social tokens and authenticators, are skipped with [`IgnoredAny`] and never
//! materialized.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use folio_core::ports::{
    OriginalFile, PaperlessChecksum, PaperlessCustomField, PaperlessCustomFieldInstance,
    PaperlessDocument, PaperlessFieldColumns, PaperlessFieldType, PaperlessNamed, PaperlessNote,
    PaperlessReadError, PaperlessReader, PaperlessRecord, PaperlessSelectOption,
    PaperlessSourceInfo, PaperlessTag, PaperlessVersion, PaperlessVersionInfo, RecordStream,
};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::paths::resolve_original;

const MANIFEST: &str = "manifest.json";
const METADATA: &str = "metadata.json";
const SPLIT_SUFFIX: &str = "-manifest.json";
/// `--use-folder-prefix` directories that hold files, not manifests
const FILE_DIRS: [&str; 3] = ["originals", "archive", "thumbnails"];
const CHANNEL_DEPTH: usize = 256;

/// Export adapter for [`PaperlessReader`]; a document's `original_handle` is
/// its path relative to the export directory
pub struct PaperlessFsReader {
    dir: PathBuf,
    skipped: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl PaperlessFsReader {
    /// Opens an export directory; the caller checks it against the allowed roots first
    pub fn open(dir: &Path) -> Result<Self, PaperlessReadError> {
        let missing = || PaperlessReadError::Missing {
            file: dir.display().to_string(),
        };
        let dir = dir.canonicalize().map_err(|_| missing())?;
        if !dir.is_dir() {
            return Err(missing());
        }
        Ok(Self {
            dir,
            skipped: Arc::default(),
        })
    }
}

#[async_trait]
impl PaperlessReader for PaperlessFsReader {
    async fn source_info(&self) -> Result<PaperlessSourceInfo, PaperlessReadError> {
        let dir = self.dir.clone();
        blocking(move || read_metadata(&dir)).await
    }

    async fn scan(&self) -> Result<RecordStream, PaperlessReadError> {
        let dir = self.dir.clone();
        let mut records = blocking(move || ManifestRecords::open(dir)).await?;
        let skipped = Arc::clone(&self.skipped);
        let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_DEPTH);
        tokio::task::spawn_blocking(move || {
            let mut counts = BTreeMap::new();
            for item in records.by_ref() {
                let sent = match item {
                    Ok(Item::Skipped(model)) => {
                        *counts.entry(model).or_default() += 1;
                        continue;
                    }
                    Ok(Item::Record(record)) => tx.blocking_send(Ok(record)),
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        return;
                    }
                };
                if sent.is_err() {
                    return;
                }
            }
            *skipped.lock().unwrap_or_else(|e| e.into_inner()) = counts;
        });
        Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        })))
    }

    async fn open_original(
        &self,
        document: &PaperlessDocument,
    ) -> Result<OriginalFile, PaperlessReadError> {
        let dir = self.dir.clone();
        let handle = document.original_handle.clone();
        let (file, size) = blocking(move || {
            let file = open_in_export(&dir, &handle)?;
            let size = file.metadata().map_err(|e| io_error(&handle, e))?.len();
            Ok((file, size))
        })
        .await?;
        Ok(OriginalFile {
            size: Some(size),
            reader: Box::new(tokio::fs::File::from_std(file)),
        })
    }

    fn skipped_models(&self) -> BTreeMap<String, u64> {
        self.skipped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, PaperlessReadError> + Send + 'static,
) -> Result<T, PaperlessReadError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| PaperlessReadError::Io {
            file: "export".to_owned(),
            source: io::Error::other(e),
        })?
}

fn open_in_export(dir: &Path, relative: &str) -> Result<File, PaperlessReadError> {
    let path = resolve_original(dir, relative)?;
    File::open(path).map_err(|e| io_error(relative, e))
}

fn read_metadata(dir: &Path) -> Result<PaperlessSourceInfo, PaperlessReadError> {
    #[derive(Deserialize)]
    struct Raw {
        version: String,
    }

    let file = open_in_export(dir, METADATA)?;
    let raw: Raw =
        serde_json::from_reader(BufReader::new(file)).map_err(|e| json_error(METADATA, e))?;
    let version = PaperlessVersion::parse(&raw.version)
        .ok_or_else(|| malformed(METADATA, format!("unrecognized version {:?}", raw.version)))?;
    Ok(PaperlessSourceInfo { version })
}

fn split_manifests(dir: &Path) -> Result<Vec<String>, PaperlessReadError> {
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let label = relative.display().to_string();
        let entries = std::fs::read_dir(dir.join(&relative)).map_err(|e| io_error(&label, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_error(&label, e))?;
            let file_type = entry.file_type().map_err(|e| io_error(&label, e))?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = relative.join(&name);
            if file_type.is_dir() {
                let holds_files =
                    relative.as_os_str().is_empty() && FILE_DIRS.contains(&name.as_str());
                if !holds_files {
                    pending.push(path);
                }
            } else if file_type.is_file() && name.ends_with(SPLIT_SUFFIX) {
                if let Some(path) = path.to_str() {
                    found.push(path.to_owned());
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

enum Item {
    Record(PaperlessRecord),
    Skipped(String),
}

/// `manifest.json` first, then each `--split-manifest` file in path order
struct ManifestRecords {
    dir: PathBuf,
    current: Option<ManifestStream>,
    pending: VecDeque<String>,
}

impl ManifestRecords {
    fn open(dir: PathBuf) -> Result<Self, PaperlessReadError> {
        let main = ManifestStream::new(MANIFEST.to_owned(), open_in_export(&dir, MANIFEST)?);
        let pending = split_manifests(&dir)?.into();
        Ok(Self {
            dir,
            current: Some(main),
            pending,
        })
    }

    fn next_item(&mut self) -> Option<Result<Item, PaperlessReadError>> {
        loop {
            if let Some(stream) = self.current.as_mut() {
                match stream.next_record() {
                    Some(item) => return Some(item),
                    None => self.current = None,
                }
            }
            let next = self.pending.pop_front()?;
            match open_in_export(&self.dir, &next) {
                Ok(file) => self.current = Some(ManifestStream::new(next, file)),
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

impl Iterator for ManifestRecords {
    type Item = Result<Item, PaperlessReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut item = self.next_item();
        match &mut item {
            Some(Ok(Item::Record(PaperlessRecord::Document(doc)))) => {
                doc.size = resolve_original(&self.dir, &doc.original_handle)
                    .and_then(|path| std::fs::metadata(path).map_err(|e| io_error("", e)))
                    .ok()
                    .map(|meta| meta.len());
            }
            Some(Err(_)) => {
                self.current = None;
                self.pending.clear();
            }
            _ => {}
        }
        item
    }
}

#[derive(Clone, Copy)]
enum Position {
    BeforeArray,
    FirstElement,
    AfterElement,
    Done,
}

/// Walks the array framing by hand and hands each element to serde_json
struct ManifestStream {
    label: String,
    reader: BufReader<File>,
    position: Position,
}

impl ManifestStream {
    fn new(label: String, file: File) -> Self {
        Self {
            label,
            reader: BufReader::with_capacity(64 * 1024, file),
            position: Position::BeforeArray,
        }
    }

    fn next_record(&mut self) -> Option<Result<Item, PaperlessReadError>> {
        let result = self.advance();
        if !matches!(result, Some(Ok(_))) {
            self.position = Position::Done;
        }
        result
    }

    fn advance(&mut self) -> Option<Result<Item, PaperlessReadError>> {
        loop {
            let byte = match self.position {
                Position::Done => return None,
                _ => match self.peek_non_whitespace() {
                    Ok(byte) => byte,
                    Err(e) => return Some(Err(e)),
                },
            };
            match (self.position, byte) {
                (Position::BeforeArray, Some(b'[')) => {
                    self.reader.consume(1);
                    self.position = Position::FirstElement;
                }
                (Position::FirstElement | Position::AfterElement, Some(b']')) => {
                    self.reader.consume(1);
                    return match self.peek_non_whitespace() {
                        Ok(None) => None,
                        Ok(Some(_)) => Some(Err(self.malformed("trailing data after the array"))),
                        Err(e) => Some(Err(e)),
                    };
                }
                (Position::AfterElement, Some(b',')) => {
                    self.reader.consume(1);
                    self.position = Position::FirstElement;
                }
                (Position::FirstElement, Some(_)) => {
                    self.position = Position::AfterElement;
                    return Some(self.read_element());
                }
                (Position::BeforeArray, _) => {
                    return Some(Err(self.malformed("expected a JSON array of records")))
                }
                (_, None) => return Some(Err(self.malformed("unexpected end of file"))),
                (_, Some(_)) => return Some(Err(self.malformed("expected `,` or `]`"))),
            }
        }
    }

    /// Reads one object; serde_json stops right after its closing brace
    fn read_element(&mut self) -> Result<Item, PaperlessReadError> {
        let mut de = serde_json::Deserializer::from_reader(&mut self.reader);
        RecordSeed
            .deserialize(&mut de)
            .map_err(|e| json_error(&self.label, e))
    }

    fn peek_non_whitespace(&mut self) -> Result<Option<u8>, PaperlessReadError> {
        loop {
            let buf = self
                .reader
                .fill_buf()
                .map_err(|e| io_error(&self.label, e))?;
            let Some(&byte) = buf.first() else {
                return Ok(None);
            };
            if byte.is_ascii_whitespace() {
                self.reader.consume(1);
            } else {
                return Ok(Some(byte));
            }
        }
    }

    fn malformed(&self, message: &str) -> PaperlessReadError {
        malformed(&self.label, message.to_owned())
    }
}

fn malformed(file: &str, message: String) -> PaperlessReadError {
    PaperlessReadError::Malformed {
        file: file.to_owned(),
        message,
    }
}

fn io_error(file: &str, source: io::Error) -> PaperlessReadError {
    if source.kind() == io::ErrorKind::NotFound {
        PaperlessReadError::Missing {
            file: file.to_owned(),
        }
    } else {
        PaperlessReadError::Io {
            file: file.to_owned(),
            source,
        }
    }
}

fn json_error(file: &str, error: serde_json::Error) -> PaperlessReadError {
    if error.is_io() {
        io_error(file, error.into())
    } else {
        malformed(file, error.to_string())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Correspondent,
    DocumentType,
    Tag,
    CustomField,
    CustomFieldInstance,
    Note,
    Document,
}

impl Model {
    fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "documents.correspondent" => Self::Correspondent,
            "documents.documenttype" => Self::DocumentType,
            "documents.tag" => Self::Tag,
            "documents.customfield" => Self::CustomField,
            "documents.customfieldinstance" => Self::CustomFieldInstance,
            "documents.note" => Self::Note,
            "documents.document" => Self::Document,
            _ => return None,
        })
    }
}

enum Fields {
    Correspondent(NamedFields),
    DocumentType(NamedFields),
    Tag(TagFields),
    CustomField(CustomFieldFields),
    CustomFieldInstance(InstanceFields),
    Note(NoteFields),
    Document(Box<DocumentFields>),
}

struct RecordSeed;

impl<'de> DeserializeSeed<'de> for RecordSeed {
    type Value = Item;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Item, D::Error> {
        deserializer.deserialize_map(RecordVisitor)
    }
}

struct RecordVisitor;

impl<'de> Visitor<'de> for RecordVisitor {
    type Value = Item;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a Django-serialized record")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Item, A::Error> {
        let mut label: Option<String> = None;
        let mut model = None;
        let mut pk: Option<i64> = None;
        let mut fields = None;
        let mut original_path: Option<String> = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "model" => {
                    let value: String = map.next_value()?;
                    model = Model::from_label(&value);
                    label = Some(value);
                }
                "pk" if model.is_some() => pk = Some(map.next_value()?),
                "fields" => match (&label, model) {
                    (None, _) => {
                        return Err(de::Error::custom("record fields come before its model"))
                    }
                    (Some(_), None) => {
                        map.next_value::<IgnoredAny>()?;
                    }
                    (Some(_), Some(model)) => fields = Some(read_fields(model, &mut map)?),
                },
                "__exported_file_name__" if model == Some(Model::Document) => {
                    original_path = Some(map.next_value()?)
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        let label = label.ok_or_else(|| de::Error::missing_field("model"))?;
        if model.is_none() {
            return Ok(Item::Skipped(label));
        }
        let id = pk.ok_or_else(|| de::Error::missing_field("pk"))?;
        let fields = fields.ok_or_else(|| de::Error::missing_field("fields"))?;
        Ok(Item::Record(match fields {
            Fields::Correspondent(f) => {
                PaperlessRecord::Correspondent(PaperlessNamed { id, name: f.name })
            }
            Fields::DocumentType(f) => {
                PaperlessRecord::DocumentType(PaperlessNamed { id, name: f.name })
            }
            Fields::Tag(f) => PaperlessRecord::Tag(PaperlessTag {
                id,
                name: f.name,
                color: f.color,
                is_inbox_tag: f.is_inbox_tag,
                parent: f.tn_parent,
            }),
            Fields::CustomField(f) => PaperlessRecord::CustomField(f.into_record(id)?),
            Fields::CustomFieldInstance(f) => {
                PaperlessRecord::CustomFieldInstance(f.into_record(id))
            }
            Fields::Note(f) => PaperlessRecord::Note(PaperlessNote {
                id,
                document: f.document,
                body: f.note,
                created: f.created,
                deleted: f.deleted_at.is_some(),
            }),
            Fields::Document(f) => {
                let original_path = original_path
                    .ok_or_else(|| de::Error::missing_field("__exported_file_name__"))?;
                PaperlessRecord::Document(f.into_record(id, original_path)?)
            }
        }))
    }
}

fn read_fields<'de, A: MapAccess<'de>>(model: Model, map: &mut A) -> Result<Fields, A::Error> {
    Ok(match model {
        Model::Correspondent => Fields::Correspondent(map.next_value()?),
        Model::DocumentType => Fields::DocumentType(map.next_value()?),
        Model::Tag => Fields::Tag(map.next_value()?),
        Model::CustomField => Fields::CustomField(map.next_value()?),
        Model::CustomFieldInstance => Fields::CustomFieldInstance(map.next_value()?),
        Model::Note => Fields::Note(map.next_value()?),
        Model::Document => Fields::Document(Box::new(map.next_value()?)),
    })
}

#[derive(Deserialize)]
struct NamedFields {
    name: String,
}

#[derive(Deserialize)]
struct TagFields {
    name: String,
    color: String,
    #[serde(default)]
    is_inbox_tag: bool,
    #[serde(default)]
    tn_parent: Option<i64>,
}

#[derive(Deserialize)]
struct CustomFieldFields {
    name: String,
    data_type: String,
    #[serde(default)]
    extra_data: Option<CustomFieldExtra>,
}

#[derive(Deserialize, Default)]
struct CustomFieldExtra {
    #[serde(default)]
    select_options: Option<Vec<SelectOption>>,
    #[serde(default)]
    default_currency: Option<String>,
}

/// Before option IDs, select options were bare labels
#[derive(Deserialize)]
#[serde(untagged)]
enum SelectOption {
    WithId { id: String, label: String },
    Label(String),
}

impl CustomFieldFields {
    fn into_record<E: de::Error>(self, id: i64) -> Result<PaperlessCustomField, E> {
        let data_type = PaperlessFieldType::from_paperless(&self.data_type).ok_or_else(|| {
            E::custom(format!(
                "unknown custom field data type {:?}",
                self.data_type
            ))
        })?;
        let extra = self.extra_data.unwrap_or_default();
        Ok(PaperlessCustomField {
            id,
            name: self.name,
            data_type,
            select_options: extra
                .select_options
                .unwrap_or_default()
                .into_iter()
                .map(|option| match option {
                    SelectOption::WithId { id, label } => PaperlessSelectOption {
                        id: Some(id),
                        label,
                    },
                    SelectOption::Label(label) => PaperlessSelectOption { id: None, label },
                })
                .collect(),
            default_currency: extra.default_currency.filter(|c| !c.is_empty()),
        })
    }
}

/// Select values were option indexes before option IDs
#[derive(Deserialize)]
#[serde(untagged)]
enum SelectValue {
    Id(String),
    Index(i64),
}

#[derive(Deserialize)]
struct InstanceFields {
    document: i64,
    field: i64,
    #[serde(default)]
    deleted_at: Option<String>,
    #[serde(default)]
    value_text: Option<String>,
    #[serde(default)]
    value_long_text: Option<String>,
    #[serde(default)]
    value_url: Option<String>,
    #[serde(default)]
    value_date: Option<NaiveDate>,
    #[serde(default)]
    value_bool: Option<bool>,
    #[serde(default)]
    value_int: Option<i64>,
    #[serde(default)]
    value_float: Option<f64>,
    #[serde(default)]
    value_monetary: Option<String>,
    #[serde(default)]
    value_select: Option<SelectValue>,
    #[serde(default)]
    value_document_ids: Option<Vec<i64>>,
}

impl InstanceFields {
    fn into_record(self, id: i64) -> PaperlessCustomFieldInstance {
        PaperlessCustomFieldInstance {
            id,
            document: self.document,
            field: self.field,
            deleted: self.deleted_at.is_some(),
            values: PaperlessFieldColumns {
                text: self.value_text,
                long_text: self.value_long_text,
                url: self.value_url,
                date: self.value_date,
                boolean: self.value_bool,
                integer: self.value_int,
                float: self.value_float,
                monetary: self.value_monetary,
                select: self.value_select.map(|v| match v {
                    SelectValue::Id(id) => id,
                    SelectValue::Index(index) => index.to_string(),
                }),
                document_ids: self.value_document_ids,
            },
        }
    }
}

#[derive(Deserialize)]
struct NoteFields {
    note: String,
    created: DateTime<Utc>,
    document: i64,
    #[serde(default)]
    deleted_at: Option<String>,
}

#[derive(Deserialize)]
struct DocumentFields {
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    mime_type: String,
    checksum: String,
    created: NaiveDate,
    added: DateTime<Utc>,
    modified: DateTime<Utc>,
    #[serde(default)]
    correspondent: Option<i64>,
    #[serde(default)]
    document_type: Option<i64>,
    #[serde(default)]
    tags: Vec<i64>,
    #[serde(default)]
    archive_serial_number: Option<i64>,
    #[serde(default)]
    original_filename: Option<String>,
    #[serde(default)]
    deleted_at: Option<String>,
    #[serde(default)]
    root_document: Option<i64>,
    #[serde(default)]
    version_index: Option<i64>,
    #[serde(default)]
    version_label: Option<String>,
}

impl DocumentFields {
    fn into_record<E: de::Error>(
        self,
        id: i64,
        original_path: String,
    ) -> Result<PaperlessDocument, E> {
        let checksum = PaperlessChecksum::parse(&self.checksum)
            .ok_or_else(|| E::custom(format!("document {id} has an unrecognized checksum")))?;
        Ok(PaperlessDocument {
            id,
            title: self.title,
            content: self.content,
            mime_type: self.mime_type,
            checksum,
            size: None,
            created: self.created,
            added: self.added,
            modified: self.modified,
            correspondent: self.correspondent,
            document_type: self.document_type,
            tags: self.tags,
            archive_serial_number: self.archive_serial_number,
            original_filename: self.original_filename,
            trashed: self.deleted_at.is_some(),
            version_of: self
                .root_document
                .map(|root_document| PaperlessVersionInfo {
                    root_document,
                    index: self.version_index,
                    label: self.version_label,
                }),
            original_handle: original_path,
        })
    }
}

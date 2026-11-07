//! Reads a Paperless-ngx `document_exporter` directory from the filesystem
//!
//! The manifest is a JSON array of Django-serialized records. Each element is
//! deserialized on its own, so memory stays bounded by the largest record.
//! Records of models the import doesn't use, including users, mail accounts,
//! social tokens and authenticators, are skipped with [`IgnoredAny`] and never
//! materialized.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use folio_core::ports::{
    CompleteIdSet, OriginalFile, PaperlessChecksum, PaperlessCustomField,
    PaperlessCustomFieldInstance, PaperlessDocument, PaperlessFieldType, PaperlessFieldValue,
    PaperlessNamed, PaperlessNote, PaperlessReadError, PaperlessReader, PaperlessRecord,
    PaperlessSelectOption, PaperlessSourceInfo, PaperlessTag, PaperlessVersion,
    PaperlessVersionInfo, RecordStream,
};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::paths::{io_error, resolve_original};

const MANIFEST: &str = "manifest.json";
const METADATA: &str = "metadata.json";
const SPLIT_SUFFIX: &[u8] = b"-manifest.json";
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
            what: dir.display().to_string(),
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

    /// Records of models the import doesn't use, by Django model label,
    /// counted during the last scan that ran to the end
    pub fn skipped_models(&self) -> BTreeMap<String, u64> {
        self.skipped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
        let mut records = blocking(move || ManifestRecords::open(dir, Mode::Full)).await?;
        let skipped = Arc::clone(&self.skipped);
        let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_DEPTH);
        tokio::task::spawn_blocking(move || {
            let mut counts = BTreeMap::new();
            while let Some(item) = records.next_item() {
                match item {
                    Ok(Item::Record(record)) => {
                        if tx.blocking_send(Ok(record)).is_err() {
                            return;
                        }
                    }
                    Ok(Item::Skipped(model)) => *counts.entry(model).or_default() += 1,
                    Ok(Item::Omitted | Item::DocumentId(_)) => {}
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        return;
                    }
                }
                if tx.is_closed() {
                    return;
                }
            }
            *skipped.lock().unwrap_or_else(|e| e.into_inner()) = counts;
        });
        Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        })))
    }

    async fn list_document_ids(&self) -> Result<CompleteIdSet, PaperlessReadError> {
        let dir = self.dir.clone();
        blocking(move || {
            let mut records = ManifestRecords::open(dir, Mode::DocumentIds)?;
            let mut ids = BTreeSet::new();
            while let Some(item) = records.next_item() {
                if let Item::DocumentId(id) = item? {
                    ids.insert(id);
                }
            }
            Ok(CompleteIdSet::from_complete_listing(ids))
        })
        .await
    }

    async fn open_original(
        &self,
        document: &PaperlessDocument,
    ) -> Result<OriginalFile, PaperlessReadError> {
        let dir = self.dir.clone();
        let handle = document.original_handle.clone();
        let (file, size) = blocking(move || {
            let (path, meta) = resolve_original(&dir, &handle)?;
            let file = File::open(path).map_err(|e| io_error(&handle, e))?;
            Ok((file, meta.len()))
        })
        .await?;
        Ok(OriginalFile {
            size: Some(size),
            reader: Box::new(tokio::fs::File::from_std(file)),
        })
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, PaperlessReadError> + Send + 'static,
) -> Result<T, PaperlessReadError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| PaperlessReadError::Io {
            what: "export".to_owned(),
            source: io::Error::other(e),
        })?
}

fn open_in_export(dir: &Path, relative: &str) -> Result<File, PaperlessReadError> {
    let (path, _) = resolve_original(dir, relative)?;
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

/// `--split-manifest` files, in path order; symlinks are never followed
fn split_manifests(dir: &Path) -> Result<Vec<String>, PaperlessReadError> {
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let label = relative.display().to_string();
        let entries = std::fs::read_dir(dir.join(&relative)).map_err(|e| io_error(&label, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_error(&label, e))?;
            let file_type = entry.file_type().map_err(|e| io_error(&label, e))?;
            let name = entry.file_name();
            if file_type.is_dir() {
                let holds_files = relative.as_os_str().is_empty()
                    && FILE_DIRS.iter().any(|d| name.as_os_str() == *d);
                if !holds_files {
                    pending.push(relative.join(&name));
                }
            } else if file_type.is_file() && name.as_encoded_bytes().ends_with(SPLIT_SUFFIX) {
                if let Some(path) = relative.join(&name).to_str() {
                    found.push(path.to_owned());
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Full,
    DocumentIds,
}

enum Item {
    Record(PaperlessRecord),
    DocumentId(i64),
    Skipped(String),
    /// Read but not passed on: deleted field values, or anything but
    /// documents when only IDs are wanted
    Omitted,
}

/// `manifest.json` first, then each `--split-manifest` file. Custom fields
/// always precede their values, so their types are known by then.
struct ManifestRecords {
    dir: PathBuf,
    current: Option<ManifestStream>,
    pending: VecDeque<String>,
    context: Context,
}

struct Context {
    mode: Mode,
    field_types: HashMap<i64, PaperlessFieldType>,
}

impl ManifestRecords {
    fn open(dir: PathBuf, mode: Mode) -> Result<Self, PaperlessReadError> {
        let main = ManifestStream::new(MANIFEST.to_owned(), open_in_export(&dir, MANIFEST)?);
        let pending = split_manifests(&dir)?.into();
        Ok(Self {
            dir,
            current: Some(main),
            pending,
            context: Context {
                mode,
                field_types: HashMap::new(),
            },
        })
    }

    /// Callers stop at the first error
    fn next_item(&mut self) -> Option<Result<Item, PaperlessReadError>> {
        loop {
            if let Some(stream) = self.current.as_mut() {
                match stream.next(&mut self.context) {
                    Some(Ok(Item::Record(PaperlessRecord::Document(mut doc)))) => {
                        doc.size = resolve_original(&self.dir, &doc.original_handle)
                            .ok()
                            .map(|(_, meta)| meta.len());
                        return Some(Ok(Item::Record(PaperlessRecord::Document(doc))));
                    }
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

#[derive(Clone, Copy)]
enum Position {
    BeforeArray,
    FirstElement,
    AfterElement,
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

    fn next(&mut self, context: &mut Context) -> Option<Result<Item, PaperlessReadError>> {
        loop {
            let byte = match self.peek_non_whitespace() {
                Ok(byte) => byte,
                Err(e) => return Some(Err(e)),
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
                    return Some(self.read_element(context));
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
    fn read_element(&mut self, context: &mut Context) -> Result<Item, PaperlessReadError> {
        let mut de = serde_json::Deserializer::from_reader(&mut self.reader);
        RecordSeed(context)
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

fn malformed(what: &str, message: String) -> PaperlessReadError {
    PaperlessReadError::Malformed {
        what: what.to_owned(),
        message,
    }
}

fn json_error(what: &str, error: serde_json::Error) -> PaperlessReadError {
    if error.is_io() {
        io_error(what, error.into())
    } else {
        malformed(what, error.to_string())
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

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum Key {
    Model,
    Pk,
    Fields,
    #[serde(rename = "__exported_file_name__")]
    ExportedFileName,
    #[serde(other)]
    Other,
}

struct RecordSeed<'a>(&'a mut Context);

impl<'de> DeserializeSeed<'de> for RecordSeed<'_> {
    type Value = Item;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Item, D::Error> {
        deserializer.deserialize_map(self)
    }
}

/// Paperless writes `model`, `pk`, `fields`, then any exporter keys; the
/// model must be known before `fields` so unused records are never read
impl<'de> Visitor<'de> for RecordSeed<'_> {
    type Value = Item;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a Django-serialized record")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Item, A::Error> {
        let context = self.0;
        let mut kind: Option<Result<Model, String>> = None;
        let mut pk: Option<i64> = None;
        let mut item = None;
        let mut original_handle: Option<String> = None;

        while let Some(key) = map.next_key::<Key>()? {
            match key {
                Key::Model => {
                    let label: String = map.next_value()?;
                    kind = Some(Model::from_label(&label).ok_or(label));
                }
                Key::Pk if matches!(kind, Some(Ok(_))) => pk = Some(map.next_value()?),
                Key::Fields => match &kind {
                    None => return Err(de::Error::custom("record fields come before its model")),
                    Some(Err(_)) => {
                        map.next_value::<IgnoredAny>()?;
                    }
                    Some(Ok(model)) => {
                        let id = pk
                            .ok_or_else(|| de::Error::custom("record fields come before its pk"))?;
                        item = Some(read_fields(*model, id, context, &mut map)?);
                    }
                },
                Key::ExportedFileName if kind == Some(Ok(Model::Document)) => {
                    original_handle = Some(map.next_value()?)
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        match kind {
            None => Err(de::Error::missing_field("model")),
            Some(Err(label)) => Ok(Item::Skipped(label)),
            Some(Ok(_)) => match item.ok_or_else(|| de::Error::missing_field("fields"))? {
                Item::Record(PaperlessRecord::Document(mut doc)) => {
                    doc.original_handle = original_handle
                        .ok_or_else(|| de::Error::missing_field("__exported_file_name__"))?;
                    Ok(Item::Record(PaperlessRecord::Document(doc)))
                }
                item => Ok(item),
            },
        }
    }
}

fn read_fields<'de, A: MapAccess<'de>>(
    model: Model,
    id: i64,
    context: &mut Context,
    map: &mut A,
) -> Result<Item, A::Error> {
    if context.mode == Mode::DocumentIds {
        map.next_value::<IgnoredAny>()?;
        return Ok(match model {
            Model::Document => Item::DocumentId(id),
            _ => Item::Omitted,
        });
    }
    let record = match model {
        Model::Correspondent => {
            let NamedFields { name } = map.next_value()?;
            PaperlessRecord::Correspondent(PaperlessNamed { id, name })
        }
        Model::DocumentType => {
            let NamedFields { name } = map.next_value()?;
            PaperlessRecord::DocumentType(PaperlessNamed { id, name })
        }
        Model::Tag => {
            let f: TagFields = map.next_value()?;
            PaperlessRecord::Tag(PaperlessTag {
                id,
                name: f.name,
                color: f.color,
                is_inbox_tag: f.is_inbox_tag,
                parent: f.tn_parent,
            })
        }
        Model::CustomField => {
            let field = map.next_value::<CustomFieldFields>()?.into_record(id);
            context.field_types.insert(id, field.data_type);
            PaperlessRecord::CustomField(field)
        }
        Model::CustomFieldInstance => {
            let f: InstanceFields = map.next_value()?;
            if f.deleted_at.is_some() {
                return Ok(Item::Omitted);
            }
            let data_type = *context.field_types.get(&f.field).ok_or_else(|| {
                de::Error::custom(format!("value for unknown custom field {}", f.field))
            })?;
            PaperlessRecord::CustomFieldInstance(PaperlessCustomFieldInstance {
                document: f.document,
                field: f.field,
                value: f.into_value(data_type),
            })
        }
        Model::Note => {
            let f: NoteFields = map.next_value()?;
            PaperlessRecord::Note(PaperlessNote {
                id,
                document: f.document,
                body: f.note,
                created: f.created,
                deleted: f.deleted_at.is_some(),
            })
        }
        Model::Document => PaperlessRecord::Document(Box::new(
            map.next_value::<DocumentFields>()?.into_record(id)?,
        )),
    };
    Ok(Item::Record(record))
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
    tn_parent: Option<i64>,
}

#[derive(Deserialize)]
struct CustomFieldFields {
    name: String,
    data_type: PaperlessFieldType,
    extra_data: Option<CustomFieldExtra>,
}

#[derive(Deserialize, Default)]
struct CustomFieldExtra {
    select_options: Option<Vec<SelectOption>>,
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
    fn into_record(self, id: i64) -> PaperlessCustomField {
        let extra = self.extra_data.unwrap_or_default();
        PaperlessCustomField {
            id,
            name: self.name,
            data_type: self.data_type,
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
        }
    }
}

/// Select values were option indexes before option IDs
#[derive(Deserialize)]
#[serde(untagged)]
enum SelectValue {
    Id(String),
    Index(i64),
}

/// Paperless keeps one column per value type; the field's type picks the column
#[derive(Deserialize)]
struct InstanceFields {
    document: i64,
    field: i64,
    deleted_at: Option<String>,
    value_text: Option<String>,
    value_long_text: Option<String>,
    value_url: Option<String>,
    value_date: Option<NaiveDate>,
    value_bool: Option<bool>,
    value_int: Option<i64>,
    value_float: Option<f64>,
    value_monetary: Option<String>,
    value_select: Option<SelectValue>,
    value_document_ids: Option<Vec<i64>>,
}

impl InstanceFields {
    fn into_value(self, data_type: PaperlessFieldType) -> Option<PaperlessFieldValue> {
        use PaperlessFieldType as T;
        use PaperlessFieldValue as V;
        match data_type {
            T::String => self.value_text.map(V::Text),
            T::LongText => self.value_long_text.map(V::LongText),
            T::Url => self.value_url.map(V::Url),
            T::Date => self.value_date.map(V::Date),
            T::Boolean => self.value_bool.map(V::Boolean),
            T::Integer => self.value_int.map(V::Integer),
            T::Float => self.value_float.map(V::Float),
            T::Monetary => self.value_monetary.map(V::Monetary),
            T::Select => self.value_select.map(|v| {
                V::Select(match v {
                    SelectValue::Id(id) => id,
                    SelectValue::Index(index) => index.to_string(),
                })
            }),
            T::DocumentLink => self.value_document_ids.map(V::DocumentLink),
        }
    }
}

#[derive(Deserialize)]
struct NoteFields {
    note: String,
    created: DateTime<Utc>,
    document: i64,
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
    correspondent: Option<i64>,
    document_type: Option<i64>,
    #[serde(default)]
    tags: Vec<i64>,
    archive_serial_number: Option<i64>,
    original_filename: Option<String>,
    deleted_at: Option<String>,
    root_document: Option<i64>,
    version_index: Option<i64>,
    version_label: Option<String>,
}

impl DocumentFields {
    /// The handle is set once `__exported_file_name__`, which follows the fields, is read
    fn into_record<E: de::Error>(self, id: i64) -> Result<PaperlessDocument, E> {
        let checksum = PaperlessChecksum::parse(&self.checksum)
            .ok_or_else(|| E::custom(format!("document {id} has an unrecognized checksum")))?;
        Ok(PaperlessDocument {
            id,
            title: self.title,
            content: self.content,
            mime_type: self.mime_type,
            original_filename: self.original_filename,
            checksum,
            size: None,
            created: self.created,
            added: self.added,
            modified: self.modified,
            correspondent: self.correspondent,
            document_type: self.document_type,
            tags: self.tags,
            archive_serial_number: self.archive_serial_number,
            trashed: self.deleted_at.is_some(),
            version_of: self
                .root_document
                .map(|root_document| PaperlessVersionInfo {
                    root_document,
                    index: self.version_index,
                    label: self.version_label,
                }),
            original_handle: String::new(),
        })
    }
}

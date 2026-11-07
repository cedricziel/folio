//! Reads a Paperless-ngx `document_exporter` directory from the filesystem
//!
//! The manifest is a JSON array of Django-serialized records. Each element is
//! deserialized on its own, so memory stays bounded by the largest record.
//! Records of models the import doesn't use, including users, mail accounts,
//! social tokens and authenticators, are skipped with [`IgnoredAny`] and never
//! materialized.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use folio_core::ports::{
    CompleteIdSet, InvalidDocumentReason, MalformedKind, OriginalFile, PaperlessChecksum,
    PaperlessCustomField, PaperlessCustomFieldInstance, PaperlessDocument, PaperlessFieldType,
    PaperlessFieldValue, PaperlessInvalidDocument, PaperlessNamed, PaperlessNote,
    PaperlessReadError, PaperlessReader, PaperlessRecord, PaperlessSelectOption,
    PaperlessSourceInfo, PaperlessTag, PaperlessVersion, PaperlessVersionInfo, RecordStream,
    SourceLocation,
};
use rustix::fs::{statat, AtFlags, FileType};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::error::Category;

use super::paths::{io_error, open_original_file, open_subdir, ImportDir};

const MANIFEST: &str = "manifest.json";
const METADATA: &str = "metadata.json";
const SPLIT_SUFFIX: &[u8] = b"-manifest.json";
/// `--use-folder-prefix` directories that hold files, not manifests
const FILE_DIRS: [&str; 3] = ["originals", "archive", "thumbnails"];
const CHANNEL_DEPTH: usize = 256;
/// How deep split-manifest discovery descends below the export directory
const MAX_DEPTH: usize = 16;

/// Export adapter for [`PaperlessReader`]; a document's `original_handle` is
/// its path relative to the export directory. The directory is opened once,
/// and every later read goes through that handle, so replacing the path
/// afterwards can't redirect the reader.
pub struct PaperlessFsReader {
    dir: Arc<File>,
    skipped: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl PaperlessFsReader {
    /// Reads the export directory the root guard opened
    pub fn new(dir: ImportDir) -> Self {
        Self {
            dir: Arc::new(dir.into_file()),
            skipped: Arc::default(),
        }
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
        let dir = Arc::clone(&self.dir);
        blocking(move || read_metadata(&dir)).await
    }

    async fn scan(&self) -> Result<RecordStream, PaperlessReadError> {
        let dir = Arc::clone(&self.dir);
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
                    Ok(Item::Omitted | Item::DocumentId(_) | Item::DuplicateDocument) => {}
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
        let dir = Arc::clone(&self.dir);
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
        let dir = Arc::clone(&self.dir);
        let handle = document.original_handle.clone();
        let (file, size) = blocking(move || open_original_file(&dir, &handle)).await?;
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

fn open_in_export(dir: &File, relative: &str) -> Result<File, PaperlessReadError> {
    open_original_file(dir, relative).map(|(file, _)| file)
}

fn read_metadata(dir: &File) -> Result<PaperlessSourceInfo, PaperlessReadError> {
    #[derive(Deserialize)]
    struct Raw {
        version: String,
    }

    let file = open_in_export(dir, METADATA)?;
    let location = || SourceLocation::file(METADATA);
    let raw: Raw =
        serde_json::from_reader(BufReader::new(file)).map_err(|e| json_error(e, location()))?;
    let version = PaperlessVersion::parse(&raw.version)
        .ok_or_else(|| malformed(MalformedKind::InvalidVersion, location()))?;
    Ok(PaperlessSourceInfo { version })
}

/// `--split-manifest` files, in path order. Every directory is opened
/// relative to its parent and symlinks are never followed. Like Paperless's
/// `**/*-manifest.json` glob, the whole tree is searched, except the file
/// directories of a `--use-folder-prefix` export, recognized by its `json/`.
fn split_manifests(dir: &File) -> Result<Vec<String>, PaperlessReadError> {
    let folder_prefix = matches!(
        statat(dir, "json", AtFlags::SYMLINK_NOFOLLOW),
        Ok(stat) if FileType::from_raw_mode(stat.st_mode as _) == FileType::Directory
    );
    let mut found = Vec::new();
    walk_for_manifests(dir, Path::new(""), 0, folder_prefix, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk_for_manifests(
    dir: &File,
    relative: &Path,
    depth: usize,
    folder_prefix: bool,
    found: &mut Vec<String>,
) -> Result<(), PaperlessReadError> {
    let label = relative.display().to_string();
    let fail = |errno: rustix::io::Errno| io_error(&label, errno.into());
    for entry in rustix::fs::Dir::read_from(dir).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        let file_type = match entry.file_type() {
            FileType::Unknown => FileType::from_raw_mode(
                statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(fail)?
                    .st_mode as _,
            ),
            known => known,
        };
        let path = relative.join(name);
        match file_type {
            FileType::Directory => {
                let holds_files =
                    folder_prefix && depth == 0 && FILE_DIRS.iter().any(|d| name == *d);
                if holds_files {
                    continue;
                }
                if depth + 1 >= MAX_DEPTH {
                    return Err(malformed(
                        MalformedKind::TooDeep,
                        SourceLocation::file(&path.display().to_string()),
                    ));
                }
                let child = File::from(open_subdir(dir, name).map_err(fail)?);
                walk_for_manifests(&child, &path, depth + 1, folder_prefix, found)?;
            }
            FileType::RegularFile if name.as_bytes().ends_with(SPLIT_SUFFIX) => {
                let path = path.to_str().ok_or_else(|| {
                    malformed(
                        MalformedKind::NonUtf8Name,
                        SourceLocation::file(&path.display().to_string()),
                    )
                })?;
                found.push(path.to_owned());
            }
            _ => {}
        }
    }
    Ok(())
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
    /// A document ID seen before; fails the read
    DuplicateDocument,
    /// Read but not passed on: deleted field values, or anything but
    /// documents when only IDs are wanted
    Omitted,
}

/// `manifest.json` first, then each `--split-manifest` file. Custom fields
/// always precede their values, so their types are known by then.
struct ManifestRecords {
    dir: Arc<File>,
    current: Option<ManifestStream>,
    pending: VecDeque<String>,
    context: Context,
}

struct Context {
    mode: Mode,
    field_types: HashMap<i64, PaperlessFieldType>,
    seen_documents: HashSet<i64>,
}

impl ManifestRecords {
    fn open(dir: Arc<File>, mode: Mode) -> Result<Self, PaperlessReadError> {
        let main = ManifestStream::new(MANIFEST.to_owned(), open_in_export(&dir, MANIFEST)?);
        let pending = split_manifests(&dir)?.into();
        Ok(Self {
            dir,
            current: Some(main),
            pending,
            context: Context {
                mode,
                field_types: HashMap::new(),
                seen_documents: HashSet::new(),
            },
        })
    }

    /// Callers stop at the first error
    fn next_item(&mut self) -> Option<Result<Item, PaperlessReadError>> {
        loop {
            if let Some(stream) = self.current.as_mut() {
                match stream.next(&mut self.context) {
                    Some(Ok(Item::Record(PaperlessRecord::Document(mut doc)))) => {
                        doc.size = open_original_file(&self.dir, &doc.original_handle)
                            .ok()
                            .map(|(_, size)| size);
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
    /// Index of the next element in the array
    index: u64,
}

impl ManifestStream {
    fn new(label: String, file: File) -> Self {
        Self {
            label,
            reader: BufReader::with_capacity(64 * 1024, file),
            position: Position::BeforeArray,
            index: 0,
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
                        Ok(Some(_)) => Some(Err(self.malformed(MalformedKind::TrailingData))),
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
                    return Some(Err(self.malformed(MalformedKind::NotAnArray)))
                }
                (_, None) => return Some(Err(self.malformed(MalformedKind::UnexpectedEnd))),
                (_, Some(_)) => return Some(Err(self.malformed(MalformedKind::InvalidJson))),
            }
        }
    }

    /// Reads one object; serde_json stops right after its closing brace
    fn read_element(&mut self, context: &mut Context) -> Result<Item, PaperlessReadError> {
        let location = self.location();
        self.index += 1;
        let mut de = serde_json::Deserializer::from_reader(&mut self.reader);
        match RecordSeed(context).deserialize(&mut de) {
            Ok(Item::DuplicateDocument) => {
                Err(malformed(MalformedKind::DuplicateDocument, location))
            }
            Ok(item) => Ok(item),
            Err(e) => Err(json_error(e, location)),
        }
    }

    fn location(&self) -> SourceLocation {
        SourceLocation {
            file: self.label.clone(),
            record: Some(self.index),
        }
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

    fn malformed(&self, kind: MalformedKind) -> PaperlessReadError {
        malformed(kind, self.location())
    }
}

pub(super) fn malformed(kind: MalformedKind, location: SourceLocation) -> PaperlessReadError {
    PaperlessReadError::Malformed { kind, location }
}

/// Keeps serde's message out: it can quote source data
pub(super) fn json_error(error: serde_json::Error, location: SourceLocation) -> PaperlessReadError {
    let kind = match error.classify() {
        Category::Io => return io_error(&location.to_string(), error.into()),
        Category::Eof => MalformedKind::UnexpectedEnd,
        Category::Syntax => MalformedKind::InvalidJson,
        Category::Data => MalformedKind::InvalidRecord,
    };
    malformed(kind, location)
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

/// What the `fields` value held, before keys after it are known
enum Fields {
    Done(Item),
    /// Document fields, read as JSON so one bad document can't stop the scan
    Document(serde_json::Value),
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
        let mut fields = None;
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
                        fields = Some(read_fields(*model, id, context, &mut map)?);
                    }
                },
                Key::ExportedFileName if kind == Some(Ok(Model::Document)) => {
                    original_handle = map.next_value()?
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        match kind {
            None => Err(de::Error::missing_field("model")),
            Some(Err(label)) => Ok(Item::Skipped(label)),
            Some(Ok(_)) => match fields.ok_or_else(|| de::Error::missing_field("fields"))? {
                Fields::Done(item) => Ok(item),
                Fields::Document(value) => {
                    let id = pk.ok_or_else(|| de::Error::missing_field("pk"))?;
                    Ok(document_item(id, value, original_handle, context.mode))
                }
            },
        }
    }
}

fn read_fields<'de, A: MapAccess<'de>>(
    model: Model,
    id: i64,
    context: &mut Context,
    map: &mut A,
) -> Result<Fields, A::Error> {
    let record = match (model, context.mode) {
        (Model::Document, _) => {
            if !context.seen_documents.insert(id) {
                map.next_value::<IgnoredAny>()?;
                return Ok(Fields::Done(Item::DuplicateDocument));
            }
            return Ok(Fields::Document(map.next_value()?));
        }
        (_, Mode::DocumentIds) => {
            map.next_value::<IgnoredAny>()?;
            return Ok(Fields::Done(Item::Omitted));
        }
        (Model::Correspondent, _) => {
            let NamedFields { name } = map.next_value()?;
            PaperlessRecord::Correspondent(PaperlessNamed { id, name })
        }
        (Model::DocumentType, _) => {
            let NamedFields { name } = map.next_value()?;
            PaperlessRecord::DocumentType(PaperlessNamed { id, name })
        }
        (Model::Tag, _) => {
            let f: TagFields = map.next_value()?;
            PaperlessRecord::Tag(PaperlessTag {
                id,
                name: f.name,
                color: f.color,
                is_inbox_tag: f.is_inbox_tag,
                parent: f.tn_parent,
            })
        }
        (Model::CustomField, _) => {
            let field = map.next_value::<CustomFieldFields>()?.into_record(id);
            context.field_types.insert(id, field.data_type);
            PaperlessRecord::CustomField(field)
        }
        (Model::CustomFieldInstance, _) => {
            let value: serde_json::Value = map.next_value()?;
            let document = owning_document(&value)?;
            let Ok(f) = InstanceFields::deserialize(value) else {
                return Ok(Fields::Done(invalid(document)));
            };
            if f.deleted_at.is_some() {
                return Ok(Fields::Done(Item::Omitted));
            }
            let Some(&data_type) = context.field_types.get(&f.field) else {
                return Ok(Fields::Done(invalid(document)));
            };
            PaperlessRecord::CustomFieldInstance(PaperlessCustomFieldInstance {
                document,
                field: f.field,
                value: f.into_value(data_type),
            })
        }
        (Model::Note, _) => {
            let value: serde_json::Value = map.next_value()?;
            let document = owning_document(&value)?;
            let Ok(f) = NoteFields::deserialize(value) else {
                return Ok(Fields::Done(invalid(document)));
            };
            PaperlessRecord::Note(PaperlessNote {
                id,
                document,
                body: f.note,
                created: f.created,
                deleted: f.deleted_at.is_some(),
            })
        }
    };
    Ok(Fields::Done(Item::Record(record)))
}

/// The document a note or field value belongs to; without it the record
/// can't be tied to one document, so the scan fails
fn owning_document<E: de::Error>(fields: &serde_json::Value) -> Result<i64, E> {
    fields
        .get("document")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| E::missing_field("document"))
}

fn invalid(document: i64) -> Item {
    Item::Record(PaperlessRecord::InvalidDocument(PaperlessInvalidDocument {
        id: document,
        reason: InvalidDocumentReason::MalformedRecord,
    }))
}

fn document_item(id: i64, fields: serde_json::Value, handle: Option<String>, mode: Mode) -> Item {
    if mode == Mode::DocumentIds {
        #[derive(Deserialize)]
        struct Liveness {
            deleted_at: Option<IgnoredAny>,
            root_document: Option<i64>,
        }
        return match Liveness::deserialize(fields) {
            Ok(Liveness {
                deleted_at: Some(_),
                ..
            })
            | Ok(Liveness {
                root_document: Some(_),
                ..
            }) => Item::Omitted,
            _ => Item::DocumentId(id),
        };
    }
    let document = DocumentFields::deserialize(fields)
        .ok()
        .zip(handle)
        .and_then(|(fields, handle)| fields.into_record(id, handle));
    match document {
        Some(document) => Item::Record(PaperlessRecord::Document(Box::new(document))),
        None => invalid(id),
    }
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
pub(super) struct CustomFieldFields {
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
    pub(super) fn into_record(self, id: i64) -> PaperlessCustomField {
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
    fn into_record(self, id: i64, original_handle: String) -> Option<PaperlessDocument> {
        let checksum = PaperlessChecksum::parse(&self.checksum)?;
        Some(PaperlessDocument {
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
            original_handle,
        })
    }
}

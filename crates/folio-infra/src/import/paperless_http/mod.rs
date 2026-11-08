//! Reads a live Paperless-ngx instance through its REST API
//!
//! Requests go out one at a time. Documents are listed in `(modified, id)`
//! order and each page starts from the last pair seen, so a document
//! modified during the listing moves behind the cursor instead of shifting
//! another one out of view. Such a document is read again there, and its
//! newer records follow the older ones in the stream.
//!
//! Every file request names its document as `?version=`: Paperless 3 would
//! otherwise serve a root's newest file version, and Paperless 2 ignores it.

mod client;
mod wire;

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::ports::{
    CompleteIdSet, InvalidDocumentReason, MalformedKind, OriginalFile, PaperlessDocument,
    PaperlessFieldType, PaperlessInvalidDocument, PaperlessNamed, PaperlessReadError,
    PaperlessReader, PaperlessRecord, PaperlessSourceInfo, PaperlessTag, PaperlessVersion,
    RecordStream, SourceLocation,
};
use futures::TryStreamExt;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

use super::paperless_fs::malformed;
pub use client::InvalidPaperlessSource;
use client::{upstream, Api};
use wire::{Content, CustomField, Document, IdPage, Key, Metadata, Page};

const PAGE_SIZE: &str = "100";
/// Paperless's own maximum; trash is read in one page so its order can't
/// shift between pages
const WHOLE_LISTING: &str = "100000";
const DOCUMENT_FIELDS: &str = "id,correspondent,document_type,title,content,tags,created,\
modified,added,deleted_at,archive_serial_number,original_file_name,mime_type,notes,\
custom_fields,root_document,versions";
const CHANNEL_DEPTH: usize = 64;

/// REST adapter for [`PaperlessReader`]; a document's `original_handle` is
/// its Paperless ID
#[derive(Debug)]
pub struct PaperlessHttpReader {
    api: Api,
}

impl PaperlessHttpReader {
    /// `base_url` is where Paperless is served, with any path prefix
    pub fn new(base_url: &str, token: &str) -> Result<Self, InvalidPaperlessSource> {
        Ok(Self {
            api: Api::new(base_url, token)?,
        })
    }

    /// Replaces the waits between retries of a 429 or 5xx response
    pub fn with_retry_delays(self, delays: Vec<Duration>) -> Self {
        Self {
            api: self.api.with_retry_delays(delays),
        }
    }

    fn listing(&self, since: Option<DateTime<Utc>>) -> RecordStream {
        let api = self.api.clone();
        let (tx, rx) = mpsc::channel(CHANNEL_DEPTH);
        tokio::spawn(async move {
            let sink = Sink(tx);
            if let Err(e) = list_records(&api, since, &sink).await {
                let _ = sink.0.send(Err(e)).await;
            }
        });
        Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        }))
    }
}

#[async_trait]
impl PaperlessReader for PaperlessHttpReader {
    async fn source_info(&self) -> Result<PaperlessSourceInfo, PaperlessReadError> {
        // `/api/` redirects to the HTML schema viewer, which refuses JSON
        let path = "api/documents/";
        let response = self
            .api
            .get(path, &[("page", "1"), ("page_size", "1"), ("fields", "id")])
            .await?;
        response
            .headers()
            .get("x-version")
            .and_then(|v| v.to_str().ok())
            .and_then(PaperlessVersion::parse)
            .map(|version| PaperlessSourceInfo { version })
            .ok_or_else(|| malformed(MalformedKind::InvalidVersion, SourceLocation::file(path)))
    }

    async fn scan(&self) -> Result<RecordStream, PaperlessReadError> {
        Ok(self.listing(None))
    }

    async fn modified_since(
        &self,
        cursor: DateTime<Utc>,
    ) -> Result<RecordStream, PaperlessReadError> {
        Ok(self.listing(Some(cursor)))
    }

    /// API version 9 lists every matching ID on the first page, from one
    /// query; the IDs must be unique and as many as Paperless counts
    async fn list_document_ids(&self) -> Result<CompleteIdSet, PaperlessReadError> {
        let ids = all_ids(&self.api, &[]).await?;
        Ok(CompleteIdSet::from_complete_listing(ids))
    }

    async fn open_original(
        &self,
        document: &PaperlessDocument,
    ) -> Result<OriginalFile, PaperlessReadError> {
        let id: i64 =
            document
                .original_handle
                .parse()
                .map_err(|_| PaperlessReadError::UnsafePath {
                    what: "original handle".to_owned(),
                })?;
        let path = format!("api/documents/{id}/download/");
        let version = id.to_string();
        let response = self
            .api
            .download(&path, &[("original", "true"), ("version", &version)])
            .await?
            .ok_or_else(|| PaperlessReadError::Missing {
                what: format!("the original of Paperless document {id}"),
            })?;
        let size = response.content_length();
        let body = response
            .bytes_stream()
            .map_err(|_| std::io::Error::other("the Paperless download was interrupted"));
        Ok(OriginalFile {
            size,
            reader: Box::new(StreamReader::new(body)),
        })
    }
}

/// The IDs of the live root documents matching `filter`
async fn all_ids(api: &Api, filter: &[(&str, &str)]) -> Result<BTreeSet<i64>, PaperlessReadError> {
    let mut query = vec![("page", "1"), ("page_size", "1"), ("fields", "id")];
    query.extend_from_slice(filter);
    let page: IdPage = api.get_json("api/documents/", &query).await?;
    let all = page.all.ok_or_else(upstream)?;
    let ids: BTreeSet<i64> = all.iter().copied().collect();
    if ids.len() != all.len() || ids.len() as u64 != page.count {
        return Err(upstream());
    }
    Ok(ids)
}

async fn metadata(api: &Api, id: i64) -> Result<Option<Metadata>, PaperlessReadError> {
    let path = format!("api/documents/{id}/metadata/");
    let version = id.to_string();
    api.get_json_opt(&path, &[("version", &version)]).await
}

/// The content of the file `version` of `root`
async fn content(api: &Api, root: i64, version: i64) -> Result<Option<String>, PaperlessReadError> {
    let path = format!("api/documents/{root}/");
    let version = version.to_string();
    let content: Option<Content> = api
        .get_json_opt(&path, &[("version", &version), ("fields", "content")])
        .await?;
    Ok(content.map(|c| c.content))
}

struct Sink(mpsc::Sender<Result<PaperlessRecord, PaperlessReadError>>);

impl Sink {
    /// Fails once the reader is gone; the listing then stops, and its error
    /// goes nowhere
    async fn send(&self, record: PaperlessRecord) -> Result<(), PaperlessReadError> {
        self.0.send(Ok(record)).await.map_err(|_| upstream())
    }

    async fn send_all(
        &self,
        records: impl IntoIterator<Item = PaperlessRecord>,
    ) -> Result<(), PaperlessReadError> {
        for record in records {
            self.send(record).await?;
        }
        Ok(())
    }
}

async fn list_records(
    api: &Api,
    since: Option<DateTime<Utc>>,
    sink: &Sink,
) -> Result<(), PaperlessReadError> {
    let taxonomy = |path| all_pages::<PaperlessNamed>(api, path);
    sink.send_all(
        taxonomy("api/correspondents/")
            .await?
            .into_iter()
            .map(PaperlessRecord::Correspondent),
    )
    .await?;
    sink.send_all(
        taxonomy("api/document_types/")
            .await?
            .into_iter()
            .map(PaperlessRecord::DocumentType),
    )
    .await?;
    sink.send_all(
        all_pages::<PaperlessTag>(api, "api/tags/")
            .await?
            .into_iter()
            .map(PaperlessRecord::Tag),
    )
    .await?;
    let mut field_types = HashMap::new();
    for field in all_pages::<CustomField>(api, "api/custom_fields/").await? {
        let field = field.fields.into_record(field.id);
        field_types.insert(field.id, field.data_type);
        sink.send(PaperlessRecord::CustomField(field)).await?;
    }
    let mut documents = Documents {
        api,
        field_types: &field_types,
        sink,
        emitted: HashMap::new(),
    };
    documents.list(since).await?;
    list_trash(api, since, &field_types, sink).await
}

/// Every page of a taxonomy listing, which is small enough to read whole
/// before any record goes out
async fn all_pages<T: DeserializeOwned>(
    api: &Api,
    path: &str,
) -> Result<Vec<T>, PaperlessReadError> {
    let mut out = Vec::new();
    for page in 1u64.. {
        let page = page.to_string();
        let query = [
            ("page", page.as_str()),
            ("page_size", PAGE_SIZE),
            ("ordering", "id"),
        ];
        let response: Page<T> = api.get_json(path, &query).await?;
        let empty = response.results.is_empty();
        out.extend(response.results);
        if response.next.is_none() || empty {
            break;
        }
    }
    Ok(out)
}

/// Where the next page of documents starts
#[derive(Clone, Copy, PartialEq, Eq)]
enum Floor {
    /// `modified >= m`; the pairs already passed are skipped
    From(DateTime<Utc>),
    /// `modified > m`, after a run of documents sharing `m` was read whole
    After(DateTime<Utc>),
}

impl Floor {
    fn at(self) -> DateTime<Utc> {
        match self {
            Floor::From(m) | Floor::After(m) => m,
        }
    }
}

fn timestamp(m: DateTime<Utc>) -> String {
    m.to_rfc3339_opts(SecondsFormat::Micros, true)
}

struct Documents<'a> {
    api: &'a Api,
    field_types: &'a HashMap<i64, PaperlessFieldType>,
    sink: &'a Sink,
    /// The `modified` of each document's last records, so a document comes
    /// out again only when it changed
    emitted: HashMap<i64, Option<DateTime<Utc>>>,
}

impl Documents<'_> {
    /// Keyset paging over `(modified, id)`. Paperless only filters on
    /// `modified`, so each request asks for `modified >= m` of the last pair
    /// seen and skips the pairs already passed. A full page that never gets
    /// past `m` is a run of documents sharing it, which page numbers would
    /// shift under; the run is read whole by ID instead.
    async fn list(&mut self, since: Option<DateTime<Utc>>) -> Result<(), PaperlessReadError> {
        let path = "api/documents/";
        let mut floor = since.map(Floor::From);
        let mut last: Option<(DateTime<Utc>, i64)> = None;
        loop {
            let bound = floor.map(|f| timestamp(f.at()));
            let mut query = vec![
                ("page", "1"),
                ("page_size", PAGE_SIZE),
                ("ordering", "modified,id"),
                ("fields", DOCUMENT_FIELDS),
            ];
            match (floor, bound.as_deref()) {
                (Some(Floor::From(_)), Some(m)) => query.push(("modified__gte", m)),
                (Some(Floor::After(_)), Some(m)) => query.push(("modified__gt", m)),
                _ => {}
            }
            let response: Page<Value> = self.api.get_json(path, &query).await?;
            let more = response.next.is_some() && !response.results.is_empty();
            for raw in response.results {
                let key = key(&raw, path)?;
                if let Some(modified) = key.modified() {
                    if last.is_some_and(|last| (modified, key.id) <= last) {
                        continue;
                    }
                    last = Some((modified, key.id));
                }
                self.emit(key, raw).await?;
            }
            if !more {
                return Ok(());
            }
            let Some((modified, id)) = last else {
                // Nothing on a full page has a readable `modified`
                return Err(upstream());
            };
            if floor.map(Floor::at) == Some(modified) {
                self.tie_run(modified, id).await?;
                floor = Some(Floor::After(modified));
            } else {
                floor = Some(Floor::From(modified));
            }
        }
    }

    /// Every document whose `modified` is exactly `m` and whose ID is above
    /// `after`, fetched by ID in chunks of one page
    async fn tie_run(&mut self, m: DateTime<Utc>, after: i64) -> Result<(), PaperlessReadError> {
        let bound = timestamp(m);
        let ids = all_ids(
            self.api,
            &[("modified__gte", &bound), ("modified__lte", &bound)],
        )
        .await?;
        let ids: Vec<i64> = ids.into_iter().filter(|id| *id > after).collect();
        let path = "api/documents/";
        for chunk in ids.chunks(100) {
            let chunk = chunk
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let query = [
                ("page", "1"),
                ("page_size", PAGE_SIZE),
                ("ordering", "id"),
                ("fields", DOCUMENT_FIELDS),
                ("id__in", chunk.as_str()),
            ];
            let response: Page<Value> = self.api.get_json(path, &query).await?;
            for raw in response.results {
                let key = key(&raw, path)?;
                self.emit(key, raw).await?;
            }
        }
        Ok(())
    }

    /// Sends a document's records unless the same or a newer state of it
    /// already went out
    async fn emit(&mut self, key: Key, raw: Value) -> Result<(), PaperlessReadError> {
        let modified = key.modified();
        if let Some(previous) = self.emitted.get(&key.id) {
            if modified <= *previous {
                return Ok(());
            }
        }
        self.emitted.insert(key.id, modified);
        let records = document_records(self.api, key.id, raw, self.field_types).await?;
        self.sink.send_all(records).await
    }
}

/// A record without a readable ID can't be tied to one document
fn key(raw: &Value, path: &str) -> Result<Key, PaperlessReadError> {
    Key::deserialize(raw)
        .map_err(|_| malformed(MalformedKind::InvalidRecord, SourceLocation::file(path)))
}

/// A document's records. A document whose file details are gone fails
/// alone, as [`PaperlessRecord::InvalidDocument`].
async fn document_records(
    api: &Api,
    id: i64,
    raw: Value,
    field_types: &HashMap<i64, PaperlessFieldType>,
) -> Result<Vec<PaperlessRecord>, PaperlessReadError> {
    let Ok(mut document) = Document::deserialize(raw) else {
        return Ok(vec![invalid(id)]);
    };
    let Some(file) = metadata(api, id).await? else {
        return Ok(vec![invalid(id)]);
    };
    if document.has_versions() {
        // A root's listed content is its newest version's
        let Some(own) = content(api, id, id).await? else {
            return Ok(vec![invalid(id)]);
        };
        document.content = own;
    }
    let Some(records) =
        document.into_records(&file.original_checksum, file.original_size, field_types)
    else {
        return Ok(vec![invalid(id)]);
    };

    let added = records.versions.iter().rev().filter(|v| !v.is_root);
    let mut versions = Vec::new();
    for (index, version) in (1..).zip(added) {
        let record = match (
            metadata(api, version.id).await?,
            content(api, id, version.id).await?,
        ) {
            (Some(file), Some(text)) => {
                wire::version_record(&records.document, version, index, text, file)
            }
            _ => None,
        };
        versions.push(match record {
            Some(record) => PaperlessRecord::Document(Box::new(record)),
            None => invalid(version.id),
        });
    }
    let mut out = vec![PaperlessRecord::Document(Box::new(records.document))];
    out.extend(versions);
    out.extend(records.rest);
    Ok(out)
}

/// Trashed documents, which `/api/documents/` leaves out. Paperless won't
/// serve `/metadata/` for them, so only Paperless 3, which lists their
/// checksum, yields them; Paperless 2 exports leave them out too. A file
/// version trashed on its own isn't yielded: the API doesn't say which
/// version it was.
async fn list_trash(
    api: &Api,
    since: Option<DateTime<Utc>>,
    field_types: &HashMap<i64, PaperlessFieldType>,
    sink: &Sink,
) -> Result<(), PaperlessReadError> {
    let path = "api/trash/";
    let response: Page<Value> = api
        .get_json(path, &[("page", "1"), ("page_size", WHOLE_LISTING)])
        .await?;
    if response.next.is_some() {
        return Err(upstream());
    }
    for raw in response.results {
        let id = key(&raw, path)?.id;
        let Ok(document) = Document::deserialize(raw) else {
            sink.send(invalid(id)).await?;
            continue;
        };
        if document.root_document.is_some() || since.is_some_and(|since| document.modified < since)
        {
            continue;
        }
        let Some(checksum) = document.listed_checksum().map(str::to_owned) else {
            continue;
        };
        let records = match document.into_records(&checksum, None, field_types) {
            Some(records) => std::iter::once(PaperlessRecord::Document(Box::new(records.document)))
                .chain(records.rest)
                .collect(),
            None => vec![invalid(id)],
        };
        sink.send_all(records).await?;
    }
    Ok(())
}

fn invalid(id: i64) -> PaperlessRecord {
    PaperlessRecord::InvalidDocument(PaperlessInvalidDocument {
        id,
        reason: InvalidDocumentReason::MalformedRecord,
    })
}

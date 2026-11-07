//! Reads a live Paperless-ngx instance through its REST API
//!
//! Requests go out one at a time. Documents are listed in `(modified, id)`
//! order and each page starts from the last pair seen, so a document
//! modified during the listing moves behind the cursor instead of shifting
//! another one out of view.

mod client;
mod wire;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::ports::{
    CompleteIdSet, InvalidDocumentReason, MalformedKind, OriginalFile, PaperlessDocument,
    PaperlessFieldType, PaperlessInvalidDocument, PaperlessReadError, PaperlessReader,
    PaperlessRecord, PaperlessSourceInfo, PaperlessVersion, RecordStream, SourceFailure,
};
use futures::TryStreamExt;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

pub use client::InvalidPaperlessUrl;
use client::{malformed, Api, Failure};
use wire::{Content, CustomField, Document, Handle, Id, Metadata, Named, Page, Tag};

const PAGE_SIZE: &str = "100";
const ID_PAGE_SIZE: &str = "1000";
const DOCUMENT_FIELDS: &str = "id,correspondent,document_type,title,content,tags,created,\
modified,added,deleted_at,archive_serial_number,original_file_name,mime_type,notes,\
custom_fields,root_document,versions";
const CHANNEL_DEPTH: usize = 64;

/// REST adapter for [`PaperlessReader`]; a document's `original_handle` is
/// its Paperless ID, plus the version to download for a Paperless 3 root
/// that has file versions
pub struct PaperlessHttpReader {
    api: Api,
}

impl fmt::Debug for PaperlessHttpReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaperlessHttpReader")
            .field("api", &self.api)
            .finish()
    }
}

impl PaperlessHttpReader {
    /// `base_url` is where Paperless is served, with any path prefix
    pub fn new(base_url: &str, token: &str) -> Result<Self, InvalidPaperlessUrl> {
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
        let response = self.api.get("api/", &[]).await?;
        response
            .headers()
            .get("x-version")
            .and_then(|v| v.to_str().ok())
            .and_then(PaperlessVersion::parse)
            .map(|version| PaperlessSourceInfo { version })
            .ok_or_else(|| malformed(MalformedKind::InvalidVersion, "api/"))
    }

    async fn scan(&self) -> Result<RecordStream, PaperlessReadError> {
        Ok(self.listing(None))
    }

    async fn list_document_ids(&self) -> Result<CompleteIdSet, PaperlessReadError> {
        let mut ids = BTreeSet::new();
        let mut expected = None;
        let mut all = None;
        for page in 1.. {
            let page = page.to_string();
            let response: Page<Id> = self
                .api
                .get_json(
                    "api/documents/",
                    &[
                        ("page", &page),
                        ("page_size", ID_PAGE_SIZE),
                        ("ordering", "id"),
                        ("fields", "id"),
                    ],
                )
                .await?;
            if expected.is_none() {
                expected = Some(response.count);
                all = response.all;
            }
            let empty = response.results.is_empty();
            for Id { id } in response.results {
                if !ids.insert(id) {
                    return Err(incomplete());
                }
            }
            if response.next.is_none() || empty {
                break;
            }
        }
        if Some(ids.len() as u64) != expected {
            return Err(incomplete());
        }
        if all.is_some_and(|all| all.into_iter().collect::<BTreeSet<_>>() != ids) {
            return Err(incomplete());
        }
        Ok(CompleteIdSet::from_complete_listing(ids))
    }

    async fn open_original(
        &self,
        document: &PaperlessDocument,
    ) -> Result<OriginalFile, PaperlessReadError> {
        let handle = Handle::decode(&document.original_handle).ok_or_else(|| {
            PaperlessReadError::UnsafePath {
                what: "original handle".to_owned(),
            }
        })?;
        let path = format!("api/documents/{}/download/", handle.id);
        let version = handle.version.map(|v| v.to_string());
        let mut query = vec![("original", "true")];
        if let Some(version) = version.as_deref() {
            query.push(("version", version));
        }
        let response = self.api.get(&path, &query).await?;
        let size = response.content_length();
        let body = response
            .bytes_stream()
            .map_err(|_| io::Error::other("the Paperless download was interrupted"));
        Ok(OriginalFile {
            size,
            reader: Box::new(StreamReader::new(body)),
        })
    }
}

/// A listing whose pages don't add up can't prove what is missing
fn incomplete() -> PaperlessReadError {
    PaperlessReadError::Source(SourceFailure::UpstreamError)
}

struct Sink(mpsc::Sender<Result<PaperlessRecord, PaperlessReadError>>);

impl Sink {
    /// Fails once the reader is gone; the listing then stops, and its error
    /// goes nowhere
    async fn send(&self, record: PaperlessRecord) -> Result<(), PaperlessReadError> {
        self.0
            .send(Ok(record))
            .await
            .map_err(|_| PaperlessReadError::Source(SourceFailure::UpstreamError))
    }
}

async fn list_records(
    api: &Api,
    since: Option<DateTime<Utc>>,
    sink: &Sink,
) -> Result<(), PaperlessReadError> {
    for named in all_pages::<Named>(api, "api/correspondents/").await? {
        sink.send(PaperlessRecord::Correspondent(named.into_named()))
            .await?;
    }
    for named in all_pages::<Named>(api, "api/document_types/").await? {
        sink.send(PaperlessRecord::DocumentType(named.into_named()))
            .await?;
    }
    for tag in all_pages::<Tag>(api, "api/tags/").await? {
        sink.send(tag.into_record()).await?;
    }
    let mut field_types = HashMap::new();
    for field in all_pages::<CustomField>(api, "api/custom_fields/").await? {
        let field = field.fields.into_record(field.id);
        field_types.insert(field.id, field.data_type);
        sink.send(PaperlessRecord::CustomField(field)).await?;
    }
    list_documents(api, since, &field_types, sink).await
}

/// Taxonomy is small, so each kind is read whole before any record goes out
async fn all_pages<T: DeserializeOwned>(api: &Api, path: &str) -> Result<Vec<T>, Failure> {
    let mut out = Vec::new();
    for page in 1.. {
        let page = page.to_string();
        let response: Page<T> = api
            .get_json(
                path,
                &[
                    ("page", &page),
                    ("page_size", PAGE_SIZE),
                    ("ordering", "id"),
                ],
            )
            .await?;
        let empty = response.results.is_empty();
        out.extend(response.results);
        if response.next.is_none() || empty {
            break;
        }
    }
    Ok(out)
}

/// Keyset paging over `(modified, id)`. Paperless only filters on
/// `modified`, so each request asks for `modified >= m` of the last pair
/// seen and skips the pairs already passed; when a whole page shares one
/// `modified`, the next request turns the page instead.
async fn list_documents(
    api: &Api,
    since: Option<DateTime<Utc>>,
    field_types: &HashMap<i64, PaperlessFieldType>,
    sink: &Sink,
) -> Result<(), PaperlessReadError> {
    let mut floor = since;
    let mut page = 1u64;
    let mut last: Option<(DateTime<Utc>, i64)> = None;
    let mut seen = HashSet::new();
    loop {
        let floor_param = floor.map(|m| m.to_rfc3339_opts(SecondsFormat::Micros, true));
        let page_param = page.to_string();
        let mut query = vec![
            ("page", page_param.as_str()),
            ("page_size", PAGE_SIZE),
            ("ordering", "modified,id"),
            ("fields", DOCUMENT_FIELDS),
        ];
        if let Some(floor) = floor_param.as_deref() {
            query.push(("modified__gte", floor));
        }
        let response: Page<Value> = api.get_json("api/documents/", &query).await?;
        let empty = response.results.is_empty();
        for raw in response.results {
            let id = raw
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| malformed(MalformedKind::InvalidRecord, "api/documents/"))?;
            if let Some(modified) = raw
                .get("modified")
                .and_then(Value::as_str)
                .and_then(|m| m.parse::<DateTime<Utc>>().ok())
            {
                if last.is_some_and(|last| (modified, id) <= last) {
                    continue;
                }
                last = Some((modified, id));
            }
            if !seen.insert(id) {
                continue;
            }
            for record in document_records(api, id, raw, field_types).await? {
                sink.send(record).await?;
            }
        }
        if response.next.is_none() || empty {
            return Ok(());
        }
        match last {
            Some((modified, _)) if floor != Some(modified) => {
                floor = Some(modified);
                page = 1;
            }
            _ => page += 1,
        }
    }
}

/// A document's records; none when it vanished while being read
async fn document_records(
    api: &Api,
    id: i64,
    raw: Value,
    field_types: &HashMap<i64, PaperlessFieldType>,
) -> Result<Vec<PaperlessRecord>, PaperlessReadError> {
    let Ok(mut document) = Document::deserialize(raw) else {
        return Ok(vec![invalid(id)]);
    };
    let own_version = document.has_versions().then(|| id.to_string());
    let own_version = own_version.as_deref().map(|v| ("version", v));
    let metadata_path = format!("api/documents/{id}/metadata/");
    let Some(metadata) = optional(
        api.get_json::<Metadata>(&metadata_path, own_version.as_slice())
            .await,
    )?
    else {
        return Ok(Vec::new());
    };
    if let Some(version) = own_version {
        // A root's listed content is its newest version's
        let Some(own) = content(api, id, version.1).await? else {
            return Ok(Vec::new());
        };
        document.content = own;
    }

    let versions = std::mem::take(&mut document.versions);
    let Some(records) = document.into_records(metadata, field_types) else {
        return Ok(vec![invalid(id)]);
    };
    let root = records.document;
    let mut out = Vec::new();
    let added = versions.iter().rev().filter(|v| !v.is_root);
    for (index, version) in (1..).zip(added) {
        let vid = version.id.to_string();
        let path = format!("api/documents/{vid}/metadata/");
        let Some(metadata) = optional(api.get_json::<Metadata>(&path, &[]).await)? else {
            continue;
        };
        let Some(text) = content(api, id, &vid).await? else {
            continue;
        };
        out.push(
            match wire::version_record(&root, version, index, text, metadata) {
                Some(record) => PaperlessRecord::Document(Box::new(record)),
                None => invalid(version.id),
            },
        );
    }
    out.insert(0, PaperlessRecord::Document(Box::new(root)));
    out.extend(records.rest);
    Ok(out)
}

/// The content of one file of `root`
async fn content(
    api: &Api,
    root: i64,
    version: &str,
) -> Result<Option<String>, PaperlessReadError> {
    let path = format!("api/documents/{root}/");
    let response = api
        .get_json::<Content>(&path, &[("version", version), ("fields", "content")])
        .await;
    Ok(optional(response)?.map(|c| c.content))
}

fn optional<T>(result: Result<T, Failure>) -> Result<Option<T>, PaperlessReadError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Failure::NotFound) => Ok(None),
        Err(Failure::Read(e)) => Err(e),
    }
}

fn invalid(id: i64) -> PaperlessRecord {
    PaperlessRecord::InvalidDocument(PaperlessInvalidDocument {
        id,
        reason: InvalidDocumentReason::MalformedRecord,
    })
}

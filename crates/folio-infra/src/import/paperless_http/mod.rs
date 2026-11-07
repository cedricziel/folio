//! Reads a live Paperless-ngx instance through its REST API
//!
//! Requests go out one at a time. Documents are listed in `(modified, id)`
//! order and each page starts from the last pair seen, so a document
//! modified during the listing moves behind the cursor instead of shifting
//! another one out of view.
//!
//! Every file request names its document as `?version=`: Paperless 3 would
//! otherwise serve a root's newest file version, and Paperless 2 ignores it.

mod client;
mod wire;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::ports::{
    digest_original, ChecksumAlgorithm, CompleteIdSet, InvalidDocumentReason, MalformedKind,
    OriginalFile, PaperlessDocument, PaperlessFieldType, PaperlessInvalidDocument, PaperlessNamed,
    PaperlessReadError, PaperlessReader, PaperlessRecord, PaperlessSourceInfo, PaperlessTag,
    PaperlessVersion, RecordStream, SourceFailure, SourceLocation,
};
use futures::TryStreamExt;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncRead;
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

use super::paperless_fs::malformed;
pub use client::InvalidPaperlessUrl;
use client::{upstream, Api};
use wire::{Content, CustomField, Document, Id, IdPage, Key, Metadata, Page};

const PAGE_SIZE: &str = "100";
const ID_PAGE_SIZE: &str = "1000";
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

    /// API version 9 lists every matching ID on the first page; without that
    /// list the pages are read one by one. Either way the IDs must be unique
    /// and as many as the count Paperless reports.
    async fn list_document_ids(&self) -> Result<CompleteIdSet, PaperlessReadError> {
        let path = "api/documents/";
        let query = [("ordering", "id"), ("fields", "id")];
        let first: IdPage = self
            .api
            .get_json(
                path,
                &[
                    ("page", "1"),
                    ("page_size", ID_PAGE_SIZE),
                    query[0],
                    query[1],
                ],
            )
            .await?;
        let (count, listed) = match first.all {
            Some(all) => (first.count, all),
            None => {
                let (count, ids) = all_pages::<Id>(&self.api, path, ID_PAGE_SIZE, &query).await?;
                (count, ids.into_iter().map(|Id { id }| id).collect())
            }
        };
        let ids: BTreeSet<i64> = listed.iter().copied().collect();
        if ids.len() != listed.len() || ids.len() as u64 != count {
            return Err(upstream());
        }
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
        let response = download(&self.api, id).await?.ok_or_else(upstream)?;
        Ok(OriginalFile {
            size: response.content_length(),
            reader: Box::new(body_reader(response)),
        })
    }
}

async fn download(api: &Api, id: i64) -> Result<Option<reqwest::Response>, PaperlessReadError> {
    let path = format!("api/documents/{id}/download/");
    let version = id.to_string();
    api.get_opt(&path, &[("original", "true"), ("version", &version)])
        .await
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

fn body_reader(response: reqwest::Response) -> impl AsyncRead + Send + Unpin {
    StreamReader::new(
        response
            .bytes_stream()
            .map_err(|_| io::Error::other("the Paperless download was interrupted")),
    )
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
    let by_id = [("ordering", "id")];
    let taxonomy = |path| all_pages::<PaperlessNamed>(api, path, PAGE_SIZE, &by_id);
    let (_, correspondents) = taxonomy("api/correspondents/").await?;
    sink.send_all(
        correspondents
            .into_iter()
            .map(PaperlessRecord::Correspondent),
    )
    .await?;
    let (_, types) = taxonomy("api/document_types/").await?;
    sink.send_all(types.into_iter().map(PaperlessRecord::DocumentType))
        .await?;
    let (_, tags) = all_pages::<PaperlessTag>(api, "api/tags/", PAGE_SIZE, &by_id).await?;
    sink.send_all(tags.into_iter().map(PaperlessRecord::Tag))
        .await?;
    let mut field_types = HashMap::new();
    let (_, fields) =
        all_pages::<CustomField>(api, "api/custom_fields/", PAGE_SIZE, &by_id).await?;
    for field in fields {
        let field = field.fields.into_record(field.id);
        field_types.insert(field.id, field.data_type);
        sink.send(PaperlessRecord::CustomField(field)).await?;
    }
    list_documents(api, since, &field_types, sink).await?;
    list_trash(api, since, &field_types, sink).await
}

/// Every page of a listing, and the count of the first page; taxonomy and
/// trash are small enough to read whole before any record goes out
async fn all_pages<T: DeserializeOwned>(
    api: &Api,
    path: &str,
    page_size: &str,
    query: &[(&str, &str)],
) -> Result<(u64, Vec<T>), PaperlessReadError> {
    let mut count = None;
    let mut out = Vec::new();
    for page in 1u64.. {
        let page = page.to_string();
        let mut paged = vec![("page", page.as_str()), ("page_size", page_size)];
        paged.extend_from_slice(query);
        let response: Page<T> = api.get_json(path, &paged).await?;
        count.get_or_insert(response.count);
        let empty = response.results.is_empty();
        out.extend(response.results);
        if response.next.is_none() || empty {
            break;
        }
    }
    Ok((count.unwrap_or_default(), out))
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
    let path = "api/documents/";
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
        let response: Page<Value> = api.get_json(path, &query).await?;
        let empty = response.results.is_empty();
        for raw in response.results {
            let key = key(&raw, path)?;
            if let Some(modified) = key.modified() {
                if last.is_some_and(|last| (modified, key.id) <= last) {
                    continue;
                }
                last = Some((modified, key.id));
            }
            if seen.insert(key.id) {
                sink.send_all(document_records(api, key.id, raw, field_types).await?)
                    .await?;
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

/// A record without a readable ID can't be tied to one document
fn key(raw: &Value, path: &str) -> Result<Key, PaperlessReadError> {
    Key::deserialize(raw)
        .map_err(|_| malformed(MalformedKind::InvalidRecord, SourceLocation::file(path)))
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
    let Some(file) = metadata(api, id).await? else {
        return Ok(Vec::new());
    };
    if document.has_versions() {
        // A root's listed content is its newest version's
        let Some(own) = content(api, id, id).await? else {
            return Ok(Vec::new());
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
        let (Some(file), Some(text)) = (
            metadata(api, version.id).await?,
            content(api, id, version.id).await?,
        ) else {
            continue;
        };
        versions.push(
            match wire::version_record(&records.document, version, index, text, file) {
                Some(record) => PaperlessRecord::Document(Box::new(record)),
                None => invalid(version.id),
            },
        );
    }
    let mut out = vec![PaperlessRecord::Document(Box::new(records.document))];
    out.extend(versions);
    out.extend(records.rest);
    Ok(out)
}

/// Trashed documents, which `/api/documents/` leaves out
async fn list_trash(
    api: &Api,
    since: Option<DateTime<Utc>>,
    field_types: &HashMap<i64, PaperlessFieldType>,
    sink: &Sink,
) -> Result<(), PaperlessReadError> {
    let path = "api/trash/";
    let (_, trashed) = all_pages::<Value>(api, path, PAGE_SIZE, &[]).await?;
    for raw in trashed {
        let id = key(&raw, path)?.id;
        let Ok(document) = Document::deserialize(raw) else {
            sink.send(invalid(id)).await?;
            continue;
        };
        if since.is_some_and(|since| document.modified < since) {
            continue;
        }
        sink.send_all(trashed_records(api, document, field_types).await?)
            .await?;
    }
    Ok(())
}

/// Paperless won't serve `/metadata/` for a trashed document. Paperless 3
/// lists its checksum; on Paperless 2 the original is hashed for it.
async fn trashed_records(
    api: &Api,
    document: Document,
    field_types: &HashMap<i64, PaperlessFieldType>,
) -> Result<Vec<PaperlessRecord>, PaperlessReadError> {
    let id = document.id;
    let (checksum, size) = match document.listed_checksum() {
        Some(listed) => (listed.to_owned(), None),
        None => {
            let Some(response) = download(api, id).await? else {
                return Ok(Vec::new());
            };
            // Paperless 2 checksums are MD5
            let digests = digest_original(body_reader(response), ChecksumAlgorithm::Md5)
                .await
                .map_err(|_| PaperlessReadError::Source(SourceFailure::Unreachable))?;
            (digests.md5.unwrap_or_default(), Some(digests.size))
        }
    };
    Ok(match document.into_records(&checksum, size, field_types) {
        Some(records) => std::iter::once(PaperlessRecord::Document(Box::new(records.document)))
            .chain(records.rest)
            .collect(),
        None => vec![invalid(id)],
    })
}

fn invalid(id: i64) -> PaperlessRecord {
    PaperlessRecord::InvalidDocument(PaperlessInvalidDocument {
        id,
        reason: InvalidDocumentReason::MalformedRecord,
    })
}

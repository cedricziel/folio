use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use folio_core::ports::{
    digest_original, PaperlessReadError, PaperlessReader, PaperlessRecord, PaperlessVersion,
    SourceFailure,
};
use folio_infra::import::{
    resolve_import_dir, InvalidPaperlessSource, PaperlessFsReader, PaperlessHttpReader,
};
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TOKEN: &str = "0123456789abcdef-SECRET-TOKEN";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paperless")
}

/// One recorded response from `generate_rest.py`
#[derive(Deserialize)]
struct Recorded {
    path: String,
    query: BTreeMap<String, String>,
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: Option<serde_json::Value>,
    body_file: Option<String>,
}

/// Matches the query exactly, except `fields`, which only trims the response
struct QueryIs(BTreeMap<String, String>);

impl QueryIs {
    fn new(mut query: BTreeMap<String, String>) -> Self {
        query.remove("fields");
        Self(query)
    }
}

impl wiremock::Match for QueryIs {
    fn matches(&self, request: &Request) -> bool {
        let actual: BTreeMap<String, String> = request
            .url
            .query_pairs()
            .filter(|(k, _)| k != "fields")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        actual == self.0
    }
}

/// Serves a recording; every response needs the token and API version 9
async fn serve(name: &str) -> MockServer {
    let server = MockServer::start().await;
    let raw = std::fs::read(fixtures().join(format!("rest_{name}.json"))).unwrap();
    let recording: Vec<Recorded> = serde_json::from_slice(&raw).unwrap();
    for entry in recording {
        let mut response = ResponseTemplate::new(entry.status);
        for (k, v) in &entry.headers {
            response = response.insert_header(k.as_str(), v.as_str());
        }
        response = match (entry.body, entry.body_file) {
            (_, Some(file)) => response.set_body_raw(
                std::fs::read(fixtures().join(file)).unwrap(),
                "application/pdf",
            ),
            (Some(body), None) => response.set_body_json(body),
            (None, None) => response,
        };
        Mock::given(method("GET"))
            .and(path(entry.path.as_str()))
            .and(QueryIs::new(entry.query))
            .and(header("authorization", format!("Token {TOKEN}").as_str()))
            .and(header("accept", "application/json; version=9"))
            .respond_with(response)
            .mount(&server)
            .await;
    }
    server
}

fn rest_reader(server: &MockServer) -> PaperlessHttpReader {
    PaperlessHttpReader::new(&server.uri(), TOKEN).unwrap()
}

async fn rest_records(reader: &PaperlessHttpReader) -> Vec<PaperlessRecord> {
    reader.scan().await.unwrap().try_collect().await.unwrap()
}

async fn export_records(name: &str) -> Vec<PaperlessRecord> {
    let dir = fixtures().join(name);
    let reader =
        PaperlessFsReader::new(resolve_import_dir(&dir, std::slice::from_ref(&dir)).unwrap());
    reader.scan().await.unwrap().try_collect().await.unwrap()
}

/// Sorted JSON lines, without the adapter-specific handle
fn canonical(records: impl IntoIterator<Item = PaperlessRecord>) -> Vec<String> {
    let mut out: Vec<String> = records
        .into_iter()
        .map(|mut r| {
            if let PaperlessRecord::Document(d) = &mut r {
                d.original_handle.clear();
            }
            serde_json::to_string(&r).unwrap()
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn v2_records_equal_the_export_of_the_same_library() {
    let server = serve("v2_20").await;
    let rest = canonical(rest_records(&rest_reader(&server)).await);
    assert_eq!(rest, canonical(export_records("v2_20").await));
}

#[tokio::test]
async fn v3_records_equal_the_export_of_the_same_library() {
    let server = serve("v3_4").await;
    let rest = canonical(rest_records(&rest_reader(&server)).await);
    // The API never shows deleted notes, and the size of a trashed document
    // only by downloading it
    let export = export_records("v3_4")
        .await
        .into_iter()
        .filter(|r| !matches!(r, PaperlessRecord::Note(n) if n.deleted))
        .map(|mut r| {
            if let PaperlessRecord::Document(d) = &mut r {
                if d.trashed {
                    d.size = None;
                }
            }
            r
        });
    assert_eq!(rest, canonical(export));
}

#[tokio::test]
async fn reads_the_version_from_the_api_root() {
    for (name, version) in [("v2_20", "2.20.15"), ("v3_4", "3.4.2")] {
        let server = serve(name).await;
        let info = rest_reader(&server).source_info().await.unwrap();
        assert_eq!(info.version, PaperlessVersion::parse(version).unwrap());
    }
}

#[tokio::test]
async fn a_path_prefix_is_kept() {
    let server = MockServer::start().await;
    Mock::given(path("/paperless/api/documents/"))
        .respond_with(ResponseTemplate::new(200).insert_header("X-Version", "2.20.15"))
        .mount(&server)
        .await;
    let reader = PaperlessHttpReader::new(&format!("{}/paperless", server.uri()), TOKEN).unwrap();
    assert!(reader.source_info().await.is_ok());
}

fn source_failure<T: std::fmt::Debug>(result: Result<T, PaperlessReadError>) -> SourceFailure {
    match result {
        Err(PaperlessReadError::Source(failure)) => failure,
        other => panic!("expected a source failure, got {other:?}"),
    }
}

#[tokio::test]
async fn a_cross_origin_redirect_is_refused_without_sending_the_token() {
    let elsewhere = MockServer::start().await;
    let server = MockServer::start().await;
    Mock::given(path("/api/documents/"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/api/", elsewhere.uri())),
        )
        .mount(&server)
        .await;

    let failure = source_failure(rest_reader(&server).source_info().await);
    assert_eq!(failure, SourceFailure::Redirected);
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_same_origin_redirect_is_followed_with_the_token() {
    let server = MockServer::start().await;
    Mock::given(path("/api/documents/"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "/new/api/"))
        .mount(&server)
        .await;
    Mock::given(path("/new/api/"))
        .and(header("authorization", format!("Token {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).insert_header("X-Version", "3.4.2"))
        .mount(&server)
        .await;
    assert!(rest_reader(&server).source_info().await.is_ok());
}

#[tokio::test]
async fn failures_map_to_fixed_reasons() {
    for (status, expected) in [
        (401, SourceFailure::Unauthorized),
        (403, SourceFailure::Unauthorized),
        (406, SourceFailure::UnsupportedVersion),
        // Not a Paperless at this URL
        (404, SourceFailure::Unreachable),
        (500, SourceFailure::UpstreamError),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/documents/"))
            .respond_with(ResponseTemplate::new(status).set_body_string("upstream detail"))
            .mount(&server)
            .await;
        let reader = rest_reader(&server).with_retry_delays(Vec::new());
        assert_eq!(
            source_failure(reader.source_info().await),
            expected,
            "{status}"
        );
    }

    // Nothing listens on port 9 on a test machine
    let reader = PaperlessHttpReader::new("http://127.0.0.1:9", TOKEN).unwrap();
    assert_eq!(
        source_failure(reader.source_info().await),
        SourceFailure::Unreachable
    );
}

#[tokio::test]
async fn only_http_urls_are_accepted() {
    for url in [
        "ftp://paperless.test",
        "file:///etc",
        "not a url",
        "https://user:pw@host",
    ] {
        assert_eq!(
            PaperlessHttpReader::new(url, TOKEN).err(),
            Some(InvalidPaperlessSource::Url),
            "{url}"
        );
    }
    assert_eq!(
        PaperlessHttpReader::new("https://paperless.test", "bad\ntoken").err(),
        Some(InvalidPaperlessSource::Token)
    );
}

#[tokio::test]
async fn originals_download_and_match_their_checksums() {
    for name in ["v2_20", "v3_4"] {
        let server = serve(name).await;
        let reader = rest_reader(&server);
        for record in rest_records(&reader).await {
            let PaperlessRecord::Document(doc) = record else {
                continue;
            };
            let original = reader.open_original(&doc).await.unwrap();
            let size = original.size;
            let digests = digest_original(original.reader, doc.checksum.algorithm)
                .await
                .unwrap();
            assert!(digests.matches(&doc.checksum), "{name}: doc {}", doc.id);
            assert_eq!(size, Some(digests.size));
            if !doc.trashed {
                assert_eq!(doc.size, Some(digests.size));
            }
        }
    }
}

#[tokio::test]
async fn lists_live_root_document_ids() {
    let server = serve("v3_4").await;
    let ids = rest_reader(&server).list_document_ids().await.unwrap();
    // 3 is trashed and 5 is a file version of 4
    assert_eq!(ids.ids().iter().copied().collect::<Vec<_>>(), [1, 2, 4]);
}

fn id_page(count: u64, ids: &[i64], next: bool, all: Option<&[i64]>) -> ResponseTemplate {
    let results: Vec<_> = ids.iter().map(|id| json!({ "id": id })).collect();
    let mut body = json!({
        "count": count,
        "next": next.then_some("http://paperless.test/api/documents/?page=2"),
        "previous": null,
        "results": results,
    });
    if let Some(all) = all {
        body["all"] = json!(all);
    }
    ResponseTemplate::new(200).set_body_json(body)
}

async fn id_listing(page: ResponseTemplate) -> Result<Vec<i64>, PaperlessReadError> {
    let server = MockServer::start().await;
    Mock::given(path("/api/documents/"))
        .and(QueryIs::new(
            [("page", "1"), ("page_size", "1")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        ))
        .respond_with(page)
        .mount(&server)
        .await;
    let reader = rest_reader(&server).with_retry_delays(Vec::new());
    let ids = reader.list_document_ids().await?;
    Ok(ids.ids().iter().copied().collect())
}

#[tokio::test]
async fn an_id_listing_is_complete_or_an_error() {
    // API version 9 lists every ID on the first page
    let listed = id_listing(id_page(3, &[1], true, Some(&[1, 2, 7]))).await;
    assert_eq!(listed.unwrap(), [1, 2, 7]);

    let short = id_listing(id_page(3, &[1], true, Some(&[1, 2]))).await;
    assert_eq!(source_failure(short), SourceFailure::UpstreamError);
    let repeated = id_listing(id_page(3, &[1], true, Some(&[1, 2, 2]))).await;
    assert_eq!(source_failure(repeated), SourceFailure::UpstreamError);
    let unlisted = id_listing(id_page(3, &[1], true, None)).await;
    assert_eq!(source_failure(unlisted), SourceFailure::UpstreamError);
    let failed = id_listing(ResponseTemplate::new(500)).await;
    assert_eq!(source_failure(failed), SourceFailure::UpstreamError);
}

fn modified(minute: u32) -> String {
    format!("2024-05-01T10:{minute:02}:00.000000Z")
}

fn doc_json(id: i64, minute: u32) -> serde_json::Value {
    json!({
        "id": id,
        "correspondent": null,
        "document_type": null,
        "title": format!("doc {id}"),
        "content": "",
        "tags": [],
        "created": "2024-05-01",
        "modified": modified(minute),
        "added": "2024-05-01T09:00:00+01:00",
        "deleted_at": null,
        "archive_serial_number": null,
        "original_file_name": format!("{id}.pdf"),
        "mime_type": "application/pdf",
        "notes": [],
        "custom_fields": [],
    })
}

fn doc_page(docs: &[serde_json::Value], next: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "count": docs.len(),
        "next": next.then_some("http://paperless.test/api/documents/?page=2"),
        "previous": null,
        "results": docs,
    }))
}

fn empty_page() -> ResponseTemplate {
    doc_page(&[], false)
}

/// A library with no taxonomy or trash, whose documents all have metadata
async fn document_server() -> MockServer {
    let server = MockServer::start().await;
    for taxonomy in ["correspondents", "document_types", "tags", "custom_fields"] {
        Mock::given(path(format!("/api/{taxonomy}/")))
            .respond_with(empty_page())
            .mount(&server)
            .await;
    }
    Mock::given(path("/api/trash/"))
        .respond_with(empty_page())
        .mount(&server)
        .await;
    Mock::given(path_regex(r"^/api/documents/\d+/metadata/$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "original_checksum": "0".repeat(32),
            "original_size": 10,
        })))
        .mount(&server)
        .await;
    server
}

async fn listing_page(server: &MockServer, query: &[(&str, String)], page: ResponseTemplate) {
    let mut expected: BTreeMap<String, String> = [
        ("page_size", "100".to_owned()),
        ("ordering", "modified,id".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    expected.extend(query.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
    Mock::given(path("/api/documents/"))
        .and(QueryIs::new(expected))
        .respond_with(page)
        .mount(server)
        .await;
}

fn document_ids(records: &[PaperlessRecord]) -> Vec<i64> {
    records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Document(d) => Some(d.id),
            _ => None,
        })
        .collect()
}

fn documents(records: &[PaperlessRecord]) -> Vec<(i64, String)> {
    records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Document(d) => Some((d.id, d.modified.to_rfc3339())),
            _ => None,
        })
        .collect()
}

fn at(minute: u32) -> String {
    modified(minute)
        .parse::<DateTime<Utc>>()
        .unwrap()
        .to_rfc3339()
}

#[tokio::test]
async fn a_document_modified_mid_listing_is_read_again() {
    let server = document_server().await;
    let first = ("page", "1".to_owned());
    let floor = |minute| ("modified__gte", modified(minute));
    // Document 1 is modified (to minute 5) after the first page is read;
    // page 2 by number would then start at document 4 and skip 3
    listing_page(
        &server,
        std::slice::from_ref(&first),
        doc_page(&[doc_json(1, 1), doc_json(2, 2)], true),
    )
    .await;
    listing_page(
        &server,
        &[first.clone(), floor(2)],
        doc_page(&[doc_json(2, 2), doc_json(3, 3)], true),
    )
    .await;
    listing_page(
        &server,
        &[first.clone(), floor(3)],
        doc_page(&[doc_json(3, 3), doc_json(4, 4)], true),
    )
    .await;
    listing_page(
        &server,
        &[first, floor(4)],
        doc_page(&[doc_json(4, 4), doc_json(1, 5)], false),
    )
    .await;

    let records = rest_records(&rest_reader(&server)).await;
    // The later record of document 1 replaces the earlier one
    assert_eq!(
        documents(&records),
        [(1, at(1)), (2, at(2)), (3, at(3)), (4, at(4)), (1, at(5))]
    );
}

#[tokio::test]
async fn a_run_sharing_one_modified_time_is_read_by_id() {
    let server = document_server().await;
    let first = ("page", "1".to_owned());
    // Bulk edits stamp many documents with one `modified`; this run fills
    // more than a page
    let run = doc_page(&[doc_json(1, 1), doc_json(2, 1)], true);
    listing_page(&server, std::slice::from_ref(&first), run.clone()).await;
    listing_page(
        &server,
        &[first.clone(), ("modified__gte", modified(1))],
        run,
    )
    .await;
    let exact: BTreeMap<String, String> = [
        ("page", "1"),
        ("page_size", "1"),
        ("modified__gte", &modified(1)),
        ("modified__lte", &modified(1)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    Mock::given(path("/api/documents/"))
        .and(QueryIs::new(exact))
        .respond_with(id_page(4, &[1], true, Some(&[1, 2, 3, 4])))
        .mount(&server)
        .await;
    // Document 3 changes (to minute 2) while the run is read
    let by_id: BTreeMap<String, String> = [
        ("page", "1"),
        ("page_size", "100"),
        ("ordering", "id"),
        ("id__in", "3,4"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    Mock::given(path("/api/documents/"))
        .and(QueryIs::new(by_id))
        .respond_with(doc_page(&[doc_json(3, 2), doc_json(4, 1)], false))
        .mount(&server)
        .await;
    let after: BTreeMap<String, String> = [
        ("page", "1"),
        ("page_size", "100"),
        ("ordering", "modified,id"),
        ("modified__gt", &modified(1)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    Mock::given(path("/api/documents/"))
        .and(QueryIs::new(after))
        .respond_with(doc_page(&[doc_json(3, 2)], false))
        .mount(&server)
        .await;

    let records = rest_records(&rest_reader(&server)).await;
    assert_eq!(
        documents(&records),
        [(1, at(1)), (2, at(1)), (3, at(2)), (4, at(1))]
    );
}

#[tokio::test]
async fn a_document_without_file_details_fails_alone() {
    let server = document_server().await;
    Mock::given(path("/api/documents/2/metadata/"))
        .respond_with(ResponseTemplate::new(404))
        .with_priority(1)
        .mount(&server)
        .await;
    listing_page(
        &server,
        &[("page", "1".to_owned())],
        doc_page(&[doc_json(1, 1), doc_json(2, 2)], false),
    )
    .await;
    let records = rest_records(&rest_reader(&server)).await;
    assert_eq!(document_ids(&records), [1]);
    assert!(records.iter().any(|r| matches!(
        r,
        PaperlessRecord::InvalidDocument(d) if d.id == 2
    )));
}

#[tokio::test]
async fn a_missing_original_is_missing() {
    let server = serve("v2_20").await;
    let reader = rest_reader(&server);
    let mut doc = rest_records(&reader)
        .await
        .into_iter()
        .find_map(|r| match r {
            PaperlessRecord::Document(d) => Some(d),
            _ => None,
        })
        .unwrap();
    doc.original_handle = "404".into();
    assert!(matches!(
        reader.open_original(&doc).await.err(),
        Some(PaperlessReadError::Missing { .. })
    ));
}

#[tokio::test]
async fn an_oversized_response_is_refused() {
    let server = document_server().await;
    let mut body = vec![b' '; 64 * 1024 * 1024];
    body.extend_from_slice(br#"{"count": 0, "next": null, "results": []}"#);
    Mock::given(path("/api/correspondents/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .with_priority(1)
        .mount(&server)
        .await;
    let results: Vec<_> = rest_reader(&server).scan().await.unwrap().collect().await;
    assert!(
        matches!(
            results.as_slice(),
            [Err(PaperlessReadError::Source(
                SourceFailure::UpstreamError
            ))]
        ),
        "{results:?}"
    );
}

#[tokio::test]
async fn modified_since_asks_for_documents_from_the_cursor() {
    let server = document_server().await;
    listing_page(
        &server,
        &[("page", "1".to_owned()), ("modified__gte", modified(3))],
        doc_page(&[doc_json(3, 3)], false),
    )
    .await;
    let cursor = modified(3).parse::<DateTime<Utc>>().unwrap();
    let records: Vec<_> = rest_reader(&server)
        .modified_since(cursor)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(document_ids(&records), [3]);
}

#[tokio::test]
async fn trashed_documents_come_from_the_trash_listing() {
    let server = serve("v3_4").await;
    let records = rest_records(&rest_reader(&server)).await;
    let trashed: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Document(d) if d.trashed => Some(d.id),
            _ => None,
        })
        .collect();
    assert_eq!(trashed, [3]);

    // Older than the cursor: left out of an incremental listing
    Mock::given(path("/api/documents/"))
        .and(query_param("modified__gte", "2030-01-01T00:00:00.000000Z"))
        .respond_with(empty_page())
        .mount(&server)
        .await;
    let cursor = "2030-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let recent: Vec<_> = rest_reader(&server)
        .modified_since(cursor)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(document_ids(&recent).is_empty());
}

fn retrying_reader(server: &MockServer) -> PaperlessHttpReader {
    rest_reader(server).with_retry_delays(vec![Duration::from_millis(10); 4])
}

#[tokio::test]
async fn a_429_then_a_200_succeeds() {
    let server = MockServer::start().await;
    Mock::given(path("/api/documents/"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/documents/"))
        .respond_with(ResponseTemplate::new(200).insert_header("X-Version", "2.20.15"))
        .mount(&server)
        .await;
    assert!(retrying_reader(&server).source_info().await.is_ok());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn retries_run_out_with_an_upstream_error() {
    let server = MockServer::start().await;
    Mock::given(path("/api/documents/"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let failure = source_failure(retrying_reader(&server).source_info().await);
    assert_eq!(failure, SourceFailure::UpstreamError);
    // The first try and four retries
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

fn trashed_json(id: i64, root: Option<i64>, checksum: Option<&str>) -> serde_json::Value {
    let mut doc = doc_json(id, 1);
    doc["deleted_at"] = json!("2025-06-01T09:00:00+01:00");
    doc["root_document"] = json!(root);
    if let Some(checksum) = checksum {
        doc["versions"] = json!([{
            "id": id,
            "added": "2024-05-01T09:00:00+01:00",
            "version_label": null,
            "checksum": checksum,
            "is_root": true,
        }]);
    }
    doc
}

#[tokio::test]
async fn only_trashed_roots_with_a_listed_checksum_are_yielded() {
    let server = document_server().await;
    listing_page(&server, &[("page", "1".to_owned())], empty_page()).await;
    let sha = "a".repeat(64);
    Mock::given(path("/api/trash/"))
        .respond_with(doc_page(
            &[
                // A Paperless 3 root, a version trashed on its own, and a
                // Paperless 2 document without a listed checksum
                trashed_json(7, None, Some(&sha)),
                trashed_json(8, Some(7), Some(&sha)),
                trashed_json(9, None, None),
            ],
            false,
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    let records = rest_records(&rest_reader(&server)).await;
    let trashed: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Document(d) => Some((d.id, d.trashed, d.checksum.value.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(trashed, [(7, true, sha)]);
    // Nothing was downloaded to learn a checksum
    let downloads = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/download/"))
        .count();
    assert_eq!(downloads, 0);
}

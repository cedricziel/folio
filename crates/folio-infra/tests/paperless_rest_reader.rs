use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use folio_core::ports::{
    PaperlessReadError, PaperlessReader, PaperlessRecord, PaperlessVersion, SourceFailure,
};
use folio_infra::import::{resolve_import_dir, PaperlessFsReader, PaperlessHttpReader};
use futures::TryStreamExt;
use serde::Deserialize;
use wiremock::matchers::{header, method, path};
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

impl wiremock::Match for QueryIs {
    fn matches(&self, request: &Request) -> bool {
        let actual: BTreeMap<String, String> = request
            .url
            .query_pairs()
            .filter(|(k, _)| k != "fields")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let expected: BTreeMap<_, _> = self
            .0
            .iter()
            .filter(|(k, _)| *k != "fields")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        actual == expected
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
            .and(QueryIs(entry.query))
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
    // The API never shows deleted notes, and trashed documents come from the
    // trash listing
    let export = export_records("v3_4")
        .await
        .into_iter()
        .filter(|r| match r {
            PaperlessRecord::Note(n) => !n.deleted,
            PaperlessRecord::Document(d) => !d.trashed,
            _ => true,
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
    Mock::given(path("/paperless/api/"))
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
    Mock::given(path("/api/"))
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
    Mock::given(path("/api/"))
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
        (404, SourceFailure::UpstreamError),
        (500, SourceFailure::UpstreamError),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/"))
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
        assert!(PaperlessHttpReader::new(url, TOKEN).is_err(), "{url}");
    }
}

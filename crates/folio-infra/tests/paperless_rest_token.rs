//! Its own test binary, so the global subscriber sees every log line

use std::sync::{Arc, Mutex};
use std::time::Duration;

use folio_core::ports::PaperlessReader;
use folio_infra::import::PaperlessHttpReader;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "0123456789abcdef-SECRET-TOKEN";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn responding(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/api/"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn the_token_never_reaches_logs_debug_output_or_errors() {
    let logs = Captured::default();
    let writer = logs.clone();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .init();

    let mut seen = String::new();
    for response in [
        ResponseTemplate::new(503),
        ResponseTemplate::new(401),
        ResponseTemplate::new(302).insert_header("Location", "http://127.0.0.1:9/"),
        ResponseTemplate::new(200).set_body_string("no version header"),
    ] {
        let server = responding(response).await;
        let reader = PaperlessHttpReader::new(&server.uri(), TOKEN)
            .unwrap()
            .with_retry_delays(vec![Duration::from_millis(1)]);
        let error = reader.source_info().await.unwrap_err();
        seen.push_str(&format!("{reader:?} {error:?} {error}\n"));
    }

    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("Paperless asked to retry"), "{logs}");
    assert!(!logs.contains(TOKEN), "{logs}");
    assert!(!seen.contains(TOKEN), "{seen}");
}

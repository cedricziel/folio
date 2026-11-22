#![cfg(unix)]

use std::net::TcpListener;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use axum::{body::Bytes, http::StatusCode, routing::post, Router};
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};

#[tokio::test]
async fn sigterm_exits_cleanly_with_an_open_event_stream() {
    let collector = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let logs_endpoint = format!("http://{}/v1/logs", collector.local_addr().unwrap());
    let flushed = Arc::new(Notify::new());
    let receive = flushed.clone();
    let route = Router::new().route(
        "/v1/logs",
        post(move |body: Bytes| async move {
            let marker = b"Folio shutdown complete";
            if body.windows(marker.len()).any(|window| window == marker) {
                receive.notify_one();
            }
            StatusCode::OK
        }),
    );
    tokio::spawn(async move { axum::serve(collector, route).await.unwrap() });

    let state = tempfile::tempdir().unwrap();
    std::fs::write(state.path().join(".env"), "").unwrap();
    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new(env!("CARGO_BIN_EXE_folio-app"))
        .current_dir(state.path())
        .env_clear()
        .env("FOLIO_SERVER__HOST", "127.0.0.1")
        .env("FOLIO_SERVER__PORT", port.to_string())
        .env("FOLIO_AUTH__REQUIRED", "false")
        .env("FOLIO_DATABASE__URL", "sqlite:archive.db?mode=rwc")
        .env("FOLIO_STORAGE__PATH", "storage")
        .env("FOLIO_OCR__PROVIDER", "none")
        .env("FOLIO_EMBEDDINGS__PROVIDER", "none")
        .env("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", logs_endpoint)
        .env("OTEL_BLRP_SCHEDULE_DELAY", "60000")
        .env("OTEL_EXPORTER_OTLP_TIMEOUT", "2000")
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://127.0.0.1:{port}/events");
    let mut stream = timeout(Duration::from_secs(20), async {
        loop {
            assert!(
                server.try_wait().unwrap().is_none(),
                "Folio exited before serving /events"
            );
            if let Ok(response) = client.get(&url).send().await {
                return response;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Folio did not start serving /events");
    assert_eq!(stream.status(), reqwest::StatusCode::OK);
    assert_eq!(stream.headers()["content-type"], "text/event-stream");

    assert!(Command::new("/bin/kill")
        .args(["-TERM", &server.id().unwrap().to_string()])
        .status()
        .await
        .unwrap()
        .success());

    let exit = timeout(Duration::from_secs(5), server.wait())
        .await
        .expect("Folio did not exit with the SSE connection held open")
        .unwrap();
    assert!(exit.success(), "Folio exited with {exit}");
    timeout(Duration::from_secs(1), flushed.notified())
        .await
        .expect("Folio did not flush its shutdown log to the local collector");
    assert!(
        timeout(Duration::from_secs(1), stream.chunk())
            .await
            .unwrap()
            .unwrap()
            .is_none(),
        "SSE did not end cleanly"
    );
}

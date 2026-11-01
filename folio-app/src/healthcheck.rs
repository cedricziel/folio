//! `folio-app healthcheck`: probe the local liveness endpoint, for Docker HEALTHCHECK

use std::time::Duration;

use anyhow::{bail, Context, Result};

const TIMEOUT: Duration = Duration::from_secs(3);

/// GET `/healthz` on loopback at `port`; succeeds only on a 2xx response.
pub async fn run(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/healthz");
    let client = reqwest::Client::builder().timeout(TIMEOUT).build()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url} failed"))?;
    if !response.status().is_success() {
        bail!("GET {url} returned {}", response.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Router};
    use tokio::net::TcpListener;

    async fn serve(status: StatusCode) -> u16 {
        let app = Router::new().route("/healthz", get(move || async move { status }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        port
    }

    #[tokio::test]
    async fn succeeds_on_200() {
        assert!(run(serve(StatusCode::OK).await).await.is_ok());
    }

    #[tokio::test]
    async fn fails_on_503() {
        assert!(run(serve(StatusCode::SERVICE_UNAVAILABLE).await)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn fails_when_nothing_listens() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(run(port).await.is_err());
    }
}

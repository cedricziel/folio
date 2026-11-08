//! HTTP plumbing for the Paperless REST adapter: auth, API version, redirect
//! policy and the mapping of every failure to a fixed [`SourceFailure`]

use std::error::Error as _;
use std::fmt;
use std::time::Duration;

use folio_core::ports::{PaperlessReadError, SourceFailure, SourceLocation};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use reqwest::redirect::{Attempt, Policy};
use reqwest::{Response, StatusCode, Url};
use serde::de::DeserializeOwned;

use crate::import::paperless_fs::json_error;

/// Paperless 2.16 through 3.x accept version 9
const ACCEPT_VERSION_9: &str = "application/json; version=9";
const USER_AGENT: &str = concat!("folio/", env!("CARGO_PKG_VERSION"), " paperless-import");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;
/// Whole-request limit for API calls; downloads stream and only time out
/// when they stall
const JSON_TIMEOUT: Duration = Duration::from_secs(120);
/// The largest JSON body read, far above a page of 100 documents
const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;

/// Waits before each retry of a 429 or 5xx response; once they run out the
/// request fails with [`SourceFailure::UpstreamError`]
const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// The URL or token can't be used to reach Paperless
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPaperlessSource {
    #[error("the Paperless URL must be an http or https URL")]
    Url,
    #[error("the Paperless token contains characters an HTTP header can't carry")]
    Token,
}

/// One Paperless instance, reached with one token
#[derive(Clone)]
pub(super) struct Api {
    http: reqwest::Client,
    base: Url,
    retry_delays: Vec<Duration>,
}

impl fmt::Debug for Api {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("redirect to another origin")]
struct CrossOrigin;

pub(super) fn upstream() -> PaperlessReadError {
    PaperlessReadError::Source(SourceFailure::UpstreamError)
}

impl Api {
    pub(super) fn new(base_url: &str, token: &str) -> Result<Self, InvalidPaperlessSource> {
        let mut base = Url::parse(base_url).map_err(|_| InvalidPaperlessSource::Url)?;
        if !matches!(base.scheme(), "http" | "https") || base.cannot_be_a_base() {
            return Err(InvalidPaperlessSource::Url);
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(InvalidPaperlessSource::Url);
        }
        base.set_query(None);
        base.set_fragment(None);
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }

        let mut auth = HeaderValue::from_str(&format!("Token {token}"))
            .map_err(|_| InvalidPaperlessSource::Token)?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        headers.insert(ACCEPT, HeaderValue::from_static(ACCEPT_VERSION_9));

        let origin = base.origin();
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .redirect(Policy::custom(move |attempt: Attempt| {
                if attempt.url().origin() != origin {
                    attempt.error(CrossOrigin)
                } else if attempt.previous().len() > MAX_REDIRECTS {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .expect("static client configuration is valid");
        Ok(Self {
            http,
            base,
            retry_delays: RETRY_DELAYS.to_vec(),
        })
    }

    pub(super) fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
    }

    /// GETs `path` (relative to the base URL), retrying 429 and 5xx; `None`
    /// on 404. Without a timeout, the response may be streamed for as long
    /// as it keeps arriving.
    async fn send(
        &self,
        path: &str,
        query: &[(&str, &str)],
        timeout: Option<Duration>,
    ) -> Result<Option<Response>, PaperlessReadError> {
        let url = self.base.join(path).map_err(|_| upstream())?;
        let mut delays = self.retry_delays.iter();
        loop {
            let mut request = self.http.get(url.clone()).query(query);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let response = request.send().await.map_err(transport_failure)?;
            let status = response.status();
            if status.is_success() {
                return Ok(Some(response));
            }
            if status == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if let Some(delay) = delays.next() {
                    tracing::debug!(
                        path,
                        status = status.as_u16(),
                        ?delay,
                        "Paperless asked to retry"
                    );
                    tokio::time::sleep(*delay).await;
                    continue;
                }
            }
            tracing::debug!(path, status = status.as_u16(), "Paperless request failed");
            return Err(PaperlessReadError::Source(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SourceFailure::Unauthorized,
                StatusCode::NOT_ACCEPTABLE => SourceFailure::UnsupportedVersion,
                _ => SourceFailure::UpstreamError,
            }));
        }
    }

    /// A file, streamed; `None` when Paperless has no such document
    pub(super) async fn download(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<Response>, PaperlessReadError> {
        self.send(path, query, None).await
    }

    /// An endpoint that exists on every Paperless; a 404 means the URL
    /// doesn't point at one
    pub(super) async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Response, PaperlessReadError> {
        self.send(path, query, Some(JSON_TIMEOUT))
            .await?
            .ok_or(PaperlessReadError::Source(SourceFailure::Unreachable))
    }

    /// One document's details; `None` when Paperless has no such document
    pub(super) async fn get_json_opt<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<T>, PaperlessReadError> {
        match self.send(path, query, Some(JSON_TIMEOUT)).await? {
            Some(response) => parse(response, path).await.map(Some),
            None => Ok(None),
        }
    }

    pub(super) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, PaperlessReadError> {
        parse(self.get(path, query).await?, path).await
    }
}

/// Reads the body in chunks up to [`MAX_JSON_BYTES`]
async fn parse<T: DeserializeOwned>(
    mut response: Response,
    path: &str,
) -> Result<T, PaperlessReadError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_failure)? {
        if body.len() + chunk.len() > MAX_JSON_BYTES {
            return Err(upstream());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|e| json_error(e, SourceLocation::file(path)))
}

/// reqwest errors quote the URL and upstream text, so only the kind survives
fn transport_failure(error: reqwest::Error) -> PaperlessReadError {
    let mut source = error.source();
    while let Some(cause) = source {
        if cause.is::<CrossOrigin>() {
            return PaperlessReadError::Source(SourceFailure::Redirected);
        }
        source = cause.source();
    }
    if error.is_redirect() || error.is_decode() || error.is_status() {
        upstream()
    } else {
        PaperlessReadError::Source(SourceFailure::Unreachable)
    }
}

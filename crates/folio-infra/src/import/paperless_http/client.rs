//! HTTP plumbing for the Paperless REST adapter: auth, API version, redirect
//! policy and the mapping of every failure to a fixed [`SourceFailure`]

use std::error::Error as _;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use folio_core::ports::{MalformedKind, PaperlessReadError, SourceFailure, SourceLocation};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use reqwest::redirect::{Attempt, Policy};
use reqwest::{Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde_json::error::Category;

/// Paperless 2.16 through 3.x accept version 9
const ACCEPT_VERSION_9: &str = "application/json; version=9";
const USER_AGENT: &str = concat!("folio/", env!("CARGO_PKG_VERSION"), " paperless-import");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;

/// Waits before each retry of a 429 or 5xx response; once they run out the
/// request fails with [`SourceFailure::UpstreamError`]
pub(super) const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// The base URL is not an http(s) URL
#[derive(Debug, thiserror::Error)]
#[error("the Paperless URL must be an http or https URL")]
pub struct InvalidPaperlessUrl;

/// One Paperless instance, reached with one token
#[derive(Clone)]
pub(super) struct Api {
    http: reqwest::Client,
    base: Url,
    retry_delays: Arc<[Duration]>,
}

impl fmt::Debug for Api {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

/// A request that didn't succeed; `NotFound` is kept apart because a
/// document can vanish between listing it and reading its details
#[derive(Debug)]
pub(super) enum Failure {
    NotFound,
    Read(PaperlessReadError),
}

impl From<Failure> for PaperlessReadError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::NotFound => PaperlessReadError::Source(SourceFailure::UpstreamError),
            Failure::Read(e) => e,
        }
    }
}

impl From<SourceFailure> for Failure {
    fn from(failure: SourceFailure) -> Self {
        Failure::Read(PaperlessReadError::Source(failure))
    }
}

#[derive(Debug)]
struct CrossOrigin;

impl fmt::Display for CrossOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("redirect to another origin")
    }
}

impl std::error::Error for CrossOrigin {}

impl Api {
    pub(super) fn new(base_url: &str, token: &str) -> Result<Self, InvalidPaperlessUrl> {
        let mut base = Url::parse(base_url).map_err(|_| InvalidPaperlessUrl)?;
        if !matches!(base.scheme(), "http" | "https") || base.cannot_be_a_base() {
            return Err(InvalidPaperlessUrl);
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(InvalidPaperlessUrl);
        }
        base.set_query(None);
        base.set_fragment(None);
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }

        let mut auth =
            HeaderValue::from_str(&format!("Token {token}")).map_err(|_| InvalidPaperlessUrl)?;
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
            retry_delays: Arc::from(RETRY_DELAYS),
        })
    }

    pub(super) fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays.into();
        self
    }

    /// GETs `path` (relative to the base URL), retrying 429 and 5xx
    pub(super) async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Response, Failure> {
        let url = self
            .base
            .join(path)
            .map_err(|_| SourceFailure::UpstreamError)?;
        let mut delays = self.retry_delays.iter();
        loop {
            let response = self
                .http
                .get(url.clone())
                .query(query)
                .send()
                .await
                .map_err(transport_failure)?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
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
            return Err(status_failure(status));
        }
    }

    pub(super) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, Failure> {
        let response = self.get(path, query).await?;
        let body = response.bytes().await.map_err(transport_failure)?;
        serde_json::from_slice(&body).map_err(|e| Failure::Read(json_error(&e, path)))
    }
}

fn status_failure(status: StatusCode) -> Failure {
    match status {
        StatusCode::NOT_FOUND => Failure::NotFound,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SourceFailure::Unauthorized.into(),
        StatusCode::NOT_ACCEPTABLE => SourceFailure::UnsupportedVersion.into(),
        _ => SourceFailure::UpstreamError.into(),
    }
}

/// reqwest errors quote the URL and upstream text, so only the kind survives
pub(super) fn transport_failure(error: reqwest::Error) -> Failure {
    let mut source = error.source();
    while let Some(cause) = source {
        if cause.is::<CrossOrigin>() {
            return SourceFailure::Redirected.into();
        }
        source = cause.source();
    }
    if error.is_redirect() || error.is_decode() || error.is_status() {
        SourceFailure::UpstreamError.into()
    } else {
        SourceFailure::Unreachable.into()
    }
}

/// Keeps serde's message out: it can quote upstream data
pub(super) fn json_error(error: &serde_json::Error, path: &str) -> PaperlessReadError {
    let kind = match error.classify() {
        Category::Eof => MalformedKind::UnexpectedEnd,
        Category::Syntax | Category::Io => MalformedKind::InvalidJson,
        Category::Data => MalformedKind::InvalidRecord,
    };
    malformed(kind, path)
}

pub(super) fn malformed(kind: MalformedKind, path: &str) -> PaperlessReadError {
    PaperlessReadError::Malformed {
        kind,
        location: SourceLocation::file(path),
    }
}

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

/// Waits before each retry of a 429 or 5xx response; once they run out the
/// request fails with [`SourceFailure::UpstreamError`]
const RETRY_DELAYS: [Duration; 4] = [
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
            retry_delays: RETRY_DELAYS.to_vec(),
        })
    }

    pub(super) fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
    }

    /// GETs `path` (relative to the base URL), retrying 429 and 5xx; `None`
    /// on 404, since a document can vanish between listing and reading it
    pub(super) async fn get_opt(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<Response>, PaperlessReadError> {
        let url = self.base.join(path).map_err(|_| upstream())?;
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

    pub(super) async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Response, PaperlessReadError> {
        self.get_opt(path, query).await?.ok_or_else(upstream)
    }

    pub(super) async fn get_json_opt<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<T>, PaperlessReadError> {
        let Some(response) = self.get_opt(path, query).await? else {
            return Ok(None);
        };
        let body = response.bytes().await.map_err(transport_failure)?;
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| json_error(e, SourceLocation::file(path)))
    }

    pub(super) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, PaperlessReadError> {
        self.get_json_opt(path, query).await?.ok_or_else(upstream)
    }
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

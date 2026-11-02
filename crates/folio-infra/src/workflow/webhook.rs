//! Signed webhook delivery

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use sha2::Sha256;

use folio_core::models::{validate_webhook_url, Id, WorkflowTrigger};

/// Per-request time limit, connect and response included
pub const TIMEOUT: Duration = Duration::from_secs(10);

const USER_AGENT: &str = concat!("Folio-Webhooks/", env!("CARGO_PKG_VERSION"));

/// How much of an error response body is kept in `last_error`
const ERROR_BODY_LIMIT: usize = 200;

/// `sha256=<hex>` HMAC of `<timestamp>.<body>`, as sent in `X-Folio-Signature`
pub fn sign(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// One delivery attempt
pub struct WebhookRequest<'a> {
    pub url: &'a str,
    pub headers: &'a BTreeMap<String, String>,
    pub secret: Option<&'a str>,
    pub event: WorkflowTrigger,
    pub run_id: Id,
    /// Position in the run's persisted action snapshot.
    pub action_index: usize,
    pub body: &'a str,
}

/// What came back from a delivery attempt
#[derive(Debug, Clone)]
pub struct DeliveryResult {
    pub status: Option<u16>,
    pub error: Option<String>,
    /// Whether trying again could help (network trouble, 5xx, 408, 429)
    pub retryable: bool,
    pub duration_ms: u64,
}

impl DeliveryResult {
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

/// HTTP client that delivers webhooks
///
/// Redirects are not followed: a webhook target that redirects is
/// misconfigured, and following would let it bounce deliveries elsewhere.
#[derive(Clone)]
pub struct WebhookClient {
    client: reqwest::Client,
}

impl WebhookClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("static client configuration is valid");
        Self { client }
    }

    pub async fn send(&self, request: WebhookRequest<'_>) -> DeliveryResult {
        let started = Instant::now();
        let outcome = self.attempt(&request).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        match outcome {
            Ok((status, error)) => {
                let retryable = matches!(status, 408 | 429 | 500..=599);
                DeliveryResult {
                    status: Some(status),
                    error,
                    retryable,
                    duration_ms,
                }
            }
            Err(failure) => DeliveryResult {
                status: None,
                error: Some(failure.message),
                retryable: failure.retryable,
                duration_ms,
            },
        }
    }

    async fn attempt(
        &self,
        request: &WebhookRequest<'_>,
    ) -> Result<(u16, Option<String>), Failure> {
        validate_webhook_url(request.url).map_err(|e| Failure::permanent(e.to_string()))?;

        let timestamp = chrono::Utc::now().timestamp();
        let mut headers = HeaderMap::new();
        for (name, value) in request.headers {
            let name = HeaderName::from_bytes(name.trim().as_bytes())
                .map_err(|e| Failure::permanent(format!("Invalid header name '{name}': {e}")))?;
            let value = HeaderValue::from_str(value).map_err(|e| {
                Failure::permanent(format!("Invalid value for header '{name}': {e}"))
            })?;
            headers.insert(name, value);
        }
        // Set last so custom headers cannot spoof them.
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("x-folio-timestamp", timestamp.into());
        headers.insert(
            "x-folio-event",
            HeaderValue::from_static(request.event.as_str()),
        );
        headers.insert(
            "x-folio-delivery",
            HeaderValue::from_str(
                &Id::new_v5(
                    &request.run_id,
                    &(request.action_index as u64).to_be_bytes(),
                )
                .to_string(),
            )
            .expect("UUIDs are ASCII"),
        );
        headers.insert(
            "x-folio-run",
            HeaderValue::from_str(&request.run_id.to_string()).expect("UUIDs are ASCII"),
        );
        headers.remove("x-folio-signature");
        if let Some(secret) = request.secret.filter(|s| !s.is_empty()) {
            headers.insert(
                "x-folio-signature",
                HeaderValue::from_str(&sign(secret, timestamp, request.body))
                    .expect("signature is ASCII"),
            );
        }

        let mut response = self
            .client
            .post(request.url)
            .headers(headers)
            .body(request.body.to_string())
            .send()
            .await
            .map_err(|e| Failure::transient(describe(e.without_url())))?;

        let status = response.status();
        if status.is_success() {
            return Ok((status.as_u16(), None));
        }

        let snippet = match response.chunk().await {
            Ok(Some(chunk)) => {
                let end = chunk.len().min(ERROR_BODY_LIMIT);
                String::from_utf8_lossy(&chunk[..end]).trim().to_string()
            }
            _ => String::new(),
        };
        let message = if snippet.is_empty() {
            format!("HTTP {status}")
        } else {
            format!("HTTP {status}: {snippet}")
        };
        Ok((status.as_u16(), Some(message)))
    }
}

impl Default for WebhookClient {
    fn default() -> Self {
        Self::new()
    }
}

struct Failure {
    message: String,
    retryable: bool,
}

impl Failure {
    fn permanent(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }

    fn transient(message: String) -> Self {
        Self {
            message,
            retryable: true,
        }
    }
}

fn describe(error: reqwest::Error) -> String {
    if error.is_timeout() {
        format!("Request timed out after {}s", TIMEOUT.as_secs())
    } else if error.is_connect() {
        format!("Could not connect: {error}")
    } else {
        error.to_string()
    }
}

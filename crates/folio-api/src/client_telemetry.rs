use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::Semaphore;
use url::Url;
use utoipa::ToSchema;

use crate::vault::Uploader;
use crate::AppState;

const MAX_BODY: usize = 1024 * 1024;
const MAX_REPLY: usize = 64 * 1024;

/// Server-owned OTLP destinations and credentials for authenticated clients.
pub struct ClientTelemetry {
    client: reqwest::Client,
    signals: HashMap<&'static str, (Url, HeaderMap)>,
    concurrent: Semaphore,
    budget: Mutex<Budget>,
}

struct Budget {
    started: Instant,
    requests: u32,
    users: HashMap<String, u32>,
}

impl Budget {
    fn admit(&mut self, user: &str) -> bool {
        if self.started.elapsed() >= Duration::from_secs(60) {
            self.started = Instant::now();
            self.requests = 0;
            self.users.clear();
        }
        if self.requests >= 1200 || (!self.users.contains_key(user) && self.users.len() >= 1024) {
            return false;
        }
        let count = self.users.entry(user.to_owned()).or_default();
        if *count >= 120 {
            return false;
        }
        *count += 1;
        self.requests += 1;
        true
    }
}

impl ClientTelemetry {
    /// Read opt-in client export settings without exposing credentials to callers.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        if get("FOLIO_TELEMETRY__CLIENTS_ENABLED").as_deref() != Some("true")
            || get("OTEL_SDK_DISABLED").is_some_and(|v| v.eq_ignore_ascii_case("true"))
        {
            return Ok(None);
        }
        let protocol = get("OTEL_EXPORTER_OTLP_PROTOCOL");
        let base = get("FOLIO_TELEMETRY__OTLP_HTTP_ENDPOINT").or_else(|| {
            if protocol.as_deref().unwrap_or("http/protobuf") == "http/protobuf" {
                get("OTEL_EXPORTER_OTLP_ENDPOINT")
            } else {
                None
            }
        });
        let mut signals = HashMap::new();
        for (signal, name) in [
            ("traces", "TRACES"),
            ("metrics", "METRICS"),
            ("logs", "LOGS"),
        ] {
            if get(&format!("OTEL_{name}_EXPORTER")).as_deref() == Some("none") {
                continue;
            }
            let specific = get(&format!("OTEL_EXPORTER_OTLP_{name}_ENDPOINT"));
            let endpoint = specific.or_else(|| {
                base.as_ref()
                    .map(|b| format!("{}/v1/{signal}", b.trim_end_matches('/')))
            });
            let Some(endpoint) = endpoint else { continue };
            let url = Url::parse(&endpoint).map_err(|_| "invalid telemetry HTTP endpoint")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("invalid telemetry HTTP endpoint".into());
            }
            let raw_headers = get(&format!("OTEL_EXPORTER_OTLP_{name}_HEADERS"))
                .or_else(|| get("OTEL_EXPORTER_OTLP_HEADERS"))
                .unwrap_or_default();
            signals.insert(signal, (url, export_headers(&raw_headers)?));
        }
        if signals.is_empty() {
            return Ok(None);
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "cannot create telemetry HTTP client")?;
        Ok(Some(Self {
            client,
            signals,
            concurrent: Semaphore::new(4),
            budget: Mutex::new(Budget {
                started: Instant::now(),
                requests: 0,
                users: HashMap::new(),
            }),
        }))
    }
}

fn export_headers(raw: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for entry in raw.split(',').filter(|v| !v.trim().is_empty()) {
        let (key, value) = entry
            .trim()
            .split_once('=')
            .ok_or("invalid telemetry export headers")?;
        let name = HeaderName::from_bytes(key.trim().as_bytes())
            .map_err(|_| "invalid telemetry export headers")?;
        if matches!(
            name.as_str(),
            "host"
                | "content-type"
                | "content-length"
                | "connection"
                | "transfer-encoding"
                | "cookie"
                | "traceparent"
                | "tracestate"
        ) {
            return Err("unsupported telemetry export header".into());
        }
        let encoded = format!("v={}", value.trim().replace('+', "%2B"));
        let decoded = url::form_urlencoded::parse(encoded.as_bytes())
            .next()
            .unwrap()
            .1
            .into_owned();
        headers.insert(
            name,
            HeaderValue::from_str(&decoded).map_err(|_| "invalid telemetry export headers")?,
        );
    }
    Ok(headers)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Configuration {
    enabled: bool,
    endpoint: &'static str,
    service_namespace: &'static str,
    service_version: &'static str,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/config", get(configuration))
        .route("/v1/{signal}", post(export))
}

#[utoipa::path(get, path = "/api/telemetry/config", tag = "telemetry",
    responses((status = 200, description = "Client telemetry configuration without export credentials", body = Configuration),
    (status = 401, description = "Authentication required")))]
pub(crate) async fn configuration(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(Configuration {
            enabled: state.client_telemetry.is_some(),
            endpoint: "/api/telemetry",
            service_namespace: "folio",
            service_version: env!("CARGO_PKG_VERSION"),
        }),
    )
}

#[utoipa::path(post, path = "/api/telemetry/v1/{signal}", tag = "telemetry",
    params(("signal" = String, Path, description = "OTLP signal: traces, metrics or logs")),
    responses((status = 200, description = "OTLP export response"),
    (status = 401, description = "Authentication required"),
    (status = 404, description = "Client export disabled or unsupported signal"),
    (status = 408, description = "Payload deadline exceeded"),
    (status = 413, description = "Payload exceeds one MiB"),
    (status = 415, description = "Unsupported content type or encoding"),
    (status = 429, description = "Export budget exceeded"),
    (status = 502, description = "Collector unavailable")))]
pub(crate) async fn export(
    State(state): State<AppState>,
    Path(signal): Path<String>,
    Uploader(user): Uploader,
    request: Request,
) -> Response {
    let Some(relay) = state.client_telemetry else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((url, headers)) = relay.signals.get(signal.as_str()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if !matches!(content_type, "application/json" | "application/x-protobuf")
        || request.headers().contains_key(header::CONTENT_ENCODING)
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let content_type = content_type.to_owned();
    let admitted = relay
        .budget
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admit(user.as_ref().map_or("anonymous", |u| u.as_str()));
    if !admitted {
        return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "60")]).into_response();
    }
    let Ok(_permit) = relay.concurrent.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let body = match tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(request.into_body(), MAX_BODY),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    let result = relay
        .client
        .post(url.clone())
        .headers(headers.clone())
        .header(header::CONTENT_TYPE, &content_type)
        .body(body)
        .send()
        .await;
    let Ok(mut upstream) = result else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let status = upstream.status();
    if !status.is_success() {
        return if status == StatusCode::TOO_MANY_REQUESTS {
            (status, [(header::RETRY_AFTER, "60")]).into_response()
        } else {
            StatusCode::BAD_GATEWAY.into_response()
        };
    }
    let mut reply = Vec::new();
    loop {
        match upstream.chunk().await {
            Ok(Some(bytes)) if reply.len() + bytes.len() <= MAX_REPLY => {
                reply.extend_from_slice(&bytes)
            }
            Ok(None) => break,
            _ => return StatusCode::BAD_GATEWAY.into_response(),
        }
    }
    (
        status,
        [(header::CONTENT_TYPE, content_type)],
        Body::from(reply),
    )
        .into_response()
}

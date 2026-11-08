//! Apple app site association, so self-built Folio apps can open this server's links

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use folio_core::models::AppleAppId;
use serde_json::{json, Value};

use crate::error::ApiResult;
use crate::state::AppState;

/// Settings-table key holding newline-separated app IDs
pub(crate) const SETTING_KEY: &str = "apple.app_ids";

/// Matches exactly one UUID path segment; Apple patterns treat `?` as one character
const UUID_MASK: &str = "????????-????-????-????-????????????";

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/.well-known/apple-app-site-association",
        get(app_site_association),
    )
}

/// Where the effective app IDs come from
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppIdSource {
    Settings,
    Config,
    Unconfigured,
    /// The stored row does not parse; the startup value applies instead
    Invalid(String),
}

/// A non-empty, valid settings row wins; otherwise the startup value applies
pub(crate) fn resolve(
    stored: Option<&str>,
    default: &[AppleAppId],
) -> (Vec<AppleAppId>, AppIdSource) {
    let fallback = || {
        if default.is_empty() {
            (Vec::new(), AppIdSource::Unconfigured)
        } else {
            (default.to_vec(), AppIdSource::Config)
        }
    };
    match stored.map(AppleAppId::parse_list) {
        Some(Ok(ids)) if !ids.is_empty() => (ids, AppIdSource::Settings),
        Some(Err(error)) => (default.to_vec(), AppIdSource::Invalid(error.to_string())),
        _ => fallback(),
    }
}

async fn app_site_association(State(state): State<AppState>) -> ApiResult<Response> {
    let stored = state.settings_repo.get(SETTING_KEY).await?;
    let (app_ids, source) = resolve(
        stored.as_ref().map(|s| s.value.as_str()),
        &state.apple_app_ids_default,
    );
    if let AppIdSource::Invalid(error) = &source {
        tracing::warn!(%error, "ignoring invalid {SETTING_KEY} setting");
    }
    if app_ids.is_empty() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    Ok((
        [(header::CACHE_CONTROL, "public, max-age=3600")],
        Json(association(&app_ids, &path_prefix(&state.public_url))),
    )
        .into_response())
}

/// Scheme, host and port of `public_url`: Apple only fetches the file from the domain root
pub(crate) fn origin(public_url: &str) -> String {
    url::Url::parse(public_url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| public_url.trim_end_matches('/').to_string())
}

/// The path of `public_url` without a trailing slash; empty at the domain root
fn path_prefix(public_url: &str) -> String {
    url::Url::parse(public_url)
        .map(|url| url.path().trim_end_matches('/').to_string())
        .unwrap_or_default()
}

/// Claims only the pages Handoff hands over: a document, a saved view and a vault library
fn association(app_ids: &[AppleAppId], prefix: &str) -> Value {
    let u = UUID_MASK;
    let app_ids: Vec<&str> = app_ids.iter().map(AppleAppId::as_str).collect();
    json!({
        "applinks": {
            "details": [{
                "appIDs": app_ids,
                "components": [
                    { "/": format!("{prefix}/vaults/{u}/documents/{u}") },
                    { "/": format!("{prefix}/vaults/{u}/saved-views/{u}") },
                    { "/": format!("{prefix}/vaults/{u}/") },
                ]
            }]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &str) -> Vec<AppleAppId> {
        AppleAppId::parse_list(list).unwrap()
    }

    #[test]
    fn resolve_prefers_a_valid_stored_row_and_falls_back_otherwise() {
        let config = ids("ABCDE12345.config");
        assert_eq!(
            resolve(Some("FGHIJ67890.stored"), &config),
            (ids("FGHIJ67890.stored"), AppIdSource::Settings)
        );
        assert_eq!(
            resolve(Some(" "), &config),
            (config.clone(), AppIdSource::Config)
        );
        assert_eq!(
            resolve(None, &config),
            (config.clone(), AppIdSource::Config)
        );
        assert_eq!(resolve(None, &[]), (Vec::new(), AppIdSource::Unconfigured));
        let (served, source) = resolve(Some("nope"), &config);
        assert_eq!(served, config);
        assert!(matches!(source, AppIdSource::Invalid(e) if e.contains("nope")));
    }
}

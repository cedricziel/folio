//! Static assets, embedded in the binary.
//!
//! Debug builds read `static/` from disk, so edits show up without a rebuild.

use axum::{
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "../../static"]
struct Assets;

pub fn router() -> Router<AppState> {
    Router::new().route("/static/*path", get(serve))
}

/// Vendored scripts, fonts and icons only change with an upgrade, so browsers
/// may keep them for a week. Our own files are revalidated on every load.
fn cache_control(path: &str) -> &'static str {
    const LONG_LIVED: [&str; 3] = ["vendor/", "fonts/", "icons/"];
    if LONG_LIVED.iter().any(|dir| path.starts_with(dir)) {
        "public, max-age=604800"
    } else {
        "no-cache"
    }
}

async fn serve(Path(path): Path<String>, headers: HeaderMap) -> Response {
    let Some(file) = Assets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let hash: String = file
        .metadata
        .sha256_hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let etag = format!("\"{hash}\"");
    let cache = cache_control(&path);

    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag);
    if unchanged {
        return (
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, etag), (header::CACHE_CONTROL, cache.into())],
        )
            .into_response();
    }

    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache.into()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_files_are_cached_longer_than_our_own() {
        assert_eq!(cache_control("vendor/turbo.js"), "public, max-age=604800");
        assert_eq!(cache_control("fonts/fonts.css"), "public, max-age=604800");
        assert_eq!(cache_control("style.css"), "no-cache");
    }
}

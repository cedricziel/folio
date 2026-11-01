//! Headers for responses that carry user-supplied file bytes.
//!
//! Uploads are untrusted: an SVG or HTML file served inline from Folio's
//! origin would run script with the user's session.

use axum::{
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
};

/// Types a browser may render inline; everything else is a download.
const INLINE_TYPES: [&str; 7] = [
    "application/pdf",
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/heic",
    "image/heif",
];

/// Builds a file response that cannot execute active content.
///
/// `force_attachment` makes explicit downloads attachments even for types that
/// are safe to show inline.
pub(crate) fn file_response(
    mime: &str,
    filename: Option<&str>,
    force_attachment: bool,
    data: Vec<u8>,
) -> Response {
    let essence = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let inline = !force_attachment && INLINE_TYPES.contains(&essence.as_str());
    let mut disposition = String::from(if inline { "inline" } else { "attachment" });
    if let Some(name) = filename {
        let safe: String = name
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '"' | '\\' | '/'))
            .collect();
        disposition.push_str(&format!("; filename=\"{safe}\""));
    }
    let mut response = data.into_response();
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    harden(&mut response);
    response
}

/// Adds the sniffing and sandbox headers to a response that is not user data
/// (generated thumbnails, placeholders).
pub(crate) fn harden(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
}

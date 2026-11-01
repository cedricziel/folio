//! Headers for responses that carry user-supplied file bytes.
//!
//! Uploads are untrusted: an SVG or HTML file served inline from Folio's
//! origin would run script with the user's session.

use axum::{
    http::{header, HeaderValue},
    response::Response,
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

/// Adds the sniffing and sandbox headers. Applies to every file response,
/// including generated ones.
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

/// Hardens a file response and sets `Content-Disposition`: inline for the
/// allowlisted types, attachment for anything else.
pub(crate) fn harden_inline(response: &mut Response, mime: &str) {
    let essence = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let disposition = if INLINE_TYPES.contains(&essence.as_str()) {
        "inline"
    } else {
        "attachment"
    };
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static(disposition),
    );
    harden(response);
}

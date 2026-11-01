//! Redirect URI validation and open-redirect protection.

use url::{Host, Url};

const FORBIDDEN_SCHEMES: [&str; 6] = ["javascript", "data", "file", "vbscript", "about", "blob"];

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Registration-time check: https, http on loopback, or a custom scheme
/// for native apps. No fragments.
pub fn validate_redirect_uri(uri: &str) -> Result<Url, String> {
    let url = Url::parse(uri).map_err(|_| format!("not a valid URI: {uri}"))?;
    if url.fragment().is_some() {
        return Err("redirect_uri must not contain a fragment".into());
    }
    match url.scheme() {
        "https" => {
            if url.host().is_none() {
                return Err("https redirect_uri needs a host".into());
            }
        }
        "http" => {
            if !is_loopback(&url) {
                return Err("http redirect_uri is only allowed for localhost".into());
            }
        }
        s if FORBIDDEN_SCHEMES.contains(&s) => {
            return Err(format!("scheme {s} is not allowed"));
        }
        _ => {}
    }
    Ok(url)
}

/// Authorization-time match. Exact, except that loopback http URIs may differ
/// in port (RFC 8252 section 7.3).
pub fn redirect_uri_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }
    let (Ok(a), Ok(b)) = (Url::parse(registered), Url::parse(requested)) else {
        return false;
    };
    a.scheme() == "http"
        && b.scheme() == "http"
        && is_loopback(&a)
        && is_loopback(&b)
        && a.host() == b.host()
        && a.path() == b.path()
        && a.query() == b.query()
}

/// Accepts only same-site absolute paths for post-login redirects.
pub fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.contains('\\')
                && !n.chars().any(|c| c.is_control()) =>
        {
            n.to_string()
        }
        _ => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_rules() {
        assert!(validate_redirect_uri("https://claude.ai/api/mcp/auth_callback").is_ok());
        assert!(validate_redirect_uri("http://localhost:33418/cb").is_ok());
        assert!(validate_redirect_uri("http://127.0.0.1/cb").is_ok());
        assert!(validate_redirect_uri("http://[::1]:1234/cb").is_ok());
        assert!(validate_redirect_uri("com.example.app:/callback").is_ok());
        assert!(validate_redirect_uri("http://evil.example/cb").is_err());
        assert!(validate_redirect_uri("javascript:alert(1)").is_err());
        assert!(validate_redirect_uri("https://a.example/cb#frag").is_err());
        assert!(validate_redirect_uri("nonsense").is_err());
    }

    #[test]
    fn loopback_port_is_flexible_others_exact() {
        assert!(redirect_uri_matches(
            "http://127.0.0.1:1/cb",
            "http://127.0.0.1:9/cb"
        ));
        assert!(!redirect_uri_matches(
            "http://127.0.0.1:1/cb",
            "http://127.0.0.1:9/other"
        ));
        assert!(!redirect_uri_matches(
            "https://a.example/cb",
            "https://a.example:444/cb"
        ));
    }

    #[test]
    fn next_must_be_local() {
        assert_eq!(
            safe_next(Some("/oauth/authorize?a=b")),
            "/oauth/authorize?a=b"
        );
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("/\\evil.example")), "/");
        assert_eq!(safe_next(None), "/");
    }
}

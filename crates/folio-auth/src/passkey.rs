//! WebAuthn relying party setup.

use url::Url;
pub use webauthn_rs::prelude::{
    DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Uuid,
};
pub use webauthn_rs::Webauthn;
use webauthn_rs::WebauthnBuilder;

/// The relying party is the host of `base_url` and the origin its scheme,
/// host and port. Both come from `server.public_url`, never from a request.
pub fn build_webauthn(base_url: &str) -> Result<Webauthn, String> {
    let url = Url::parse(base_url).map_err(|e| e.to_string())?;
    let host = url.host_str().ok_or("the public URL has no host")?;
    let origin = Url::parse(&url.origin().ascii_serialization()).map_err(|e| e.to_string())?;
    WebauthnBuilder::new(host, &origin)
        .and_then(|b| b.rp_name("Folio").build())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_for_a_domain_and_localhost_but_not_an_ip() {
        assert!(build_webauthn("https://folio.example.com").is_ok());
        assert!(build_webauthn("https://folio.example.com/sub").is_ok());
        assert!(build_webauthn("http://localhost:3000").is_ok());
        assert!(build_webauthn("http://127.0.0.1:3000").is_err());
    }
}

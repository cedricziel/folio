//! Encryption at rest for small secrets (TOTP seeds) and keyed hashing of
//! recovery codes, both under the operator's `auth.secret_key`.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

use crate::crypto::random_token;

const NONCE_LEN: usize = 24;
const MIN_KEY_LEN: usize = 16;

/// Seals and opens secrets with XChaCha20-Poly1305. The key is derived from
/// the configured secret, so any string of at least 16 characters works.
#[derive(Clone)]
pub struct SecretBox {
    cipher_key: [u8; 32],
    mac_key: [u8; 32],
}

fn derive(secret: &str, label: &str) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(label.as_bytes());
    mac.finalize().into_bytes().into()
}

impl SecretBox {
    pub fn new(secret: &str) -> Result<Self, String> {
        if secret.len() < MIN_KEY_LEN {
            return Err(format!(
                "the secret key must be at least {MIN_KEY_LEN} characters long"
            ));
        }
        Ok(Self {
            cipher_key: derive(secret, "folio/secret-box/encryption/v1"),
            mac_key: derive(secret, "folio/secret-box/mac/v1"),
        })
    }

    /// A fresh random 256-bit key, base64url encoded.
    pub fn generate_key() -> String {
        random_token()
    }

    /// Encrypts `plaintext`. `context` is authenticated but not stored, so a
    /// ciphertext cannot be moved to another user's row.
    pub fn seal(&self, plaintext: &[u8], context: &str) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut out = nonce.to_vec();
        let sealed = XChaCha20Poly1305::new(&self.cipher_key.into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: context.as_bytes(),
                },
            )
            .expect("encrypting in memory cannot fail");
        out.extend(sealed);
        URL_SAFE_NO_PAD.encode(out)
    }

    /// `None` for a wrong key, a different `context` or damaged data.
    pub fn open(&self, sealed: &str, context: &str) -> Option<Vec<u8>> {
        let raw = URL_SAFE_NO_PAD.decode(sealed).ok()?;
        if raw.len() <= NONCE_LEN {
            return None;
        }
        let (nonce, ciphertext) = raw.split_at(NONCE_LEN);
        XChaCha20Poly1305::new(&self.cipher_key.into())
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .ok()
    }

    /// Keyed hash for low-entropy-ish secrets such as recovery codes: a stolen
    /// database alone cannot be brute-forced.
    pub fn keyed_hash(&self, message: &str) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.mac_key).expect("any key size");
        mac.update(message.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef-test-key";

    #[test]
    fn round_trip() {
        let b = SecretBox::new(KEY).unwrap();
        let sealed = b.seal(b"hunter2", "user-1");
        assert!(!sealed.contains("hunter2"));
        assert_eq!(b.open(&sealed, "user-1").unwrap(), b"hunter2");
        assert_ne!(sealed, b.seal(b"hunter2", "user-1"), "nonce is random");
    }

    #[test]
    fn wrong_key_context_or_damage_fails() {
        let b = SecretBox::new(KEY).unwrap();
        let sealed = b.seal(b"seed", "user-1");
        let other = SecretBox::new("another-key-another-key").unwrap();
        assert!(other.open(&sealed, "user-1").is_none());
        assert!(b.open(&sealed, "user-2").is_none());
        let mut damaged = sealed.clone();
        damaged.push('A');
        assert!(b.open(&damaged, "user-1").is_none());
        assert!(b.open("", "user-1").is_none());
        assert!(b.open("short", "user-1").is_none());
    }

    #[test]
    fn short_keys_are_refused() {
        assert!(SecretBox::new("short").is_err());
        assert!(SecretBox::new(&SecretBox::generate_key()).is_ok());
    }

    #[test]
    fn keyed_hash_depends_on_the_key() {
        let a = SecretBox::new(KEY).unwrap();
        let b = SecretBox::new("another-key-another-key").unwrap();
        assert_eq!(a.keyed_hash("x"), a.keyed_hash("x"));
        assert_ne!(a.keyed_hash("x"), b.keyed_hash("x"));
    }
}

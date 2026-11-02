//! Time-based one-time passwords and recovery codes.

use qrcode::{render::svg, QrCode};
use rand::{RngExt, TryRng};
use totp_rs::{Builder, Secret, Totp};

/// Shown in the authenticator app next to the account name.
pub const ISSUER: &str = "Folio";
/// Recovery codes issued at a time.
pub const RECOVERY_CODE_COUNT: usize = 10;
const SECRET_BYTES: usize = 20;
const RECOVERY_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

pub fn new_totp_secret() -> Vec<u8> {
    let mut bytes = vec![0u8; SECRET_BYTES];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS random source is unavailable");
    bytes
}

/// SHA-1, six digits, 30 seconds and one step of skew either way: what
/// every authenticator app supports.
pub fn totp(secret: &[u8], account: &str) -> Result<Totp, String> {
    Builder::new()
        .with_secret(Secret::new(secret.to_vec().into_boxed_slice()))
        .with_skew(1)
        .with_issuer(Some(ISSUER))
        .with_account_name(account)
        .build()
        .map_err(|e| e.to_string())
}

/// The step the code belongs to, when it is valid at `now` and newer than
/// `last_step`. Accepting only newer steps rejects a replayed code.
pub fn check_code(totp: &Totp, code: &str, now: u64, last_step: i64) -> Option<i64> {
    let digits: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let step = i64::try_from(totp.check(&digits, now)?).ok()?;
    (step > last_step).then_some(step)
}

pub fn base32_secret(secret: &[u8]) -> String {
    Secret::new(secret.to_vec().into_boxed_slice()).to_base32()
}

/// The `otpauth://` URI an authenticator app reads from the QR code.
pub fn otpauth_uri(totp: &Totp) -> Result<String, String> {
    totp.to_url().map_err(|e| e.to_string())
}

/// The URI as an inline SVG, drawn on the server so no script or CDN is needed.
pub fn qr_svg(uri: &str) -> Result<String, String> {
    let code = QrCode::new(uri.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// Ten codes like `k3m9-x7qp-4wzd`, 60 bits each.
pub fn generate_recovery_codes() -> Vec<String> {
    let mut rng = rand::rand_core::UnwrapErr(rand::rngs::SysRng);
    (0..RECOVERY_CODE_COUNT)
        .map(|_| {
            let chars: Vec<char> = (0..12)
                .map(|_| {
                    char::from(RECOVERY_ALPHABET[rng.random_range(0..RECOVERY_ALPHABET.len())])
                })
                .collect();
            chars
                .chunks(4)
                .map(|c| c.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect()
}

/// Lower case without spaces and dashes, so `K3M9 X7QP-4WZD` matches.
pub fn normalize_recovery_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Totp {
        totp(&[7u8; 20], "ada").unwrap()
    }

    #[test]
    fn accepts_the_current_step_and_one_step_either_way() {
        let t = sample();
        let now = 1_700_000_010;
        for offset in [-30i64, 0, 30] {
            let code = t.generate((now as i64 + offset) as u64).to_string();
            assert!(check_code(&t, &code, now, 0).is_some(), "offset {offset}");
        }
        let far = t.generate(now + 90).to_string();
        assert!(check_code(&t, &far, now, 0).is_none());
    }

    #[test]
    fn a_step_is_accepted_once() {
        let t = sample();
        let now = 1_700_000_010;
        let code = t.generate(now).to_string();
        let step = check_code(&t, &code, now, 0).unwrap();
        assert_eq!(check_code(&t, &code, now, step), None);
        let older = t.generate(now - 30).to_string();
        assert_eq!(check_code(&t, &older, now, step), None);
    }

    #[test]
    fn malformed_codes_are_refused() {
        let t = sample();
        for bad in ["", "12345", "1234567", "abcdef", "12 34"] {
            assert!(check_code(&t, bad, 1_700_000_010, 0).is_none(), "{bad:?}");
        }
        let spaced = t.generate(1_700_000_010).to_string();
        let spaced = format!("{} {}", &spaced[..3], &spaced[3..]);
        assert!(check_code(&t, &spaced, 1_700_000_010, 0).is_some());
    }

    #[test]
    fn uri_and_qr() {
        let t = sample();
        let uri = otpauth_uri(&t).unwrap();
        assert!(uri.starts_with("otpauth://totp/Folio:ada?"), "{uri}");
        assert!(uri.contains("issuer=Folio"));
        assert!(qr_svg(&uri).unwrap().contains("<svg"));
        assert_eq!(base32_secret(&[7u8; 20]).len(), 32);
    }

    #[test]
    fn recovery_codes_are_distinct_and_normalise() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let mut unique = codes.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), RECOVERY_CODE_COUNT);
        assert_eq!(codes[0].len(), 14);
        assert_eq!(
            normalize_recovery_code(&codes[0].to_uppercase().replace('-', " ")),
            normalize_recovery_code(&codes[0])
        );
    }
}

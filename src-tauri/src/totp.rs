//! Time-based one-time passwords (RFC 6238), the serverless second factor.
//!
//! This is real two-factor with no email and no network at all: a secret is
//! generated here, shown once as a QR code, and scanned into an authenticator
//! app (Google Authenticator, Aegis, and the rest). The app then shows a fresh
//! six-digit code every thirty seconds, computed from that shared secret and
//! the clock. To confirm a reveal the user types the current code, and this
//! module checks it against the same computation.
//!
//! The second factor is genuine because the secret lives on a separate device
//! (the phone). Someone sitting at an unlocked machine does not have it, which
//! is the whole point. Standard HMAC-SHA1, six digits, thirty-second step.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

use crate::error::{Result, WalletError};

const DIGITS: u32 = 6;
const PERIOD: i64 = 30;
/// How many steps of clock skew each way are accepted, so a phone a few seconds
/// off, or a code entered right on a boundary, still passes.
const SKEW: i64 = 1;
const SECRET_LEN: usize = 20;

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A fresh random secret, as the base32 an authenticator app imports.
pub fn new_secret() -> String {
    let mut bytes = [0u8; SECRET_LEN];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

/// The otpauth URI an authenticator scans from a QR code.
pub fn provisioning_uri(secret: &str, account: &str) -> String {
    format!(
        "otpauth://totp/ShinyFlakes:{account}?secret={secret}&issuer=ShinyFlakes\
         &algorithm=SHA1&digits={DIGITS}&period={PERIOD}"
    )
}

/// The code for a secret at a Unix time. Used to verify what the user types.
fn code_at(secret_bytes: &[u8], unix: i64) -> u32 {
    let counter = (unix / PERIOD) as u64;
    hotp(secret_bytes, counter)
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = <Hmac<Sha1>>::new_from_slice(secret).expect("hmac takes any key length");
    mac.update(&counter.to_be_bytes());
    let hs = mac.finalize().into_bytes();

    // Dynamic truncation, per RFC 4226.
    let offset = (hs[hs.len() - 1] & 0x0f) as usize;
    let bin = ((hs[offset] as u32 & 0x7f) << 24)
        | ((hs[offset + 1] as u32) << 16)
        | ((hs[offset + 2] as u32) << 8)
        | (hs[offset + 3] as u32);
    bin % 10u32.pow(DIGITS)
}

/// Whether `input` is the valid code for `secret` right now, allowing a little
/// clock skew. `secret` is the base32 shown at setup.
pub fn verify(secret: &str, input: &str) -> Result<bool> {
    verify_at(secret, input, now())
}

fn verify_at(secret: &str, input: &str, unix: i64) -> Result<bool> {
    let entered: u32 = match input.trim().parse() {
        Ok(n) => n,
        Err(_) => return Ok(false),
    };
    let bytes = base32_decode(secret)?;
    for step in -SKEW..=SKEW {
        if code_at(&bytes, unix + step * PERIOD) == entered {
            return Ok(true);
        }
    }
    Ok(false)
}

fn now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for c in s.trim().chars() {
        if c == '=' || c.is_whitespace() {
            continue;
        }
        let up = c.to_ascii_uppercase() as u8;
        let val = BASE32
            .iter()
            .position(|&b| b == up)
            .ok_or_else(|| WalletError::Derivation("bad base32 in the 2FA secret".into()))?;
        buffer = (buffer << 5) | val as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The RFC 6238 reference secret: ASCII "12345678901234567890".
    const REF: &[u8] = b"12345678901234567890";

    #[test]
    fn matches_the_rfc6238_vectors() {
        // Six-digit truncation of the published SHA1 vectors.
        assert_eq!(code_at(REF, 59), 287082);
        assert_eq!(code_at(REF, 1111111109), 81804);
        assert_eq!(code_at(REF, 1234567890), 5924);
        assert_eq!(code_at(REF, 2000000000), 279037);
    }

    #[test]
    fn base32_round_trips() {
        // The reference secret's known base32 form.
        assert_eq!(base32_encode(REF), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(base32_decode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap(), REF);
        for len in 1..=32 {
            let data: Vec<u8> = (0..len as u8).collect();
            assert_eq!(base32_decode(&base32_encode(&data)).unwrap(), data);
        }
    }

    #[test]
    fn a_fresh_secret_verifies_its_own_current_code() {
        let secret = new_secret();
        let bytes = base32_decode(&secret).unwrap();
        let t = 1_700_000_000;
        let code = format!("{:06}", code_at(&bytes, t));
        assert!(verify_at(&secret, &code, t).unwrap());
        // A wrong code fails.
        assert!(!verify_at(&secret, "000000", t).unwrap());
    }

    #[test]
    fn a_code_from_the_step_before_still_passes() {
        let secret = new_secret();
        let bytes = base32_decode(&secret).unwrap();
        let t = 1_700_000_000;
        // A code computed one step ago is accepted when entered now (skew).
        let earlier = format!("{:06}", code_at(&bytes, t - PERIOD));
        assert!(verify_at(&secret, &earlier, t).unwrap());
        // But two steps is too far.
        let older = format!("{:06}", code_at(&bytes, t - 3 * PERIOD));
        assert!(!verify_at(&secret, &older, t).unwrap());
    }

    #[test]
    fn junk_input_is_just_wrong_not_an_error() {
        let secret = new_secret();
        assert!(!verify(&secret, "notacode").unwrap());
        assert!(!verify(&secret, "").unwrap());
    }
}

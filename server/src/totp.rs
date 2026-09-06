//! RFC 6238 TOTP(SHA-1,6 位,30 s),兼容 Google Authenticator / 1Password 等。

use hmac::{Hmac, Mac};
use sha1::Sha1;

const STEP: i64 = 30;
const ALPHABET: base32::Alphabet = base32::Alphabet::Rfc4648 { padding: false };

pub fn new_secret() -> String {
    use rand::RngCore;
    let mut raw = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut raw);
    base32::encode(ALPHABET, &raw)
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("hmac key");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = ((h[off] & 0x7f) as u32) << 24
        | (h[off + 1] as u32) << 16
        | (h[off + 2] as u32) << 8
        | h[off + 3] as u32;
    bin % 1_000_000
}

pub fn code_at(secret_b32: &str, ts: i64) -> Option<String> {
    let secret = base32::decode(ALPHABET, &secret_b32.to_ascii_uppercase())?;
    Some(format!("{:06}", hotp(&secret, (ts / STEP) as u64)))
}

/// 允许前后各一个时间步(±30 s)的时钟漂移
pub fn verify(secret_b32: &str, code: &str, now: i64) -> bool {
    let code = code.trim();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    [-1i64, 0, 1].iter().any(|d| code_at(secret_b32, now + d * STEP).as_deref() == Some(code))
}

pub fn otpauth_url(secret_b32: &str, email: &str) -> String {
    format!("otpauth://totp/KiteTerm:{email}?secret={secret_b32}&issuer=KiteTerm&algorithm=SHA1&digits=6&period=30")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_vectors() {
        // RFC 6238 附录 B 的密钥 "12345678901234567890"(8 位码取后 6 位)
        let secret = base32::encode(ALPHABET, b"12345678901234567890");
        assert_eq!(code_at(&secret, 59).unwrap(), "287082");
        assert_eq!(code_at(&secret, 1111111109).unwrap(), "081804");
        assert_eq!(code_at(&secret, 1234567890).unwrap(), "005924");
    }

    #[test]
    fn verify_window_and_format() {
        let secret = new_secret();
        assert_eq!(secret.len(), 32);
        let now = 1_700_000_000;
        let c = code_at(&secret, now).unwrap();
        assert!(verify(&secret, &c, now));
        assert!(verify(&secret, &c, now + 29));
        assert!(verify(&secret, &c, now - 30));
        assert!(!verify(&secret, &c, now + 90));
        assert!(!verify(&secret, "12345", now));
        assert!(!verify(&secret, "abcdef", now));
        assert!(otpauth_url(&secret, "a@b.c").starts_with("otpauth://totp/KiteTerm:a@b.c?secret="));
    }
}

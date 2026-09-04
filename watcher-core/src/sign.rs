//! HMAC-SHA256 signing/verification for agent bundles (Phase 4D M-01).
//! The shared secret never appears in this module's own output — callers
//! are responsible for reading it only from `FOUNDRY_AGENT_KEY` or a
//! `--key-file` and never logging it (see `src/bin/foundry-agent.rs`).

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Signs `bytes` with `secret`, returning the MAC as a lowercase hex string.
pub fn sign_hex(secret: &[u8], bytes: &[u8]) -> String {
    // `new_from_slice` never fails for HMAC — any key length is accepted.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(bytes);
    hex_encode(&mac.finalize().into_bytes())
}

/// Verifies `sig_hex` is the correct HMAC-SHA256 of `bytes` under `secret`.
/// Uses a constant-time comparison — signature checks must never be a
/// short-circuiting `==` (timing side channel).
pub fn verify(secret: &[u8], bytes: &[u8], sig_hex: &str) -> bool {
    let expected = sign_hex(secret, bytes);
    constant_time_eq(expected.as_bytes(), sig_hex.as_bytes())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_then_verify_round_trips() {
        let secret = b"topsecret";
        let sig = sign_hex(secret, b"hello world");
        assert!(verify(secret, b"hello world", &sig));
    }

    #[test]
    fn tampered_body_fails_verify() {
        let secret = b"topsecret";
        let sig = sign_hex(secret, b"hello world");
        assert!(!verify(secret, b"hello WORLD", &sig));
    }

    #[test]
    fn wrong_key_fails_verify() {
        let sig = sign_hex(b"key1", b"hello world");
        assert!(!verify(b"key2", b"hello world", &sig));
    }
}

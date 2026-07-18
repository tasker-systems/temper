//! Authenticated encryption for the Slack grant vault.
//!
//! A single primitive: seal a secret (a refresh or access token) with XChaCha20-Poly1305 under
//! the instance's vault key, and open it again. The key is 32 bytes of operator-provided
//! randomness (`SLACK_VAULT_ENC_KEY`, base64), parsed ONCE at config load into a [`VaultKey`] so
//! nothing downstream ever handles an unvalidated key. Every seal draws a fresh 24-byte random
//! nonce — XChaCha's extended nonce is wide enough that random-per-write reuse is a non-issue —
//! and stores it beside the ciphertext. Opening a tampered ciphertext (or one sealed under a
//! different key) fails the Poly1305 tag: `decrypt` returns `Err`, never garbage.
//!
//! **Associated data binds each secret to its context.** Every seal takes an `aad` (associated
//! data) that is authenticated but not encrypted — the vault passes the principal plus a field
//! tag (`rt`/`at`). The tag is covered by the Poly1305 MAC, so a ciphertext sealed for one
//! principal-and-field will NOT open under any other: a DB-write attacker who transplants a valid
//! `(nonce, ciphertext)` from row A into row B (or from the `rt` column into the `at` column)
//! gets `OpenFailed`, not another user's token. The key alone is no longer enough to relocate a
//! sealed secret.
//!
//! This is deliberately the ONLY place that touches cipher internals. The vault service seals
//! and opens through here; it never sees a nonce size or a key byte.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore as _;

/// The extended-nonce width XChaCha20-Poly1305 uses. Stored per row.
pub const NONCE_LEN: usize = 24;

/// Raw AEAD key length: 32 bytes.
const KEY_LEN: usize = 32;

/// A parsed, validated vault key. Constructed once from `SLACK_VAULT_ENC_KEY`; from then on the
/// type IS the proof that the key is well-formed (parse, don't validate).
///
/// `Debug` is hand-written to redact the key material — this rides inside `SlackLinkConfig`,
/// which rides inside `ApiConfig`. Neither is currently `Debug`-logged, but if either ever is,
/// the key bytes must not reach the line; the redaction makes that safe by construction.
#[derive(Clone)]
pub struct VaultKey([u8; KEY_LEN]);

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(redacted)")
    }
}

/// What can go wrong sealing or opening a secret. Kept narrow and NEVER carries key or plaintext
/// bytes — the messages are safe to log.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("vault key is not valid base64")]
    KeyNotBase64,
    #[error("vault key must be exactly {KEY_LEN} bytes ({got} decoded)")]
    KeyWrongLength { got: usize },
    #[error("stored nonce is not {NONCE_LEN} bytes")]
    BadNonce,
    #[error("decryption failed (wrong key, or the ciphertext was tampered)")]
    OpenFailed,
}

impl VaultKey {
    /// Parse a base64-encoded 32-byte key. The two failure modes — not base64, wrong length —
    /// are distinct so an operator who mis-set the variable gets an actionable message.
    ///
    /// Generate one with `openssl rand -base64 32`.
    pub fn from_base64(encoded: &str) -> Result<Self, CryptoError> {
        let bytes = STANDARD
            .decode(encoded.trim())
            .map_err(|_| CryptoError::KeyNotBase64)?;
        let arr: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::KeyWrongLength { got: bytes.len() })?;
        Ok(Self(arr))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(Key::from_slice(&self.0))
    }

    /// Seal `plaintext`, binding `aad` (associated data) into the authentication tag. Returns
    /// `(nonce, ciphertext)` — store both; opening requires both AND the same `aad`. The nonce is
    /// fresh random every call.
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> ([u8; NONCE_LEN], Vec<u8>) {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);
        // In-memory AEAD over a small buffer does not fail in practice; the only documented error
        // is a length overflow that a token-sized plaintext cannot reach. Treat it as unreachable
        // rather than threading a can't-happen error through every caller.
        let ciphertext = self
            .cipher()
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .expect("XChaCha20-Poly1305 sealing of a token-sized buffer cannot fail");
        (nonce_bytes, ciphertext)
    }

    /// Open a `(nonce, ciphertext)` pair under the same `aad` it was sealed with. Fails closed on
    /// a bad nonce length, a wrong key, a tampered ciphertext, OR a mismatched `aad` (wrong
    /// principal/field) — the caller gets `Err`, never plaintext it should not trust.
    pub fn decrypt(
        &self,
        nonce: &[u8],
        ciphertext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError::BadNonce)?;
        self.cipher()
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::OpenFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn a_key() -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([7u8; KEY_LEN])).unwrap()
    }

    const AAD: &[u8] = b"slack:T:U\0rt";

    #[test]
    fn roundtrips_a_secret() {
        let key = a_key();
        let secret = b"a-refresh-token-value";
        let (nonce, ct) = key.encrypt(secret, AAD);
        assert_eq!(nonce.len(), NONCE_LEN);
        assert_ne!(ct.as_slice(), secret, "ciphertext must not equal plaintext");
        assert_eq!(key.decrypt(&nonce, &ct, AAD).unwrap(), secret);
    }

    #[test]
    fn a_fresh_nonce_each_call_makes_identical_plaintext_seal_differently() {
        let key = a_key();
        let (n1, c1) = key.encrypt(b"same", AAD);
        let (n2, c2) = key.encrypt(b"same", AAD);
        assert_ne!(n1, n2, "nonces must differ");
        assert_ne!(
            c1, c2,
            "sealing the same bytes twice must not produce the same ciphertext"
        );
    }

    #[test]
    fn a_tampered_ciphertext_fails_the_tag() {
        let key = a_key();
        let (nonce, mut ct) = key.encrypt(b"secret", AAD);
        ct[0] ^= 0xff;
        assert!(matches!(
            key.decrypt(&nonce, &ct, AAD),
            Err(CryptoError::OpenFailed)
        ));
    }

    #[test]
    fn a_different_key_cannot_open_it() {
        let sealed = a_key();
        let (nonce, ct) = sealed.encrypt(b"secret", AAD);
        let other = VaultKey::from_base64(&STANDARD.encode([9u8; KEY_LEN])).unwrap();
        assert!(matches!(
            other.decrypt(&nonce, &ct, AAD),
            Err(CryptoError::OpenFailed)
        ));
    }

    /// The AAD binding: a ciphertext sealed for one context must not open under another. This is
    /// the row/field-transplant defense — swapping a valid ciphertext into a row with a different
    /// principal (or the `at` field) presents a different AAD and fails the tag.
    #[test]
    fn a_mismatched_aad_cannot_open_it() {
        let key = a_key();
        let (nonce, ct) = key.encrypt(b"secret", b"slack:T:U\0rt");
        // Same key, same ciphertext, DIFFERENT aad (another principal, or the `at` field).
        assert!(matches!(
            key.decrypt(&nonce, &ct, b"slack:T:OTHER\0rt"),
            Err(CryptoError::OpenFailed)
        ));
        assert!(matches!(
            key.decrypt(&nonce, &ct, b"slack:T:U\0at"),
            Err(CryptoError::OpenFailed)
        ));
    }

    #[test]
    fn a_wrong_length_nonce_is_rejected_not_panicked() {
        let key = a_key();
        assert!(matches!(
            key.decrypt(&[0u8; 12], b"x", AAD),
            Err(CryptoError::BadNonce)
        ));
    }

    #[test]
    fn key_parsing_distinguishes_bad_base64_from_wrong_length() {
        assert!(matches!(
            VaultKey::from_base64("not base64 !!!"),
            Err(CryptoError::KeyNotBase64)
        ));
        assert!(matches!(
            VaultKey::from_base64(&STANDARD.encode([0u8; 16])),
            Err(CryptoError::KeyWrongLength { got: 16 })
        ));
        assert!(VaultKey::from_base64(&STANDARD.encode([0u8; KEY_LEN])).is_ok());
    }

    #[test]
    fn debug_does_not_leak_key_bytes() {
        let key = VaultKey::from_base64(&STANDARD.encode([0xABu8; KEY_LEN])).unwrap();
        assert_eq!(format!("{key:?}"), "VaultKey(redacted)");
    }
}

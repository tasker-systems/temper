//! PKCE (RFC 7636) S256 pair generation.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

/// Generate a PKCE `code_verifier` and its S256 `code_challenge`.
///
/// 32 random bytes -> a 43-character base64url verifier.
pub fn generate_pkce_pair() -> (String, String) {
    let random_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    (verifier, challenge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use sha2::{Digest, Sha256};

    #[test]
    fn challenge_is_s256_of_verifier() {
        let (verifier, challenge) = generate_pkce_pair();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn verifier_is_43_chars_and_pairs_differ() {
        let (v1, _) = generate_pkce_pair();
        let (v2, _) = generate_pkce_pair();
        assert_eq!(v1.len(), 43);
        assert_ne!(v1, v2);
    }
}

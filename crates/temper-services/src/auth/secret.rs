//! Temper-minted machine credentials (Phase B1, D1/D3). temper generates the client_id and
//! the secret; only the secret's SHA-256 hex is ever stored. `sha256_hex` is byte-identical
//! to the TS AS's `hashToken` (`createHash("sha256").digest("hex")`), so a hash written here
//! verifies against a secret presented at the TS `/oauth/token` endpoint.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore as _;
use sha2::{Digest, Sha256};

/// Prefix on temper-minted client ids, distinguishing them from Auth0 client ids at a glance.
const CLIENT_ID_PREFIX: &str = "tmpr_";

/// Lowercase SHA-256 hex of `input`. Matches the TS AS's `hashToken`.
pub fn sha256_hex(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    format!("{:x}", h.finalize())
}

/// A freshly minted secret: the plaintext (returned once, never stored) and its stored hash.
#[derive(Debug)]
pub struct MintedSecret {
    pub plaintext: String,
    pub hash: String,
}

/// Mint a 32-byte random secret (base64url-no-pad) and its SHA-256 hex.
pub fn mint_secret() -> MintedSecret {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let plaintext = URL_SAFE_NO_PAD.encode(bytes);
    let hash = sha256_hex(&plaintext);
    MintedSecret { plaintext, hash }
}

/// Mint a temper client id: `tmpr_` + base64url of 16 random bytes.
pub fn mint_client_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("{CLIENT_ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_answer() {
        // echo -n "abc" | sha256sum
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn mint_secret_hash_is_the_sha256_of_its_plaintext() {
        let s = mint_secret();
        assert_eq!(s.hash, sha256_hex(&s.plaintext));
        assert_eq!(s.hash.len(), 64, "sha256 hex is 64 chars");
        assert!(
            s.plaintext.len() >= 43,
            "32 bytes base64url-no-pad is 43 chars"
        );
    }

    #[test]
    fn mint_client_id_is_prefixed_and_unique() {
        let a = mint_client_id();
        let b = mint_client_id();
        assert!(a.starts_with("tmpr_"));
        assert_ne!(a, b, "two mints must differ");
    }
}

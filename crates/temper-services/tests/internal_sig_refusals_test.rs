//! Why an internal v2 signature is refused, before its nonce is ever spent.
//!
//! Outside `src/` because the fixtures are fixed nonces, which CodeQL's
//! `rust/hard-coded-cryptographic-value` reads as hard-coded wherever they sit; Rust has no inline
//! suppression, and `.github/codeql/codeql-config.yml` excludes `crates/*/tests/**` for this case.
//! None of these reach a database.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use temper_core::internal_sig::MAX_SKEW_SECS;
use temper_core::internal_sig::{sign_v2, DOOR_PRINCIPAL_RESOLVE, DOOR_SAML_RECONCILE};
use temper_services::services::internal_sig_service::{check, Refusal, SignedRequest};

const SECRET: &[u8] = b"secret";
const NONCE: &str = "0123456789abcdef0123456789abcdef";
const NOW: i64 = 1_750_000_000;
const BODY: &[u8] = b"{}";

/// The retired `"{timestamp}.{body}"` construction (`X-Temper-Signature`), built here because
/// nothing in `src/` signs it any more. It exists only so a test can show it is refused.
fn sign_v1(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn request<'a>(door: &'a str, signature: &'a str) -> SignedRequest<'a> {
    SignedRequest {
        method: "POST",
        door,
        timestamp: NOW,
        nonce: NONCE,
        signature,
        body: BODY,
    }
}

#[test]
fn a_signature_for_this_door_passes() {
    let sig = sign_v2(SECRET, "POST", DOOR_SAML_RECONCILE, NOW, NONCE, BODY);
    assert_eq!(
        check(SECRET, &request(DOOR_SAML_RECONCILE, &sig), NOW),
        Ok(())
    );
}

#[test]
fn a_signature_for_a_sibling_door_is_cross_door() {
    let sig = sign_v2(SECRET, "POST", DOOR_SAML_RECONCILE, NOW, NONCE, BODY);
    assert_eq!(
        check(SECRET, &request(DOOR_PRINCIPAL_RESOLVE, &sig), NOW),
        Err(Refusal::CrossDoor(DOOR_SAML_RECONCILE))
    );
}

#[test]
fn a_wrong_key_is_a_mismatch() {
    let sig = sign_v2(b"other", "POST", DOOR_SAML_RECONCILE, NOW, NONCE, BODY);
    assert_eq!(
        check(SECRET, &request(DOOR_SAML_RECONCILE, &sig), NOW),
        Err(Refusal::Mismatch)
    );
}

#[test]
fn the_original_construction_is_a_mismatch() {
    let sig = sign_v1(SECRET, NOW, BODY);
    assert_eq!(
        check(SECRET, &request(DOOR_SAML_RECONCILE, &sig), NOW),
        Err(Refusal::Mismatch)
    );
}

#[test]
fn stale_and_malformed_are_refused_before_the_mac() {
    let sig = sign_v2(SECRET, "POST", DOOR_SAML_RECONCILE, NOW, NONCE, BODY);
    let req = request(DOOR_SAML_RECONCILE, &sig);
    assert_eq!(
        check(SECRET, &req, NOW + MAX_SKEW_SECS + 1),
        Err(Refusal::Stale)
    );
    let bad_nonce = SignedRequest {
        nonce: "NOT-HEX",
        ..request(DOOR_SAML_RECONCILE, &sig)
    };
    assert_eq!(check(SECRET, &bad_nonce, NOW), Err(Refusal::MalformedNonce));
}
